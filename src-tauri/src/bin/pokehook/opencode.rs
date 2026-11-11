//! opencode plugin 接入:install / uninstall / check。
//!
//! opencode 的接入单元不是"改配置文件",而是投放一个 JS plugin。opencode 启动时
//! 自动 glob 加载 `${XDG_CONFIG_HOME:-~/.config}/opencode/{plugin,plugins}/*.{ts,js}`。
//! plugin 订阅 opencode event bus,把每个事件**归一成 poke-hook 已认识的扁平字段**
//! (`hook_event_name` / `session_id` / `cwd` / `message` / `reason`),再 shell-out
//! 调 `poke-hook --source=opencode` 从 stdin 喂进现有 /notify 主链路。
//!
//! 关键适配:opencode `session.idle` 只带 `sessionID`,不带 cwd。plugin 从 context
//! 的 `directory`(当前 opencode 实例项目根)补 cwd 到每个事件,因此 Rust 侧
//! `resolve_workspace_path` 的默认 `cwd` 分支即可正确处理,无需 opencode 专用分支。
//!
//! `__POKE_HOOK_BIN__` 占位符在 install 时替换成 `~/.local/bin/poke-hook` 的绝对路径
//! (展开 HOME),plugin 不依赖 PATH。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::deploy::{ensure_binary_installed, print_json};
use super::paths::{hook_bin_path, opencode_plugin_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use std::fs;

/// PokePoke 投放的 opencode plugin 模板。install 时把 `__POKE_HOOK_BIN__` 替换成
/// hook 二进制绝对路径后写盘。内嵌为常量(不作为独立源文件长期维护),与 Cursor
/// 内嵌 hook entry 同风格。
///
/// **runtime 兼容**:opencode 1.14.37+ 的桌面/CLI sidecar 是 **Node runtime**(不是
/// Bun),因此不能 `import {$} from "bun"`。改用 `createRequire` 取 `child_process`
/// 的 `spawn`——Node 和 Bun 都支持,把 payload JSON 写进 poke-hook 的 stdin。
/// plugin 用 `export default`(与社区 plugin 一致的加载约定)。
const OPENCODE_PLUGIN_JS: &str = include_str!("opencode-plugin.js");

/// 占位符替换后的 plugin 全文。
pub(crate) fn opencode_plugin_contents() -> String {
    OPENCODE_PLUGIN_JS.replace("__POKE_HOOK_BIN__", &hook_bin_path().display().to_string())
}

pub(crate) fn cmd_install_opencode() {
    if !ensure_binary_installed() {
        return;
    }
    let plugin_path = opencode_plugin_path();
    if let Some(parent) = plugin_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            print_json(&serde_json::json!({
                "status": "error",
                "message": format!("Failed to create {}: {}", parent.display(), e),
            }));
            return;
        }
    }

    record_pre_write_snapshot("opencode", &plugin_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&plugin_path);

    // plugin 是 PokePoke 独占的固定文件名,直接覆盖即幂等(不像共享配置需合并)。
    if let Err(e) = fs::write(&plugin_path, opencode_plugin_contents()) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to write {}: {}", plugin_path.display(), e),
        }));
        return;
    }

    if let Err(e) = verify::verify_opencode_writes(&plugin_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("opencode install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!("Installed opencode plugin at {}", plugin_path.display()),
    }));
}

pub(crate) fn cmd_uninstall_opencode() {
    let plugin_path = opencode_plugin_path();
    if !plugin_path.exists() {
        print_json(
            &serde_json::json!({"status":"ok","message":"No opencode plugin found, nothing to do"}),
        );
        return;
    }

    record_pre_write_snapshot("opencode", &plugin_path);

    // plugin 文件是 PokePoke 独占的,直接删除;不碰用户其他 plugin。
    match fs::remove_file(&plugin_path) {
        Ok(()) => print_json(&serde_json::json!({
            "status": "ok",
            "message": "Removed opencode plugin",
        })),
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to remove {}: {}", plugin_path.display(), e),
        })),
    }
}

/// opencode plugin 是否已接入:文件存在且内容含 PokePoke 标记 + `--source=opencode`。
pub(crate) fn opencode_plugin_configured(plugin_path: &std::path::Path) -> bool {
    fs::read_to_string(plugin_path)
        .map(|c| c.contains("PokePoke opencode integration") && c.contains("--source=opencode"))
        .unwrap_or(false)
}

/// 已接入但 plugin 内烘焙的 hook 路径与当前 `~/.local/bin/poke-hook` 不一致 →
/// 判为过期(drift),交由 detect_hook_drift 走"告警 + 一键修复"。
pub(crate) fn opencode_plugin_outdated(plugin_path: &std::path::Path) -> bool {
    let Ok(content) = fs::read_to_string(plugin_path) else {
        return false;
    };
    if !content.contains("PokePoke opencode integration") {
        return false;
    }
    let expected = hook_bin_path().display().to_string();
    !content.contains(&expected)
}

pub(crate) fn cmd_check_opencode() {
    let installed = hook_bin_path().exists();
    let plugin_path = opencode_plugin_path();
    let hooks_configured = plugin_path.exists() && opencode_plugin_configured(&plugin_path);
    let hooks_outdated = hooks_configured && opencode_plugin_outdated(&plugin_path);

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured,
    }));
}
