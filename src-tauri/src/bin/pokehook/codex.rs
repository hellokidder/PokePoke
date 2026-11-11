//! Codex CLI 接入:install / uninstall / check + hooks.json 合并。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::cc::{contains_poke_hook, drop_poke_hook_entries_from_groups};
use super::constants::CODEX_HOOK_EVENTS;
use super::deploy::{ensure_binary_installed, print_json};
use super::drift::{codex_feature_enabled, codex_hooks_outdated};
use super::paths::{codex_dir, hook_bin_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use serde_json::Value;
use std::fs;

/// 把 PokePoke 的 hook 合并进 Codex `hooks.json` 的 JSON 值:确保 root 是
/// `{"hooks": {...}}`,对每个 `CODEX_HOOK_EVENTS` 事件先移除既有的 PokePoke
/// group、再 append 一条,幂等且保留用户在其它事件/同事件里手写的 group。
/// 若 root 不是 object(损坏)则重置为空对象(与 install 读文件时的容错一致)。
pub(crate) fn merge_poke_hooks_into_codex(hooks_json: &mut Value, hook_command: &str) {
    if !hooks_json.is_object() {
        *hooks_json = serde_json::json!({});
    }
    let hooks = hooks_json
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }

    let new_group = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": hook_command,
        }]
    });

    for event in CODEX_HOOK_EVENTS {
        let event_hooks = hooks
            .as_object_mut()
            .unwrap()
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        if let Some(arr) = event_hooks.as_array_mut() {
            // entry 级 retain,保留用户塞在同 group 内的自己 entry(review H2)
            drop_poke_hook_entries_from_groups(arr);
            arr.push(new_group.clone());
        }
    }
}

// ==================== Codex CLI subcommands ====================

pub(crate) fn cmd_install_codex() {
    if !ensure_binary_installed() {
        return;
    }

    let dir = codex_dir();
    let _ = fs::create_dir_all(&dir);

    let hooks_path = dir.join("hooks.json");
    let config_path = dir.join("config.toml");

    // L1 transaction:写两份(hooks.json + config.toml),verify 失败一起回滚。
    let mut tx = WriteTransaction::new();
    tx.snapshot(&hooks_path);
    tx.snapshot(&config_path);

    // 1. Merge into hooks.json —— 读现有配置后只增改 PokePoke 的事件,保留用户
    //    在其它事件(甚至同一事件里)手写的 hook。绝不整文件覆写(否则会静默
    //    抹掉用户既有 Codex hooks),对齐 CC/Cursor 的合并语义。
    let hook_command = format!("{} --source=codex", hook_bin_path().display());

    let mut hooks_json: Value = if hooks_path.exists() {
        let content = fs::read_to_string(&hooks_path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    merge_poke_hooks_into_codex(&mut hooks_json, &hook_command);
    record_pre_write_snapshot("codex-hooks", &hooks_path);
    if let Err(e) = fs::write(
        &hooks_path,
        serde_json::to_string_pretty(&hooks_json).unwrap_or_default(),
    ) {
        print_json(
            &serde_json::json!({"status":"error","message":format!("Failed to write hooks.json: {}", e)}),
        );
        return;
    }

    // 2. Edit config.toml: enable feature flag。**不写 `hooks = "./hooks.json"`**
    //    —— codex 0.124+ 把这一行解析成 string 直接拒,见
    //    https://developers.openai.com/codex/config-reference(`hooks` 字段
    //    schema 是 inline table)。codex 会自动发现 `~/.codex/hooks.json`,
    //    所以根本不需要在 config.toml 里指路。
    //    如果用户的 config 里残留旧版 PokePoke 写过的 string `hooks` 项,
    //    顺手清掉,免得用户升级后还卡在 codex 启动报错。inline `[hooks]`
    //    table 是用户自己的合法配置,不动。
    let content = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    if doc
        .get("hooks")
        .and_then(|v| v.as_value())
        .and_then(|v| v.as_str())
        .is_some()
    {
        doc.remove("hooks");
    }

    // 写 canonical key `hooks`(2026-05 codex docs: "Use `hooks` as the canonical feature key.
    // `codex_hooks` still works as a deprecated alias.")。
    // 用户既存的 `codex_hooks` 键不动 —— 老 PokePoke 版本升级上来的用户保留兼容性,
    // check 端会同时识别两个键。
    // features 必须是 table 才能往下写 hooks。用户 config.toml 里若残留
    // `features = true` 之类非 table 值,直接 `doc["features"]["hooks"]` 下标会
    // 触发 toml_edit 的 `.expect("index not found")` panic。非 table(含缺失)
    // 统一重置为空 table。
    if !doc.get("features").is_some_and(|f| f.is_table()) {
        doc["features"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["features"]["hooks"] = toml_edit::value(true);

    record_pre_write_snapshot("codex-config", &config_path);
    if let Err(e) = fs::write(&config_path, doc.to_string()) {
        print_json(
            &serde_json::json!({"status":"error","message":format!("Failed to write config.toml: {}", e)}),
        );
        return;
    }

    if let Err(e) = verify::verify_codex_writes(&dir) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Codex install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": "Installed poke-hook for Codex CLI: hooks.json created + config.toml updated"
    }));
}

pub(crate) fn cmd_uninstall_codex() {
    let dir = codex_dir();
    let hooks_path = dir.join("hooks.json");
    let config_path = dir.join("config.toml");

    // L1 transaction:写两份外部文件,verify 失败一起回滚。
    let mut tx = WriteTransaction::new();
    tx.snapshot(&hooks_path);
    tx.snapshot(&config_path);

    // Clean hooks.json
    if hooks_path.exists() {
        record_pre_write_snapshot("codex-hooks", &hooks_path);
        let content = fs::read_to_string(&hooks_path).unwrap_or_default();
        if let Ok(mut val) = serde_json::from_str::<Value>(&content) {
            if let Some(hooks) = val.get_mut("hooks").and_then(|h| h.as_object_mut()) {
                for (_, event_hooks) in hooks.iter_mut() {
                    if let Some(arr) = event_hooks.as_array_mut() {
                        arr.retain(|group| !contains_poke_hook(group));
                    }
                }
                hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
            }
            if val["hooks"].as_object().is_none_or(|h| h.is_empty()) {
                let _ = fs::remove_file(&hooks_path);
            } else {
                let _ = fs::write(
                    &hooks_path,
                    serde_json::to_string_pretty(&val).unwrap_or_default(),
                );
            }
        }
    }

    // 只清理 PokePoke 旧版本写过的 legacy string `hooks = "./hooks.json"`
    // (codex 现在 reject 这个格式)。inline `[hooks]` table 是用户自己的
    // 合法配置,uninstall 不该一起带走。feature flag 同理保留 ——
    // 可能是用户独立偏好,与 PokePoke 管理的 hook 入口解耦。
    if config_path.exists() {
        record_pre_write_snapshot("codex-config", &config_path);
        let content = fs::read_to_string(&config_path).unwrap_or_default();
        if let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() {
            let is_legacy_string = doc
                .get("hooks")
                .and_then(|v| v.as_value())
                .and_then(|v| v.as_str())
                .is_some();
            if is_legacy_string {
                doc.remove("hooks");
                let _ = fs::write(&config_path, doc.to_string());
            }
        }
    }

    if let Err(e) = verify::verify_codex_writes(&dir) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Codex uninstall verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(
        &serde_json::json!({"status":"ok","message":"Removed poke-hook from Codex CLI config"}),
    );
}

pub(crate) fn cmd_check_codex() {
    let installed = hook_bin_path().exists();
    let dir = codex_dir();

    let hooks_json: Value = {
        let hooks_path = dir.join("hooks.json");
        if hooks_path.exists() {
            let content = fs::read_to_string(&hooks_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        }
    };

    let hooks_configured = CODEX_HOOK_EVENTS.iter().all(|event| {
        hooks_json["hooks"][event]
            .as_array()
            .is_some_and(|arr| arr.iter().any(contains_poke_hook))
    });

    let hooks_outdated = codex_hooks_outdated(&hooks_json, "codex");

    let feature_enabled = {
        let config_path = dir.join("config.toml");
        if config_path.exists() {
            let content = fs::read_to_string(&config_path).unwrap_or_default();
            codex_feature_enabled(&content)
        } else {
            false
        }
    };

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "feature_enabled": feature_enabled,
        "connected": installed && hooks_configured && feature_enabled
    }));
}
