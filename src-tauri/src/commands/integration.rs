//! 各 agent 接入端(CC / Codex / Cursor / opencode / pi / kimi / grok)的统一入口。
//!
//! 2026-08-12 按职责拆分(AGENTS.md §4a,行为零变化,逻辑逐字搬移):
//! - [`drift`]   : 启动自检 drift 告警 / 一键修复 / auto-connect
//! - [`status`]  : CC + managed 各端状态查询与诊断导出
//! - [`managed`] : connect/disconnect / install/uninstall 等变更操作
//! - 本文件保留跨组共享 helper(binary 部署 / 探活 / source 映射 / settings 标记)
//!   与 `pub use` 重导出,保持 `crate::commands::integration::*` 外部引用面不变。

use crate::hook_deploy::deploy_hook_from_to;
use crate::settings::SettingsStore;
use crate::tray;
use std::process::Command;
use tauri::AppHandle;

mod drift;
mod managed;
mod status;

// ---- 重导出:保持 crate::commands::integration::* 引用面不变 ----
//
// 注:`#[tauri::command]` 会为每个 command 生成 `__cmd__<name>` 宏包装
// (invoke_handler! 按 `integration::__cmd__<name>` 路径解析),重导出函数时
// 必须同步重导出该宏。`#[allow(unused_imports)]`:本文件是纯转发层,
// 引用方在 lib.rs / tray.rs / diagnostics.rs / bootstrap.rs / tests.rs,
// 非 test 构建下部分符号仅被测试引用,不加 allow 会误报 unused_imports。

#[allow(unused_imports)]
pub use drift::{
    __cmd__get_hook_drift_state, __cmd__repair_hook_drift, get_hook_drift_state, repair_hook_drift,
    HookDriftState,
};
#[allow(unused_imports)]
pub(crate) use drift::{
    auto_connect_action, detect_hook_drift_and_emit, ensure_auto_connect_hooks, AutoConnectAction,
};
#[allow(unused_imports)]
pub use managed::{
    __cmd__connect_managed_integration, __cmd__disconnect_managed_integration,
    __cmd__install_cursor_integration, __cmd__install_opencode_integration,
    __cmd__install_pi_integration, __cmd__repair_cc_integration,
    __cmd__uninstall_cursor_integration, __cmd__uninstall_opencode_integration,
    __cmd__uninstall_pi_integration, clear_sessions_for_integration, connect_managed_integration,
    disconnect_managed_integration, install_cursor_integration, install_opencode_integration,
    install_pi_integration, repair_cc_integration, uninstall_cursor_integration,
    uninstall_opencode_integration, uninstall_pi_integration,
};
#[allow(unused_imports)]
pub(crate) use managed::{disconnect_integration_fully, session_ids_for_integration};
#[allow(unused_imports)]
pub use status::{
    __cmd__check_cc_integration, __cmd__check_codex_integration, __cmd__check_cursor_integration,
    __cmd__check_managed_integration, __cmd__check_opencode_integration,
    __cmd__check_pi_integration, __cmd__list_managed_integrations, cc_integration_status,
    check_cc_integration, check_codex_integration, check_cursor_integration,
    check_managed_integration, check_opencode_integration, check_pi_integration,
    emit_cc_integration_updated, list_managed_integrations, CcIntegrationStatus,
    ManagedIntegrationStatus,
};
#[allow(unused_imports)]
pub(crate) use status::{
    codex_check_json, cursor_check_json, grok_check_json, kimi_check_json, managed_status_from_cc,
    managed_status_from_hook_check, opencode_check_json, pi_check_json,
};

// ---- 共享:binary 部署 / 探活 ----

pub(crate) fn hook_bin_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".local/bin/poke-hook")
}

/// 诊断导出复用:hook 二进制是否存在 / 可执行。
pub(crate) fn hook_binary_state() -> (bool, bool) {
    let path = hook_bin_path();
    let exists = path.exists();
    (exists, exists && is_executable(&path))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.exists()
}

/// 启动期把 bundle 内的 poke-hook 部署到 ~/.local/bin/poke-hook(自愈)。
///
/// 协议升级时(server-hook transport 变化等)用户不必手动 `pnpm install:hook`,
/// 重启 PokePoke 即可:每次启动都强制覆盖,~10ms,幂等。详见
/// `docs/briefs/20260423-bundle-hook-binary.md`。
///
/// 源路径来自 Tauri 对 `[[bin]] pokehook` 的自动打包——产物落在
/// `Contents/MacOS/pokehook`,与主进程 binary 同目录,所以用
/// `current_exe().parent().join("pokehook")` 定位。文件名 `pokehook`(无连字符)
/// 是为了规避 tauri-bundler 把 `src/bin/X.rs` 的文件 basename 当作 sidecar
/// binary 名去 `target/release/<basename>` 找文件——`hook.rs` 会让它去找
/// `target/release/hook` 而非 `target/release/poke-hook`。
///
/// dev 模式下兄弟目录不存在 pokehook → log_warn 后跳过,继续依赖
/// `pnpm install:hook` 部署 debug 版。
pub(crate) fn ensure_hook_deployed(_app: &AppHandle) {
    let bundled = match std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("pokehook")))
    {
        Some(p) => p,
        None => {
            crate::log_warn!("lifecycle", "current_exe parent unavailable");
            return;
        }
    };

    if !bundled.exists() {
        crate::log_warn!(
            "lifecycle",
            "bundled hook not found at {} (dev mode? run `pnpm install:hook`)",
            bundled.display()
        );
        return;
    }

    let target = hook_bin_path();
    match deploy_hook_from_to(&bundled, &target) {
        Ok(()) => crate::log_info!("lifecycle", "hook deployed to {}", target.display()),
        Err(e) => crate::log_error!(
            "lifecycle",
            "hook deploy failed src={} dst={}: {}",
            bundled.display(),
            target.display(),
            e
        ),
    }
}

/// 调用 poke-hook binary 并解析 stdout 为 JSON。
/// binary 缺失 / 命令失败 / JSON 解析失败统一返回一份"未接入"占位对象，
/// 避免前端 / 托盘在异常路径下拿到空值。
fn run_poke_hook_json(args: &[&str]) -> serde_json::Value {
    let hook_path = hook_bin_path();
    if !hook_path.exists() {
        return serde_json::json!({
            "installed": false,
            "hooks_configured": false,
            "connected": false,
        });
    }
    Command::new(&hook_path)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            serde_json::from_str(out.trim()).ok()
        })
        .unwrap_or(serde_json::json!({
            "installed": false,
            "hooks_configured": false,
            "connected": false,
        }))
}

fn after_integration_change(app: &AppHandle, source: &str) {
    tray::refresh_tray_menu(app);
    if source == "cc" {
        emit_cc_integration_updated(app);
    }
}

// ---- 共享:managed source 映射 ----

/// managed source id → `/notify` payload 里的 `source` 字段值。
pub fn notify_source_for_integration(integration: &str) -> Option<&'static str> {
    match integration {
        "cc" => Some("claude-code"),
        "codex" => Some("codex"),
        "cursor" => Some("cursor"),
        "opencode" => Some("opencode"),
        "pi" => Some("pi"),
        "omp" => Some("omp"),
        "kimi" => Some("kimi"),
        "grok" => Some("grok"),
        "dsh" => Some("dsh"),
        _ => None,
    }
}
/// `/notify` payload `source` → managed integration id。
pub fn integration_for_notify_source(notify_source: &str) -> Option<&'static str> {
    match notify_source {
        "claude-code" => Some("cc"),
        "codex" => Some("codex"),
        "cursor" => Some("cursor"),
        "opencode" => Some("opencode"),
        "pi" => Some("pi"),
        "omp" => Some("omp"),
        "kimi" => Some("kimi"),
        "grok" => Some("grok"),
        "dsh" => Some("dsh"),
        _ => None,
    }
}

const MANAGED_INTEGRATION_ORDER: &[&str] = &[
    "cc", "codex", "cursor", "opencode", "pi", "omp", "kimi", "grok", "dsh",
];

fn check_arg_for_source(source: &str) -> Option<&'static str> {
    match source {
        "cc" => Some("--check"),
        "codex" => Some("--check-codex"),
        "cursor" => Some("--check-cursor"),
        "opencode" => Some("--check-opencode"),
        "pi" => Some("--check-pi"),
        "omp" => Some("--check-omp"),
        "kimi" => Some("--check-kimi"),
        "grok" => Some("--check-grok"),
        "dsh" => Some("--check-dsh"),
        _ => None,
    }
}
fn install_arg_for_source(source: &str) -> Option<&'static str> {
    match source {
        "cc" => Some("--install"),
        "codex" => Some("--install-codex"),
        "cursor" => Some("--install-cursor"),
        "opencode" => Some("--install-opencode"),
        "pi" => Some("--install-pi"),
        "omp" => Some("--install-omp"),
        "kimi" => Some("--install-kimi"),
        "grok" => Some("--install-grok"),
        "dsh" => Some("--install-dsh"),
        _ => None,
    }
}
fn uninstall_arg_for_source(source: &str) -> Option<&'static str> {
    match source {
        "cc" => Some("--uninstall"),
        "codex" => Some("--uninstall-codex"),
        "cursor" => Some("--uninstall-cursor"),
        "opencode" => Some("--uninstall-opencode"),
        "pi" => Some("--uninstall-pi"),
        "omp" => Some("--uninstall-omp"),
        "kimi" => Some("--uninstall-kimi"),
        "grok" => Some("--uninstall-grok"),
        "dsh" => Some("--uninstall-dsh"),
        _ => None,
    }
}

// ---- 共享:settings 侧状态(disconnected / auto_connected)----

pub(crate) fn mark_source_auto_connected(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    source: &str,
) {
    if let Ok(mut g) = settings_store.lock() {
        if !g
            .settings
            .auto_connected_sources
            .iter()
            .any(|s| s == source)
        {
            g.settings.auto_connected_sources.push(source.to_string());
            g.save();
        }
    }
}

pub(crate) fn clear_integration_disconnected(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    source: &str,
) {
    if let Ok(mut g) = settings_store.lock() {
        let before = g.settings.disconnected_sources.len();
        g.settings.disconnected_sources.retain(|s| s != source);
        if g.settings.disconnected_sources.len() != before {
            g.save();
        }
    }
}

pub fn is_integration_disconnected(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    integration: &str,
) -> bool {
    settings_store
        .lock()
        .map(|g| {
            g.settings
                .disconnected_sources
                .iter()
                .any(|s| s == integration)
        })
        .unwrap_or(false)
}

pub fn is_notify_source_disconnected(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    notify_source: Option<&str>,
) -> bool {
    let Some(notify_source) = notify_source else {
        return false;
    };
    let Some(integration) = integration_for_notify_source(notify_source) else {
        return false;
    };
    is_integration_disconnected(settings_store, integration)
}

#[cfg(test)]
#[path = "integration/tests.rs"]
mod tests;
