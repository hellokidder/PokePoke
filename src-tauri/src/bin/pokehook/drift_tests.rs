//! Hook config drift detection 相关测试。
//! 从 `tests.rs` 抽出(AGENTS.md §4a,行为零变化)——2026-08-12 的 outdated
//! 修复(事件覆盖度维度)让 tests.rs 超 file-size ratchet,按 §5 选项①拆出。

use super::super::{
    cc_hooks_outdated, codex_feature_enabled, codex_hooks_outdated, cursor_hooks_outdated,
    expected_hook_command, hook_entry_outdated, CURSOR_HOOK_EVENTS,
};
use serde_json::json;
use serial_test::serial;

/// 还原 cmd_install_cc 当前写出形状(command 含 --source=<x>)。
fn cc_settings_with_command(cmd: &str) -> serde_json::Value {
    let mut hook_obj = serde_json::Map::new();
    hook_obj.insert("type".into(), json!("command"));
    hook_obj.insert("command".into(), json!(cmd));
    let group = json!({"hooks": [hook_obj]});
    let mut hooks = serde_json::Map::new();
    // 必须覆盖 CC_HOOK_EVENTS 全集(含 2026-08-19 新增的 PermissionRequest),
    // 否则会被事件缺失判定为 outdated。
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Notification",
        "PermissionRequest",
        "Stop",
        "StopFailure",
    ] {
        hooks.insert(event.to_string(), json!([group]));
    }
    json!({ "hooks": hooks })
}

fn cursor_hooks_with_command(cmd: &str) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    entry.insert("command".into(), json!(cmd));
    entry.insert("timeout".into(), json!(8));
    let mut hooks = serde_json::Map::new();
    // 必须覆盖 CURSOR_HOOK_EVENTS 全集,否则会被事件缺失判定为 outdated
    // (2026-08-12 起 outdated 增加事件覆盖度维度,见 cursor_hooks_outdated)。
    for event in CURSOR_HOOK_EVENTS {
        hooks.insert((*event).to_string(), json!([entry]));
    }
    json!({"version": 1, "hooks": hooks})
}

fn codex_hooks_with_command(cmd: &str) -> serde_json::Value {
    let mut hook_obj = serde_json::Map::new();
    hook_obj.insert("type".into(), json!("command"));
    hook_obj.insert("command".into(), json!(cmd));
    let group = json!({"hooks": [hook_obj]});
    let mut hooks = serde_json::Map::new();
    // 4 个事件需与 CODEX_HOOK_EVENTS 保持一致(SessionStart / UserPromptSubmit /
    // Stop / PermissionRequest);加新事件时这里也要补。
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Stop",
        "PermissionRequest",
    ] {
        hooks.insert(event.to_string(), json!([group]));
    }
    json!({ "hooks": hooks })
}

#[test]
fn cc_hooks_outdated_true_when_command_lacks_source_arg() {
    // 升级残留场景:旧版本写入裸命令,无 --source=cc 后缀
    let legacy = cc_settings_with_command("/Users/me/.local/bin/poke-hook");
    assert!(cc_hooks_outdated(&legacy, "cc"));
}

#[test]
#[serial]
fn cc_hooks_outdated_false_when_command_matches_expected_exactly() {
    // command 完全等于当前部署 target → healthy
    // v1.0.1 review H3 后:drift 判「字符串完全等值」而非「含 needle」
    let expected = expected_hook_command("cc");
    let modern = cc_settings_with_command(&expected);
    assert!(!cc_hooks_outdated(&modern, "cc"));

    // 完全未安装:not outdated(避免本启动自检给"主动选择不接入"的用户自动装上)
    let empty = json!({});
    assert!(!cc_hooks_outdated(&empty, "cc"));
}

#[test]
fn cc_hooks_outdated_true_when_command_path_diverges() {
    // review H3 场景:旧路径二进制(如 /Volumes/xxx/poke-hook)+ 正确 arg
    // 此前 `!contains("--source=cc")` 会报 healthy,现在字符串等值判定 → outdated
    let stale_path = cc_settings_with_command("/Volumes/old/PokePoke/poke-hook --source=cc");
    assert!(cc_hooks_outdated(&stale_path, "cc"));
}

#[test]
#[serial]
fn cc_hooks_outdated_true_when_command_has_extra_suffix() {
    // command 尾部追加参数(如 --debug)也算 outdated:§9b 硬边界要求字符串等值。
    // 真实世界没有用户会手动 append,即使有一键修复也应把它回归 canonical。
    let expected = expected_hook_command("cc");
    let extended = cc_settings_with_command(&format!("{} --debug", expected));
    assert!(cc_hooks_outdated(&extended, "cc"));
}

#[test]
fn cc_hooks_outdated_true_when_events_missing_from_previous_install() {
    // 旧版本装机没有 PermissionRequest(2026-08-19 前),升级后 CC_HOOK_EVENTS
    // 扩到 6 个。command 是 canonical 的(无路径漂移),但事件集不全 → 必须判
    // outdated,否则 auto-connect 会当成"用户断开"跳过,新事件永远装不上。
    let mut hook_obj = serde_json::Map::new();
    hook_obj.insert("type".into(), json!("command"));
    hook_obj.insert("command".into(), json!(expected_hook_command("cc")));
    let group = json!({"hooks": [hook_obj]});
    let mut hooks = serde_json::Map::new();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        hooks.insert(event.to_string(), json!([group]));
    }
    let legacy = json!({ "hooks": hooks });
    assert!(cc_hooks_outdated(&legacy, "cc"));

    // 完全没有我们的 entry(从未安装 / 用户断开)→ 不算 outdated
    assert!(!cc_hooks_outdated(&json!({}), "cc"));
    let user_only = json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"command": "/opt/user/other-hook"}]}]
        }
    });
    assert!(!cc_hooks_outdated(&user_only, "cc"));
}

#[test]
fn codex_hooks_outdated_true_when_command_lacks_source_arg() {
    let legacy = codex_hooks_with_command("/Users/me/.local/bin/poke-hook");
    assert!(codex_hooks_outdated(&legacy, "codex"));
}

#[test]
#[serial]
fn codex_hooks_outdated_false_when_command_matches_expected_exactly() {
    let expected = expected_hook_command("codex");
    let modern = codex_hooks_with_command(&expected);
    assert!(!codex_hooks_outdated(&modern, "codex"));
    assert!(!codex_hooks_outdated(&json!({}), "codex"));
}

#[test]
fn codex_hooks_outdated_true_when_command_path_diverges() {
    // review H3:codex 侧同样要挡老路径二进制
    let stale = codex_hooks_with_command("/Volumes/old/PokePoke/poke-hook --source=codex");
    assert!(codex_hooks_outdated(&stale, "codex"));
}

// ---- codex_feature_enabled: canonical key + deprecated alias ----
// 2026-05 codex 把 `[features].codex_hooks` 改名为 `[features].hooks`,旧名仍是
// deprecated alias。check 端必须同时识别两个键,否则曾被老版 PokePoke 装过的
// 用户升级后会被误报"未启用"。

#[test]
fn codex_feature_enabled_recognizes_canonical_hooks_key() {
    // 新装(canonical key 写入)
    let toml = "[features]\nhooks = true\n";
    assert!(codex_feature_enabled(toml));
}

#[test]
fn codex_feature_enabled_recognizes_deprecated_codex_hooks_alias() {
    // 老用户场景:config 里只有 deprecated alias
    let toml = "[features]\ncodex_hooks = true\n";
    assert!(codex_feature_enabled(toml));
}

#[test]
fn codex_feature_enabled_recognizes_both_keys_present() {
    // 升级路径:老用户既有 codex_hooks,PokePoke 升级后又写了 hooks
    let toml = "[features]\nhooks = true\ncodex_hooks = true\n";
    assert!(codex_feature_enabled(toml));
}

#[test]
fn codex_feature_enabled_false_when_no_keys() {
    // 都没有 → 未启用,触发 Connect 引导
    assert!(!codex_feature_enabled(""));
    assert!(!codex_feature_enabled("[features]\n"));
    // 显式 disable 也算未启用
    assert!(!codex_feature_enabled("[features]\nhooks = false\n"));
    assert!(!codex_feature_enabled(
        "[features]\nhooks = false\ncodex_hooks = false\n"
    ));
}

#[test]
fn cursor_hooks_outdated_true_when_command_lacks_source_arg() {
    let legacy = cursor_hooks_with_command("/Users/me/.local/bin/poke-hook");
    assert!(cursor_hooks_outdated(&legacy, "cursor"));
}

#[test]
#[serial]
fn cursor_hooks_outdated_false_when_command_matches_expected_exactly() {
    let expected = expected_hook_command("cursor");
    let modern = cursor_hooks_with_command(&expected);
    assert!(!cursor_hooks_outdated(&modern, "cursor"));
    assert!(!cursor_hooks_outdated(&json!({}), "cursor"));

    // 用户的非 PokePoke 条目混在同一事件里:不影响判定
    // (未含 poke-hook 子串的 entry → 不视为 PokePoke 自有 → 不算 outdated)
    let mut mixed_hooks = serde_json::Map::new();
    for event in CURSOR_HOOK_EVENTS {
        mixed_hooks.insert(
            (*event).to_string(),
            json!([
                { "command": "/opt/user/other-hook", "timeout": 5 },
                { "command": expected.clone(), "timeout": 8 },
            ]),
        );
    }
    let mixed = json!({"version": 1, "hooks": mixed_hooks});
    assert!(!cursor_hooks_outdated(&mixed, "cursor"));
}

#[test]
fn cursor_hooks_outdated_true_when_events_missing_from_previous_install() {
    // 旧版本装机只有 4 个事件,升级后 CURSOR_HOOK_EVENTS 扩到 6 个
    // (subagentStart / subagentStop)。command 是 canonical 的(无路径漂移),
    // 但事件集不全 → 必须判 outdated,否则 auto-connect 会当成"用户断开"跳过。
    let mut entry = serde_json::Map::new();
    entry.insert("command".into(), json!(expected_hook_command("cursor")));
    entry.insert("timeout".into(), json!(8));
    let mut hooks = serde_json::Map::new();
    for event in ["sessionStart", "beforeSubmitPrompt", "stop", "sessionEnd"] {
        hooks.insert(event.to_string(), json!([entry]));
    }
    let legacy = json!({"version": 1, "hooks": hooks});
    assert!(cursor_hooks_outdated(&legacy, "cursor"));

    // 完全没有我们的 entry(从未安装 / 用户断开)→ 不算 outdated
    assert!(!cursor_hooks_outdated(&json!({}), "cursor"));
    let user_only = json!({
        "version": 1,
        "hooks": {
            "sessionStart": [{"command": "/opt/user/other-hook", "timeout": 5}]
        }
    });
    assert!(!cursor_hooks_outdated(&user_only, "cursor"));
}

#[test]
fn cursor_hooks_outdated_true_when_command_path_diverges() {
    let stale = cursor_hooks_with_command("/Volumes/old/PokePoke/poke-hook --source=cursor");
    assert!(cursor_hooks_outdated(&stale, "cursor"));
}

#[test]
fn hook_entry_outdated_only_flags_poke_hook_entries() {
    // review H3 后:签名从 needle 改为 expected_cmd(完整字符串等值)
    let expected = "/x/poke-hook --source=cc";

    // PokePoke 自有 + 完全等于 expected → not outdated
    let modern = json!({"command": expected});
    assert!(!hook_entry_outdated(&modern, expected));

    // PokePoke 自有 + 缺 arg → outdated
    let bare = json!({"command": "/x/poke-hook"});
    assert!(hook_entry_outdated(&bare, expected));

    // PokePoke 自有 + 老路径 → outdated(H3 核心场景)
    let stale_path = json!({"command": "/Volumes/old/poke-hook --source=cc"});
    assert!(hook_entry_outdated(&stale_path, expected));

    // 不是 PokePoke 自有 → 永远 not outdated(不会触发自动重写用户的 hook)
    let third_party = json!({"command": "/opt/user/other-hook --source=cc"});
    assert!(!hook_entry_outdated(&third_party, expected));
}
