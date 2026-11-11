//! Kimi Code CLI 接入:install / uninstall / check(TOML array-of-tables)。
//!
//! Kimi Code CLI(`kimi`)的 hook 契约与 Claude Code 几乎一致:同名事件 + stdin JSON
//! + exit 2 block。区别仅在配置落点 -- Kimi 把 hook 直接写在 `~/.kimi-code/config.toml`
//! 的 `[[hooks]]` array-of-tables 里(非 JSON、非独立 hooks.json、无 feature flag)。
//! 本节用 `toml_edit` 对该 array 做幂等增删,**保留用户既有 providers/models/[thinking]
//! 等配置与注释**(toml_edit 是保留格式的编辑器,非 lossy round-trip)。
//!
//! Kimi config schema 严格(`[[hooks]]` 只允许 event/matcher/command/timeout,多字段
//! 会让 kimi 拒载 -- 已用 `kimi doctor config` 真机验证),故不能加 `managed_by` 之类
//! 标记字段。改用 `command` 子串("poke-hook" + "--source=kimi")识别我们自己的 entry,
//! 与 cc/codex 的 `contains_poke_hook` 同思路。uninstall 只删我们的 entry,用户的不动。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::constants::KIMI_HOOK_EVENTS;
use super::deploy::{ensure_binary_installed, print_json};
use super::drift::expected_hook_command;
use super::paths::{hook_bin_path, kimi_config_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use std::fs;

/// 一条 `[[hooks]]` entry 是否是 PokePoke 装的:command 同时含 `poke-hook` 与
/// `--source=kimi`。用子串而非精确匹配 -- reinstall 时 outdated 路径(旧 HOME /
/// 迁移后 hook_bin_path 变了)也能被识别并替换。
pub(crate) fn kimi_command_is_ours(cmd: &str) -> bool {
    cmd.contains("poke-hook") && cmd.contains("--source=kimi")
}

pub(crate) fn kimi_table_command(t: &toml_edit::Table) -> Option<&str> {
    t.get("command").and_then(|i| i.as_str())
}

/// 幂等合并:删掉我们既有的 entry(含 outdated 路径),再为每个 KIMI_HOOK_EVENTS
/// 追加一条 fresh `[[hooks]]`。保留用户手写的 [[hooks]]。纯函数,便于单测。
pub(crate) fn merge_poke_hooks_into_kimi(doc: &mut toml_edit::DocumentMut, hook_command: &str) {
    // `hooks` 必须是 array-of-tables。用户 config 里若残留非 array 的 `hooks`
    // (防御),重置为空 array-of-tables,对齐 cc 的防御性 reset。
    if !doc.get("hooks").is_some_and(|h| h.is_array_of_tables()) {
        doc["hooks"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let arr = doc["hooks"]
        .as_array_of_tables_mut()
        .expect("hooks is array_of_tables after ensure");
    let to_remove: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, t)| kimi_table_command(t).is_some_and(kimi_command_is_ours))
        .map(|(i, _)| i)
        .collect();
    for i in to_remove.into_iter().rev() {
        arr.remove(i);
    }
    // 不加 matcher(缺省匹配全部实例,如 SessionStart 的 startup+resume 都要);
    // 不加 timeout(用 kimi 默认 30s)。只写 event + command,避免触发 kimi 严格 schema 拒载。
    for event in KIMI_HOOK_EVENTS {
        let mut t = toml_edit::Table::new();
        t["event"] = toml_edit::value((*event).to_string());
        t["command"] = toml_edit::value(hook_command.to_string());
        arr.push(t);
    }
}

/// 删掉我们所有的 entry;array 空了就把 `hooks` key 整个移除(不留空 `[[hooks]]` 壳)。
/// 保留用户手写的 [[hooks]]。纯函数,便于单测。
pub(crate) fn remove_poke_hooks_from_kimi(doc: &mut toml_edit::DocumentMut) {
    let Some(arr) = doc
        .get_mut("hooks")
        .and_then(|h| h.as_array_of_tables_mut())
    else {
        return;
    };
    let to_remove: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, t)| kimi_table_command(t).is_some_and(kimi_command_is_ours))
        .map(|(i, _)| i)
        .collect();
    for i in to_remove.into_iter().rev() {
        arr.remove(i);
    }
    if arr.is_empty() {
        doc.remove("hooks");
    }
}

pub(crate) fn cmd_install_kimi() {
    if !ensure_binary_installed() {
        return;
    }

    let config_path = kimi_config_path();
    if let Some(parent) = config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let content = if config_path.exists() {
        fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    let expected_cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &expected_cmd);

    record_pre_write_snapshot("kimi", &config_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&config_path);

    if let Err(e) = fs::write(&config_path, doc.to_string()) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to write config.toml: {}", e),
        }));
        return;
    }

    if let Err(e) = verify::verify_kimi_writes(&config_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Kimi install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!(
            "Installed poke-hook to {} and configured {} hooks",
            expected_cmd,
            KIMI_HOOK_EVENTS.len()
        )
    }));
}

pub(crate) fn cmd_uninstall_kimi() {
    let config_path = kimi_config_path();
    if !config_path.exists() {
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "No config.toml found, nothing to do"
        }));
        return;
    }

    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() else {
        // 解析不动:不碰用户文件,只报已处理(无法判定就当没有我们的入口)。
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "config.toml unparseable, left untouched"
        }));
        return;
    };

    remove_poke_hooks_from_kimi(&mut doc);

    record_pre_write_snapshot("kimi", &config_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&config_path);

    let _ = fs::write(&config_path, doc.to_string());

    if let Err(e) = verify::verify_kimi_writes(&config_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Kimi uninstall verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": "Removed poke-hook from Kimi Code config"
    }));
}

pub(crate) fn cmd_check_kimi() {
    let installed = hook_bin_path().exists();
    let config_path = kimi_config_path();
    let doc = if config_path.exists() {
        fs::read_to_string(&config_path)
            .ok()
            .and_then(|c| c.parse::<toml_edit::DocumentMut>().ok())
    } else {
        None
    };

    let hooks_configured = doc.as_ref().is_some_and(kimi_hooks_configured);
    let hooks_outdated = doc.as_ref().is_some_and(kimi_hooks_outdated);

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured,
    }));
}

/// 我们的所有 KIMI_HOOK_EVENTS 是否都至少有一条我们的 entry。
pub(crate) fn kimi_hooks_configured(doc: &toml_edit::DocumentMut) -> bool {
    let Some(arr) = doc.get("hooks").and_then(|h| h.as_array_of_tables()) else {
        return false;
    };
    KIMI_HOOK_EVENTS.iter().all(|event| {
        arr.iter().any(|t| {
            t.get("event").and_then(|i| i.as_str()) == Some(*event)
                && kimi_table_command(t).is_some_and(kimi_command_is_ours)
        })
    })
}

/// 我们的 entry 里是否有 command 与当前 expected_hook_command("kimi") 不一致
/// (路径漂移:旧 HOME / 迁移后 hook 二进制位置变了),或事件集不全
/// (版本升级新增事件未补装,如 SubagentStart/SubagentStop)。
///
/// 「曾装过」信号 = 至少存在一条我们的 entry(命令含 poke-hook + --source=kimi)。
/// 只有这个信号成立时,事件集不全才算 outdated —— 从未安装 / 用户主动断开
/// (entry 全无)不算 outdated,避免自动装回用户不想要的端(§9b)。
pub(crate) fn kimi_hooks_outdated(doc: &toml_edit::DocumentMut) -> bool {
    let Some(arr) = doc.get("hooks").and_then(|h| h.as_array_of_tables()) else {
        return false;
    };
    let has_any_ours = arr
        .iter()
        .any(|t| kimi_table_command(t).is_some_and(kimi_command_is_ours));
    if !has_any_ours {
        return false;
    }
    let expected = expected_hook_command("kimi");
    let cmd_drift = arr.iter().any(|t| {
        kimi_table_command(t).is_some_and(|c| kimi_command_is_ours(c) && c != expected.as_str())
    });
    !kimi_hooks_configured(doc) || cmd_drift
}
