//! Hook config drift detection
//!
//! 判定三端 hook 配置是否「过期」。当前触发面(收敛到 AGENTS.md §9b 最小集:
//! 二进制存在 + entry 存在 + command 字符串等值):
//!   command != 当前部署 target 完整字符串
//!     (含 poke-hook 但路径 / arg / 尾部有别 = outdated,一键修复重装到 canonical)
//!
//! 语义约定:
//! - 完全未安装(settings 里无 PokePoke 条目)→ false(不算 outdated)
//! - 已安装且 command 完全等于 `<hook_bin_path> --source=<expected>` → false(最新)
//! - 已安装但 command 不等于上式(路径旧、arg 缺、末尾追加 flag、任何字符差别)→ true
//!
//! 「未安装 ≠ outdated」是关键:避免本启动自检逻辑给"主动选择不接入某端
//! 的用户"自动装上他没要的 hook 集成。
//!
//! v1.0.1 review H3:此前用 `!cmd.contains("--source=<x>")` 判定,
//!   `/Volumes/old/PokePoke/poke-hook --source=cc` 之类的旧路径二进制会被误报健康。
//!   §9b 硬边界明确要求「command 字符串等值」,本次收紧到此。
//!
//! 详见 `docs/archive/briefs/20260422-startup-hook-drift-repair.md` + review followup H3。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::cc::contains_poke_hook;
use super::constants::{CC_HOOK_EVENTS, CODEX_HOOK_EVENTS, CURSOR_HOOK_EVENTS};
use super::cursor::cursor_entry_has_poke_hook;
use super::paths::hook_bin_path;
use serde_json::Value;

/// 单条 hook entry / cursor entry 是否过期。
/// 「是 PokePoke 自有(command 含 poke-hook 子串)」+ 「command != expected 完整字符串」→ outdated。
/// 「不是 PokePoke 自有」→ 不算 outdated(不会触发自动重写用户的 hook)。
pub(crate) fn hook_entry_outdated(entry: &Value, expected_cmd: &str) -> bool {
    let cmd = entry["command"].as_str().unwrap_or("");
    if !cmd.contains("poke-hook") {
        return false;
    }
    cmd != expected_cmd
}

/// CC 风格 group（`{"hooks": [{"command": "..."}, ...]}`）下任一 hook entry 是否过期。
pub(crate) fn group_has_outdated_hook(group: &Value, expected_cmd: &str) -> bool {
    group["hooks"]
        .as_array()
        .is_some_and(|hooks| hooks.iter().any(|h| hook_entry_outdated(h, expected_cmd)))
}

/// 通用：扫给定事件列表下任一 group 是否过期（CC / Codex 共用 schema）。
pub(crate) fn any_event_group_outdated(root: &Value, events: &[&str], expected_cmd: &str) -> bool {
    events.iter().any(|event| {
        root["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|g| group_has_outdated_hook(g, expected_cmd)))
    })
}

/// 拼出当前部署 target 的完整 command 字符串。三端 install 时写入的完全一致的形式。
pub(crate) fn expected_hook_command(source_arg: &str) -> String {
    format!("{} --source={}", hook_bin_path().display(), source_arg)
}

pub(crate) fn cc_hooks_outdated(settings: &Value, expected_arg: &str) -> bool {
    let expected_cmd = expected_hook_command(expected_arg);
    // 「曾装过」信号 = 任一事件下存在我们的 entry(命令含 poke-hook)。只有这个
    // 信号成立时,事件集不全才算 outdated —— 从未安装 / 用户主动断开(entry 全无)
    // 不算 outdated,避免自动装回用户不想要的端(§9b)。镜像 cursor_hooks_outdated
    // 的「事件覆盖度」维度:升级新增事件(PermissionRequest 等)未补装时,CC 会
    // 卡在 connected=false + outdated=false,auto-connect 误判成「用户断开」跳过,
    // 新事件永远装不上(2026-08-19 PermissionRequest 修复)。
    let ours_in = |event: &str| {
        settings["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(contains_poke_hook))
    };
    let has_any_ours = CC_HOOK_EVENTS.iter().any(|e| ours_in(e));
    if !has_any_ours {
        return false;
    }
    let events_complete = CC_HOOK_EVENTS.iter().all(|e| ours_in(e));
    let cmd_drift = any_event_group_outdated(settings, CC_HOOK_EVENTS, &expected_cmd);
    !events_complete || cmd_drift
}

pub(crate) fn codex_hooks_outdated(hooks_json: &Value, expected_arg: &str) -> bool {
    let expected_cmd = expected_hook_command(expected_arg);
    any_event_group_outdated(hooks_json, CODEX_HOOK_EVENTS, &expected_cmd)
}

/// codex `[features]` 是否启用了 hooks。
/// canonical 键是 `hooks`(2026-05+),老 PokePoke 版本写的 `codex_hooks` 作为
/// deprecated alias 同样视为启用 —— 任一为 true 即可,避免升级后给曾被装过的
/// 用户误报"未启用"。
pub(crate) fn codex_feature_enabled(config_toml_content: &str) -> bool {
    let Ok(doc) = config_toml_content.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(features) = doc.get("features") else {
        return false;
    };
    let read = |key: &str| features.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    read("hooks") || read("codex_hooks")
}

/// Cursor 是否过期:升级新增事件(subagentStart/subagentStop 等)未补装、或
/// command 字符串漂移。
///
/// 「曾装过」信号 = 任一事件下存在我们的 entry(命令含 poke-hook)。只有这个
/// 信号成立时,事件集不全才算 outdated —— 从未安装 / 用户主动断开(entry 全无)
/// 不算 outdated,避免自动装回用户不想要的端(§9b)。
pub(crate) fn cursor_hooks_outdated(hooks: &Value, expected_arg: &str) -> bool {
    let expected_cmd = expected_hook_command(expected_arg);
    let ours_in = |event: &str| {
        hooks["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(cursor_entry_has_poke_hook))
    };
    let has_any_ours = CURSOR_HOOK_EVENTS.iter().any(|e| ours_in(e));
    if !has_any_ours {
        return false;
    }
    let events_complete = CURSOR_HOOK_EVENTS.iter().all(|e| ours_in(e));
    let cmd_drift = CURSOR_HOOK_EVENTS.iter().any(|event| {
        hooks["hooks"][event].as_array().is_some_and(|arr| {
            arr.iter()
                .any(|entry| hook_entry_outdated(entry, &expected_cmd))
        })
    });
    !events_complete || cmd_drift
}
