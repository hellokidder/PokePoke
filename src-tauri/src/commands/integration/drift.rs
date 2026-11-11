use crate::settings::SettingsStore;
use crate::tray;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

use super::{
    check_arg_for_source, clear_integration_disconnected, hook_bin_path, install_arg_for_source,
    mark_source_auto_connected, run_poke_hook_json, MANAGED_INTEGRATION_ORDER,
};

/// 启动自检:扫三端 hook 配置,发现 command 字符串缺 `--source=<x>` 后缀
/// 时**不再静默重装**——把 source 写入 `HookDriftState` + emit
/// `hook-drift-detected` 事件,等待用户在 panel / tray 点"一键修复"
/// 触发 `repair_hook_drift(source)` command 才真正改外部配置。
///
/// 由 `docs/archive/scope-audit-20260422.md §3.5 / §8.2` + `docs/briefs/20260422-drift-alert-not-silent.md`
/// 决议:静默自动修复违反"用户可感、可信"产品定位,改为告警 + 一键修复。
///
/// 同步 fn(所有 IO 都是 Command::output() 阻塞调用);调用方应当在独立
/// std::thread::spawn 中跑,不要阻塞 setup / tokio runtime。
pub(crate) fn detect_hook_drift_and_emit(app: AppHandle) {
    let started_at = std::time::Instant::now();
    crate::log_debug!("lifecycle", "startup drift check started");

    let outdated_sources: Vec<String> = MANAGED_INTEGRATION_ORDER
        .iter()
        .filter_map(|source| {
            let check_arg = check_arg_for_source(source)?;
            let check = run_poke_hook_json(&[check_arg]);
            let outdated = check
                .get("hooks_outdated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if outdated {
                crate::log_warn!(
                    "lifecycle",
                    "hook config drift detected for {}, awaiting user action",
                    source
                );
                Some((*source).to_string())
            } else {
                None
            }
        })
        .collect();

    if let Some(state) = app.try_state::<Arc<Mutex<HookDriftState>>>() {
        let mut s = state.lock().unwrap_or_else(|p| p.into_inner());
        s.set_outdated(&outdated_sources);
    }
    let _ = app.emit("hook-drift-detected", outdated_sources.clone());

    tray::refresh_tray_menu(&app);

    crate::log_debug!(
        "lifecycle",
        "startup drift check finished in {}ms (outdated={:?})",
        started_at.elapsed().as_millis(),
        outdated_sources,
    );
}

/// 用户从 panel 或托盘点"一键修复"时调用。按 source 走对应 install 路径,
/// 完成后从 `HookDriftState` 移除该 source + emit `hook-drift-repaired`。
#[tauri::command]
pub fn repair_hook_drift(app: AppHandle, source: String) -> serde_json::Value {
    let Some(install_arg) = install_arg_for_source(&source) else {
        return serde_json::json!({
            "status": "error",
            "message": format!("unknown source: {}", source),
        });
    };

    crate::log_info!("lifecycle", "repair_hook_drift triggered for {}", source);
    let result = run_poke_hook_json(&[install_arg]);
    let ok = result
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s == "ok")
        .unwrap_or(false);

    if ok {
        if let Some(state) = app.try_state::<Arc<Mutex<HookDriftState>>>() {
            state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear_source(&source);
        }
        if let Some(settings_store) = app.try_state::<Arc<Mutex<SettingsStore>>>() {
            clear_integration_disconnected(&settings_store, &source);
            mark_source_auto_connected(&settings_store, &source);
        }
        crate::log_info!("lifecycle", "hook config drift repaired for {}", source);
        let _ = app.emit("hook-drift-repaired", &source);
        tray::refresh_tray_menu(&app);
    } else {
        crate::log_error!(
            "lifecycle",
            "repair_hook_drift failed for {}: {}",
            source,
            result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        );
    }
    result
}

/// 前端 panel mount 时拉一次当前 drift 状态(避免错过启动自检 emit 的事件)。
#[tauri::command]
pub fn get_hook_drift_state(state: State<'_, Arc<Mutex<HookDriftState>>>) -> Vec<String> {
    state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .outdated_sources()
}

/// 内存维护"哪几端当前过期、待用户一键修复"。
#[derive(Default)]
pub struct HookDriftState {
    sources: std::collections::HashSet<String>,
}

impl HookDriftState {
    pub fn set_outdated(&mut self, sources: &[String]) {
        self.sources = sources.iter().cloned().collect();
    }
    pub fn clear_source(&mut self, source: &str) {
        self.sources.remove(source);
    }
    pub fn outdated_sources(&self) -> Vec<String> {
        let mut v: Vec<String> = self.sources.iter().cloned().collect();
        v.sort();
        v
    }
}

/// 启动时隐性补齐未接入端。对每个 source 做三级判定:
///
///   connected = true              → 跳过(已接好),记录到 auto_connected_sources
///   hooks_outdated = true          → 覆盖安装(命令格式/路径变了),不检查 auto_connected
///   其余(未接+不过期)               → 未在 auto_connected 则补齐;已在则跳过(用户断开过)
///
/// 这套逻辑覆盖:
///   - 版本新增端(如 v1.0.2 新增 opencode)          → 未 auto-connect → 补齐
///   - 版本升级改 event 列表 / 二进制路径 / plugin  → outdated → 覆盖(无视 auto_connected)
///   - 用户手动断开                                   → 未接+不过期+已 auto-connect → 跳过,不装回
pub(crate) fn ensure_auto_connect_hooks(
    settings_store: &std::sync::Arc<std::sync::Mutex<crate::settings::SettingsStore>>,
) {
    #[cfg(not(test))]
    const SOURCES: &[&str] = MANAGED_INTEGRATION_ORDER;
    #[cfg(test)]
    const SOURCES: &[&str] = &[];

    fn check(source: &str) -> serde_json::Value {
        let Some(arg) = check_arg_for_source(source) else {
            return serde_json::json!({"connected": true});
        };
        run_poke_hook_json(&[arg])
    }

    let bin_path = hook_bin_path();
    if !bin_path.exists() {
        return;
    }

    let already: std::collections::HashSet<String> = {
        let g = settings_store.lock().unwrap_or_else(|p| p.into_inner());
        g.settings.auto_connected_sources.iter().cloned().collect()
    };

    let mut new_sources: Vec<String> = Vec::new();

    for source in SOURCES {
        let chk = check(source);
        let connected = chk
            .get("connected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let outdated = chk
            .get("hooks_outdated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let action = auto_connect_action(
            crate::agent_presence::is_agent_present(source),
            connected,
            outdated,
            already.contains(*source),
        );

        match action {
            AutoConnectAction::SkipAgentNotPresent => {
                crate::log_debug!(
                    "lifecycle",
                    "auto-connect: {} not detected on host, skip",
                    source,
                );
                continue;
            }
            AutoConnectAction::RecordConnected => {
                if !already.contains(*source) {
                    new_sources.push(source.to_string());
                }
                continue;
            }
            AutoConnectAction::SkipUserDisconnected => {
                crate::log_debug!(
                    "lifecycle",
                    "auto-connect: {} was auto-connected but not configured (user disconnected), skip",
                    source,
                );
                continue;
            }
            AutoConnectAction::InstallOutdated => {
                crate::log_info!(
                    "lifecycle",
                    "auto-connect: {} hooks outdated, reinstalling",
                    source,
                );
            }
            AutoConnectAction::InstallNew => {
                crate::log_info!(
                    "lifecycle",
                    "auto-connect: {} not configured, installing silently",
                    source,
                );
            }
        }

        let Some(install_arg) = install_arg_for_source(source) else {
            continue;
        };
        let result = std::process::Command::new(&bin_path)
            .arg(install_arg)
            .output();
        match result {
            Ok(o) if o.status.success() => {
                crate::log_info!(
                    "lifecycle",
                    "auto-connect: {} installed successfully",
                    source,
                );
                new_sources.push(source.to_string());
            }
            Ok(o) => crate::log_warn!(
                "lifecycle",
                "auto-connect: {} install failed: {}",
                source,
                String::from_utf8_lossy(&o.stderr).trim(),
            ),
            Err(e) => crate::log_warn!(
                "lifecycle",
                "auto-connect: {} install spawn error: {}",
                source,
                e,
            ),
        }
    }

    if new_sources.is_empty() {
        return;
    }

    // 把新补齐的 source 写入 auto_connected_sources。
    if let Ok(mut g) = settings_store.lock() {
        for s in &new_sources {
            if !g.settings.auto_connected_sources.contains(s) {
                g.settings.auto_connected_sources.push(s.clone());
            }
        }
        g.save();
        crate::log_info!(
            "lifecycle",
            "auto-connect: saved auto_connected_sources={:?}",
            g.settings.auto_connected_sources,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoConnectAction {
    SkipAgentNotPresent,
    RecordConnected,
    SkipUserDisconnected,
    InstallOutdated,
    InstallNew,
}

/// `ensure_auto_connect_hooks` 对单端的决策(纯函数,可单测)。
pub(crate) fn auto_connect_action(
    agent_present: bool,
    connected: bool,
    hooks_outdated: bool,
    already_auto_connected: bool,
) -> AutoConnectAction {
    if !agent_present {
        return AutoConnectAction::SkipAgentNotPresent;
    }
    if connected {
        return AutoConnectAction::RecordConnected;
    }
    if hooks_outdated {
        return AutoConnectAction::InstallOutdated;
    }
    if already_auto_connected {
        return AutoConnectAction::SkipUserDisconnected;
    }
    AutoConnectAction::InstallNew
}
