//! Cursor 接入:install / uninstall / check(全局 `~/.cursor/hooks.json`)。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::constants::CURSOR_HOOK_EVENTS;
use super::deploy::{ensure_binary_installed, print_json};
use super::drift::cursor_hooks_outdated;
use super::paths::{cursor_settings_path, hook_bin_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use serde_json::Value;
use std::fs;

/// Check if a Cursor hook entry contains poke-hook.
pub(crate) fn cursor_entry_has_poke_hook(entry: &Value) -> bool {
    entry["command"]
        .as_str()
        .is_some_and(|c| c.contains("poke-hook"))
}

// ==================== Cursor CLI subcommands ====================

pub(crate) fn cmd_install_cursor() {
    if !ensure_binary_installed() {
        return;
    }
    let hooks_path = cursor_settings_path();
    let hook_command = format!("{} --source=cursor", hook_bin_path().display());
    record_pre_write_snapshot("cursor", &hooks_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&hooks_path);

    match install_cursor_to_path(&hooks_path, &hook_command) {
        Ok(()) => {
            if let Err(e) = verify::verify_cursor_writes(&hooks_path) {
                tx.rollback();
                print_json(&serde_json::json!({
                    "status": "error",
                    "message": format!("Cursor install verify failed (rolled back): {}", e),
                }));
                return;
            }
            print_json(&serde_json::json!({
                "status": "ok",
                "message": format!("Installed Cursor hooks at {}", hooks_path.display())
            }));
        }
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": e,
        })),
    }
}

pub(crate) fn cmd_uninstall_cursor() {
    let hooks_path = cursor_settings_path();
    record_pre_write_snapshot("cursor", &hooks_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&hooks_path);

    match uninstall_cursor_from_path(&hooks_path) {
        Ok(message) => {
            if let Err(e) = verify::verify_cursor_writes(&hooks_path) {
                tx.rollback();
                print_json(&serde_json::json!({
                    "status": "error",
                    "message": format!("Cursor uninstall verify failed (rolled back): {}", e),
                }));
                return;
            }
            print_json(&serde_json::json!({
                "status": "ok",
                "message": message,
            }));
        }
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": e,
        })),
    }
}

pub(crate) fn cmd_check_cursor() {
    let installed = hook_bin_path().exists();
    let hooks_path = cursor_settings_path();
    let hooks_configured = cursor_hooks_configured(&hooks_path);

    let hooks_value: Value = if hooks_path.exists() {
        let content = fs::read_to_string(&hooks_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let hooks_outdated = cursor_hooks_outdated(&hooks_value, "cursor");

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured
    }));
}

/// 把 PokePoke 的 hook 条目合入给定 `hooks.json`（全局或测试沙盒皆可）。
///
/// 合并规则（与项目级实现保持一致，不整体覆写用户配置）：
/// - 保留 `version: 1`（写入前补齐）
/// - 保留用户已有的非 PokePoke 条目（按 `cursor_entry_has_poke_hook` 过滤）
/// - PokePoke 自己的条目幂等：重复执行不会重复插入
/// - parse 失败一律 Err，交由上层决策（`cmd_install_cursor` 会中止并报错，
///   避免静默修复损坏用户配置，详见 brief 风险 1）
pub(crate) fn install_cursor_to_path(
    hooks_path: &std::path::Path,
    hook_command: &str,
) -> Result<(), String> {
    if let Some(parent) = hooks_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let mut hooks: Value = if hooks_path.exists() {
        let content = fs::read_to_string(hooks_path)
            .map_err(|e| format!("Failed to read {}: {}", hooks_path.display(), e))?;
        // 空文件当作空对象处理，非空但解析失败则中止，避免误伤用户配置。
        if content.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&content).map_err(|e| {
                format!(
                    "Failed to parse {}: {} (aborting to avoid overwriting user config)",
                    hooks_path.display(),
                    e
                )
            })?
        }
    } else {
        serde_json::json!({})
    };

    if !hooks.is_object() {
        return Err(format!(
            "{} root is not a JSON object (aborting to avoid overwriting user config)",
            hooks_path.display()
        ));
    }

    hooks["version"] = serde_json::json!(1);
    let hooks_obj = hooks
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    // 内层 hooks 字段可能不是 object(如 "hooks": "broken"),防御性 reset,
    // 避免 below 循环里 hooks_obj.as_object_mut().unwrap() panic。root 已在上方守卫。
    if !hooks_obj.is_object() {
        *hooks_obj = serde_json::json!({});
    }

    let new_entry = serde_json::json!({
        "command": hook_command,
        "timeout": 8,
    });

    for event in CURSOR_HOOK_EVENTS {
        let event_hooks = hooks_obj
            .as_object_mut()
            .unwrap()
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));

        if let Some(arr) = event_hooks.as_array_mut() {
            arr.retain(|entry| !cursor_entry_has_poke_hook(entry));
            arr.push(new_entry.clone());
        }
    }

    let pretty = serde_json::to_string_pretty(&hooks).unwrap_or_default();
    fs::write(hooks_path, &pretty)
        .map_err(|e| format!("Failed to write {}: {}", hooks_path.display(), e))
}

/// 从给定 `hooks.json` 移除 PokePoke 条目；保留用户其他 hooks。
///
/// 卸载风格对齐 CC：空数组事件 key 清理；若 `hooks` 整体为空，仅保留
/// `{"version":1}`，不删除文件。用户自定义 hooks 永远不被我们碰。
///
/// 返回值作为给用户的提示文案（附带旧版按项目接入的手动清理说明）。
pub(crate) fn uninstall_cursor_from_path(hooks_path: &std::path::Path) -> Result<String, String> {
    if !hooks_path.exists() {
        // 统一返回相同文案，避免用户看到 "no file" 误以为出错；同时提示旧版
        // 项目级接入残留需要手动清理。
        return Ok(legacy_cursor_uninstall_notice());
    }

    let content = fs::read_to_string(hooks_path)
        .map_err(|e| format!("Failed to read {}: {}", hooks_path.display(), e))?;
    let mut hooks: Value = if content.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse {}: {} (aborting to avoid overwriting user config)",
                hooks_path.display(),
                e
            )
        })?
    };

    if let Some(hooks_obj) = hooks.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_, event_hooks) in hooks_obj.iter_mut() {
            if let Some(arr) = event_hooks.as_array_mut() {
                arr.retain(|entry| !cursor_entry_has_poke_hook(entry));
            }
        }
        hooks_obj.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    }

    // 与 CC 卸载风格对齐：`hooks` 清空后仍保留文件 + `version` 字段，
    // 用户如果原本就有其他版本字段也不至于丢失；避免"文件消失"在用户侧
    // 造成配置来源感知混乱。
    if hooks["hooks"].as_object().is_none_or(|h| h.is_empty()) {
        if let Some(obj) = hooks.as_object_mut() {
            obj.remove("hooks");
        }
    }
    if hooks.as_object().is_none_or(|o| o.is_empty()) {
        hooks = serde_json::json!({"version": 1});
    } else {
        hooks["version"] = serde_json::json!(1);
    }

    let pretty = serde_json::to_string_pretty(&hooks).unwrap_or_default();
    fs::write(hooks_path, &pretty)
        .map_err(|e| format!("Failed to write {}: {}", hooks_path.display(), e))?;
    Ok(legacy_cursor_uninstall_notice())
}

pub(crate) fn legacy_cursor_uninstall_notice() -> String {
    // 与 brief 风险 5 呼应：旧版按项目接入会在各 project/.cursor/hooks.json
    // 残留，这里无法自动枚举所有路径，只能给用户明确的手动清理指引。
    "Removed poke-hook from Cursor user-level hooks. \
     If you previously installed per-project (~/<project>/.cursor/hooks.json), \
     please remove them manually."
        .to_string()
}

/// 判断给定 `hooks.json` 是否"完整接入"——四个 Cursor 事件都含 PokePoke 条目。
/// 不存在或 parse 失败一律视为未配置，避免把破损文件误当成已接入。
pub(crate) fn cursor_hooks_configured(hooks_path: &std::path::Path) -> bool {
    if !hooks_path.exists() {
        return false;
    }
    let content = fs::read_to_string(hooks_path).unwrap_or_default();
    let val: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    CURSOR_HOOK_EVENTS.iter().all(|event| {
        val["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(cursor_entry_has_poke_hook))
    })
}
