//! Claude Code 接入:install / uninstall / check + PokePoke 自有 hook 判定。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::constants::CC_HOOK_EVENTS;
use super::deploy::{ensure_binary_installed, print_json};
use super::drift::cc_hooks_outdated;
use super::paths::{claude_settings_path, hook_bin_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use serde_json::Value;
use std::fs;

// ==================== PokePoke 自有 hook 判定 ====================
//
// PokePoke 自己写入的 hook command 形如 `~/.local/bin/poke-hook --source=<x>`,
// 直接用 `command.contains("poke-hook")` 子串判定即可——足够精确,且
// 真实场景下"第三方 binary 撞名 poke-hook"几乎不存在。
// 详见 `docs/archive/scope-audit-20260422.md §3.4 / §8.2`。

/// Check if a hook group JSON contains a poke-hook command (CC/Codex format).
pub(crate) fn contains_poke_hook(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|h| {
            h["command"]
                .as_str()
                .is_some_and(|c| c.contains("poke-hook"))
        })
    })
}

/// 单条 hook entry 的 command 是否含 poke-hook 子串(判"这是 PokePoke 装的")。
/// install / uninstall 时按 entry 粒度 retain 用,保留同 group 内用户自己的 entry。
pub(crate) fn hook_entry_is_poke_hook(entry: &Value) -> bool {
    entry["command"]
        .as_str()
        .is_some_and(|c| c.contains("poke-hook"))
}

/// 从 CC/Codex 的 group 数组里清除所有 PokePoke entry:
/// 1) 遍历每个 group,从 `group["hooks"][]` 内按 entry 粒度删掉含 poke-hook 的项;
/// 2) 清空后的 group(hooks 数组变空)再从外层 group 数组里删掉。
/// 保留同 group 内用户自己的 entry(v1.0.1 review H2 修正:此前是 group 粒度 retain)。
pub(crate) fn drop_poke_hook_entries_from_groups(arr: &mut Vec<Value>) {
    for group in arr.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(|v| v.as_array_mut()) {
            hooks.retain(|h| !hook_entry_is_poke_hook(h));
        }
    }
    arr.retain(|g| {
        g.get("hooks")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
    });
}

// ==================== CC CLI subcommands ====================

pub(crate) fn cmd_install_cc() {
    if !ensure_binary_installed() {
        return;
    }

    // Read or create settings.json
    let settings_path = claude_settings_path();
    if let Some(parent) = settings_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut settings: Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Merge hooks
    // 防御:settings.json 可能合法解析成非 object(null / [] / 42 / 第三方工具写坏),
    // 直接 .as_object_mut().unwrap() 会 panic,首启 bootstrap 报 install failed。
    // 重置为空对象,对齐 Codex 的 merge_poke_hooks_into_codex 容错;损坏文件由
    // verify_cc_writes 兜底报错。
    if !settings.is_object() {
        settings = serde_json::json!({});
    }
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    // hooks 字段本身可能不是 object(如 "hooks": "broken"),同样防御性 reset,
    // 避免 below 循环里 hooks.as_object_mut().unwrap() panic。
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }

    let hook_command = format!("{} --source=cc", hook_bin_path().display());
    let new_entry = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "command": &hook_command,
        }]
    }]);

    for event in CC_HOOK_EVENTS {
        let event_hooks = hooks
            .as_object_mut()
            .unwrap()
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));

        if let Some(arr) = event_hooks.as_array_mut() {
            // entry 级 retain,保留用户塞在同 group 内的自己 entry(review H2)
            drop_poke_hook_entries_from_groups(arr);
        }
        if let Some(arr) = event_hooks.as_array_mut() {
            if let Some(new_arr) = new_entry.as_array() {
                arr.extend(new_arr.iter().cloned());
            }
        }
    }

    record_pre_write_snapshot("cc", &settings_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&settings_path);

    let pretty = serde_json::to_string_pretty(&settings).unwrap_or_default();
    if let Err(e) = fs::write(&settings_path, &pretty) {
        print_json(
            &serde_json::json!({"status":"error","message":format!("Failed to write settings: {}", e)}),
        );
        return;
    }

    if let Err(e) = verify::verify_cc_writes(&settings_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("CC install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!("Installed poke-hook to {} and configured {} hooks", hook_command, CC_HOOK_EVENTS.len())
    }));
}

pub(crate) fn cmd_uninstall_cc() {
    let settings_path = claude_settings_path();
    if !settings_path.exists() {
        print_json(
            &serde_json::json!({"status":"ok","message":"No settings file found, nothing to do"}),
        );
        return;
    }

    let content = fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".into());
    let mut settings: Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    const LEGACY_EVENTS: &[&str] = &["PreToolUse", "PostToolUse"];

    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event in CC_HOOK_EVENTS.iter().chain(LEGACY_EVENTS.iter()) {
            if let Some(event_hooks) = hooks.get_mut(*event).and_then(|v| v.as_array_mut()) {
                // entry 级 retain(review H2):同 group 内保留用户自己 entry;空 group 会被清掉
                drop_poke_hook_entries_from_groups(event_hooks);
            }
        }
        let empty_keys: Vec<String> = hooks
            .iter()
            .filter(|(_, v)| v.as_array().is_some_and(|a| a.is_empty()))
            .map(|(k, _)| k.clone())
            .collect();
        for key in empty_keys {
            hooks.remove(&key);
        }
        if hooks.is_empty() {
            settings.as_object_mut().unwrap().remove("hooks");
        }
    }

    record_pre_write_snapshot("cc", &settings_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&settings_path);

    let pretty = serde_json::to_string_pretty(&settings).unwrap_or_default();
    let _ = fs::write(&settings_path, &pretty);

    if let Err(e) = verify::verify_cc_writes(&settings_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("CC uninstall verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(
        &serde_json::json!({"status":"ok","message":"Removed poke-hook from Claude Code settings"}),
    );
}

pub(crate) fn cmd_check_cc() {
    let installed = hook_bin_path().exists();

    let settings: Value = if claude_settings_path().exists() {
        let content = fs::read_to_string(claude_settings_path()).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let hooks_configured = CC_HOOK_EVENTS.iter().all(|event| {
        settings["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(contains_poke_hook))
    });

    let hooks_outdated = cc_hooks_outdated(&settings, "cc");

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured
    }));
}
