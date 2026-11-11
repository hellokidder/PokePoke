//! pi agent extension 接入:install / uninstall / check。
//!
//! pi(coding agent CLI, npm `@earendil-works/pi-coding-agent`)走 file-drop
//! 模式:把一个 TS 扩展投到 `~/.pi/agent/extensions/pokepoke/index.ts`。
//! pi 启动时经 jiti 自动发现该目录(不需改 settings.json),扩展订阅 pi 事件
//! 并归一为 PokePoke 内部协议经 stdin 传给 `poke-hook --source=pi`。
//! server 端复用现有 SessionStart/UserPromptSubmit/Stop/SessionEnd 链路,零 schema 变更。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::deploy::{ensure_binary_installed, print_json};
use super::paths::{hook_bin_path, pi_extension_dir, pi_extension_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use std::fs;

const PI_EXTENSION_TS: &str = include_str!("pi-extension.ts");

/// 占位符替换后的扩展全文。
pub(crate) fn pi_extension_contents() -> String {
    PI_EXTENSION_TS.replace("__POKE_HOOK_BIN__", &hook_bin_path().display().to_string())
}

pub(crate) fn cmd_install_pi() {
    if !ensure_binary_installed() {
        return;
    }
    let ext_path = pi_extension_path();
    let ext_dir = pi_extension_dir();
    if let Err(e) = fs::create_dir_all(&ext_dir) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to create {}: {}", ext_dir.display(), e),
        }));
        return;
    }

    record_pre_write_snapshot("pi", &ext_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&ext_path);

    // pokepoke/ 目录是 PokePoke 独占投放物,index.ts 直接覆盖即幂等。
    if let Err(e) = fs::write(&ext_path, pi_extension_contents()) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to write {}: {}", ext_path.display(), e),
        }));
        return;
    }

    if let Err(e) = verify::verify_pi_writes(&ext_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("pi install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!("Installed pi extension at {}", ext_path.display()),
    }));
}

pub(crate) fn cmd_uninstall_pi() {
    let ext_dir = pi_extension_dir();
    if !ext_dir.exists() {
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "No pi extension found, nothing to do"
        }));
        return;
    }

    // 卸载前对 index.ts 做一份 snapshot,便于用户手动复原。
    let ext_path = pi_extension_path();
    if ext_path.exists() {
        record_pre_write_snapshot("pi", &ext_path);
    }

    // pokepoke/ 目录是 PokePoke 独占的,直接删整个目录;不碰用户其他扩展。
    match fs::remove_dir_all(&ext_dir) {
        Ok(()) => print_json(&serde_json::json!({
            "status": "ok",
            "message": "Removed pi extension",
        })),
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to remove {}: {}", ext_dir.display(), e),
        })),
    }
}

/// pi 扩展是否已接入:文件存在且内容含 PokePoke 标记 + `--source=pi`。
pub(crate) fn pi_extension_configured(ext_path: &std::path::Path) -> bool {
    fs::read_to_string(ext_path)
        .map(|c| c.contains("PokePoke pi integration") && c.contains("--source=pi"))
        .unwrap_or(false)
}

/// 已接入但扩展内烘焙的 hook 路径与当前 `~/.local/bin/poke-hook` 不一致 ->
/// 判为过期(drift),交由 detect_hook_drift 走"告警 + 一键修复"。
pub(crate) fn pi_extension_outdated(ext_path: &std::path::Path) -> bool {
    let Ok(content) = fs::read_to_string(ext_path) else {
        return false;
    };
    if !content.contains("PokePoke pi integration") {
        return false;
    }
    let expected = hook_bin_path().display().to_string();
    !content.contains(&expected)
}

pub(crate) fn cmd_check_pi() {
    let installed = hook_bin_path().exists();
    let ext_path = pi_extension_path();
    let hooks_configured = ext_path.exists() && pi_extension_configured(&ext_path);
    let hooks_outdated = hooks_configured && pi_extension_outdated(&ext_path);

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured,
    }));
}
