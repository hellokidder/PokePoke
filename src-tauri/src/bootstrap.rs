//! 应用启动期后台任务编排 — 从 `lib.rs` setup 抽出。

use crate::commands::{integration, PendingUpdate, PendingUpdateState};
use crate::popup;
use crate::settings::SettingsStore;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// 首次运行自动接入本机已检测到的 cc / codex / cursor / opencode / pi / omp hooks。
pub fn run_first_run_hook_install(is_first_run: bool, settings_store: &Arc<Mutex<SettingsStore>>) {
    if !is_first_run {
        return;
    }

    let candidates: [(&str, &str); 9] = [
        ("cc", "--install"),
        ("codex", "--install-codex"),
        ("cursor", "--install-cursor"),
        ("opencode", "--install-opencode"),
        ("pi", "--install-pi"),
        ("omp", "--install-omp"),
        ("kimi", "--install-kimi"),
        ("grok", "--install-grok"),
        ("dsh", "--install-dsh"),
    ];
    let present: Vec<&str> = candidates
        .iter()
        .filter_map(|(label, _)| {
            if crate::agent_presence::is_agent_present(label) {
                Some(*label)
            } else {
                crate::log_info!(
                    "lifecycle",
                    "first-run: {} not detected on host, skip auto-install",
                    label
                );
                None
            }
        })
        .collect();
    if present.is_empty() {
        crate::log_info!(
            "lifecycle",
            "first run detected, no managed agents found on host — skip auto-install"
        );
        settings_store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .save();
        return;
    }

    crate::log_info!(
        "lifecycle",
        "first run detected, auto-installing hooks for: {}",
        present.join(", ")
    );
    let hook_bin = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".local/bin/poke-hook"))
        .filter(|p| p.exists());
    if let Some(bin) = hook_bin {
        // Bug 5 保险:防止 dev 首启用到残留的老 ~/.local/bin/poke-hook 写 hook 配置。
        // Release 走 tauri-bundler,`ensure_hook_deployed` 已把 bundle 内的 pokehook
        // 强制覆盖到该路径,版本一致;dev 未跑 `pnpm install:hook` 时会漂移。
        if !deployed_hook_version_matches(&bin) {
            crate::log_warn!(
                "lifecycle",
                "first-run: deployed hook at {} version mismatches app (expected {}), \
                 skip auto-install to avoid stale-hook config drift; \
                 run `pnpm install:hook` (dev) or reinstall the app to refresh",
                bin.display(),
                env!("CARGO_PKG_VERSION")
            );
            settings_store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .save();
            return;
        }
        for (label, arg) in candidates
            .iter()
            .filter(|(label, _)| present.contains(label))
        {
            match std::process::Command::new(&bin).arg(arg).output() {
                Ok(o) if o.status.success() => {
                    crate::log_info!("lifecycle", "first-run: {} hook installed", label)
                }
                Ok(o) => crate::log_warn!(
                    "lifecycle",
                    "first-run: {} install failed: {}",
                    label,
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(e) => crate::log_warn!(
                    "lifecycle",
                    "first-run: {} install spawn error: {}",
                    label,
                    e
                ),
            }
        }
    } else {
        crate::log_warn!(
            "lifecycle",
            "first-run: hook binary at ~/.local/bin/poke-hook missing, skip auto-install"
        );
    }
    settings_store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .save();
}

/// Deployed hook 是否与当前 app 同版本。
///
/// 判据:跑 `<bin> --version`,输出应为 `poke-hook <ver>`,比对 `<ver>` 与
/// `CARGO_PKG_VERSION`。任何异常(spawn 失败 / 老版本二进制不认识 --version /
/// 版本号格式不对)都视为**不匹配**,让 caller 走 skip 分支——宁可跳过自动安装,
/// 也不能让老二进制写老配置。老 poke-hook(<0.x)不认 `--version` 时会走 hook_mode
/// 报"not invoked in hook mode",exit code 非 0,自然落到不匹配分支。
fn deployed_hook_version_matches(bin: &std::path::Path) -> bool {
    let expected = env!("CARGO_PKG_VERSION");
    let output = match std::process::Command::new(bin)
        .arg("--version")
        // 老版本 pokehook 不认 --version 会走 hook_mode 读 stdin;显式 null 保证立即 EOF
        // 不 hang(父 GUI 进程 stdin 状态不可控)。
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            crate::log_warn!(
                "lifecycle",
                "first-run: hook --version spawn error: {} (treat as mismatch)",
                e
            );
            return false;
        }
    };
    if !output.status.success() {
        crate::log_warn!(
            "lifecycle",
            "first-run: hook --version exit={:?} stderr={} (treat as mismatch)",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // 期望 "poke-hook <ver>";只认前缀,允许后续携带 build metadata 之类。
    let deployed = stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("poke-hook "))
        .map(|s| s.trim());
    match deployed {
        Some(v) if v == expected => true,
        Some(v) => {
            crate::log_warn!(
                "lifecycle",
                "first-run: deployed hook version {} != app {}",
                v,
                expected
            );
            false
        }
        None => {
            crate::log_warn!(
                "lifecycle",
                "first-run: hook --version output unparsable: {:?} (treat as mismatch)",
                stdout.trim()
            );
            false
        }
    }
}

/// 启动期"部署后"流水线 — first_run → auto_connect → drift,一线程内串行保序。
///
/// 挪到独立线程的理由(2026-07-20):
/// - `run_first_run_hook_install` 内部会走 login-shell CLI 探测(nvm/volta 兜底,
///   见 agent_presence::login_shell_which),用户 rc 文件复杂时单端可达 ~2s,
///   四端最坏 8s 全部落在 Tauri `.setup()` 里,首启白屏。
/// - `ensure_auto_connect_hooks` 每个未接入端会 spawn `poke-hook --check-*`,
///   常态也是百毫秒级,GUI 冷启动没必要吃这段延迟。
///
/// 保序:注释在 lib.rs 里详述过,auto-connect 必须先于 drift_detection 跑完,
/// 否则 drift 会先看到"过期"状态、弹一个马上被 auto-connect 自愈的假告警。
/// 三步在同一线程内 sequential 执行,天然保持这个顺序。
pub fn spawn_post_deploy_pipeline(
    app: AppHandle,
    is_first_run: bool,
    settings_store: Arc<Mutex<SettingsStore>>,
) {
    std::thread::spawn(move || {
        run_first_run_hook_install(is_first_run, &settings_store);
        integration::ensure_auto_connect_hooks(&settings_store);
        integration::detect_hook_drift_and_emit(app);
    });
}

/// 后台 updater 巡检任务。
pub fn spawn_updater_poll(
    app: AppHandle,
    settings_store: Arc<Mutex<SettingsStore>>,
    popup_list: popup::PopupList,
    pending: PendingUpdate,
) {
    tauri::async_runtime::spawn(async move {
        use std::time::Instant;
        const TICK: std::time::Duration = std::time::Duration::from_secs(60);
        const OK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
        const ERR_RETRY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let mut next_check_at = Instant::now();
        let mut last_auto_check = true;
        loop {
            let (auto_check, skip_ver): (bool, Option<String>) = {
                let g = settings_store.lock().unwrap_or_else(|p| p.into_inner());
                (
                    g.settings.update_auto_check,
                    g.settings.update_skip_version.clone(),
                )
            };
            if !last_auto_check && auto_check {
                crate::log_info!("updater", "auto_check re-enabled, forcing immediate check");
                next_check_at = Instant::now();
            }
            last_auto_check = auto_check;
            if auto_check && Instant::now() >= next_check_at {
                match app.updater() {
                    Ok(u) => match u.check().await {
                        Ok(Some(update)) => {
                            let v = update.version.clone();
                            if Some(v.as_str()) == skip_ver.as_deref() {
                                *pending.lock().unwrap_or_else(|p| p.into_inner()) =
                                    PendingUpdateState::None;
                                crate::log_info!(
                                    "updater",
                                    "version {} matches user-skipped, silent",
                                    v
                                );
                            } else {
                                *pending.lock().unwrap_or_else(|p| p.into_inner()) =
                                    PendingUpdateState::Available(update.clone());
                                crate::log_info!(
                                    "updater",
                                    "new version available: {} (current {})",
                                    v,
                                    update.current_version
                                );
                                popup::show_update_popup(&app, &v, &popup_list);
                            }
                            next_check_at = Instant::now() + OK_INTERVAL;
                        }
                        Ok(None) => {
                            crate::log_info!("updater", "up_to_date");
                            *pending.lock().unwrap_or_else(|p| p.into_inner()) =
                                PendingUpdateState::None;
                            next_check_at = Instant::now() + OK_INTERVAL;
                        }
                        Err(e) => {
                            crate::log_warn!("updater", "check failed (retry in 15min): {}", e);
                            next_check_at = Instant::now() + ERR_RETRY;
                        }
                    },
                    Err(e) => {
                        crate::log_warn!(
                            "updater",
                            "updater builder failed (retry in 15min): {}",
                            e
                        );
                        next_check_at = Instant::now() + ERR_RETRY;
                    }
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}
