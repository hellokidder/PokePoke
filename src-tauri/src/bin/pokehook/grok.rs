//! Grok Build managed integration（`~/.grok/hooks/pokepoke.json`）。
//!
//! Grok 默认合并读 CC/Cursor 兼容 hook；hook mode 用 `GROK_HOOK_EVENT` 纠偏 source。

use super::build_payload::PROTOCOL_VERSION;
use super::*;
use serde_json::Value;

// Events we register for Grok Build(`~/.grok/hooks/pokepoke.json`)。
// 对齐 CC proven 生命周期 + SessionEnd(干净移除)。Notification 在 hook_mode
// 内按 notificationType 白名单升 Pending;Stop 仅 reason=end_turn 升 Idle。
pub(crate) const GROK_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

const GROK_HOOK_MARKER: &str = "PokePoke grok integration";

/// Grok hook runner 对每个子进程注入 `GROK_HOOK_EVENT`（见 xai-grok-hooks command runner）。
pub(crate) fn is_grok_hook_runtime() -> bool {
    env::var("GROK_HOOK_EVENT")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
}

/// Grok `Notification` 仅这两类应升 Pending；`task_complete` 等忽略。
pub(crate) fn grok_notification_is_pending(data: &Value) -> bool {
    matches!(
        pick_notification_type(data).as_deref(),
        Some("permission_prompt") | Some("elicitation_dialog")
    )
}

/// Grok 在 session 结束时会额外 fire 一次 `Stop`(reason 非 end_turn)；只把真 turn 结束当 Idle。
pub(crate) fn grok_stop_is_turn_end(data: &Value) -> bool {
    pick_str(data, &["reason"])
        .map(|r| r == "end_turn")
        .unwrap_or(true)
}

pub(crate) fn grok_notification_message(data: &Value) -> Option<String> {
    if !grok_notification_is_pending(data) {
        return None;
    }
    Some(pick_str(data, &["message", "title"]).unwrap_or_else(|| "Awaiting user input".to_string()))
}

pub(crate) fn grok_pending_notification_payload(
    task_id: &str,
    project: &str,
    cwd: &str,
    data: &Value,
    source: Source,
) -> Option<Value> {
    let message = grok_notification_message(data)?;
    Some(serde_json::json!({
        "task_id": task_id,
        "event_type": "pending",
        "title": project.to_string(),
        "message": message,
        "source": source.as_str(),
        "status": "pending",
        "workspace_path": cwd,
        // W9 review fix:grok 专用出口同样带协议版本(检测点无盲区)。
        "protocol_version": PROTOCOL_VERSION,
    }))
}

fn grok_home_dir() -> PathBuf {
    env::var("GROK_HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".grok"))
}

fn grok_hooks_path() -> PathBuf {
    grok_home_dir().join("hooks").join("pokepoke.json")
}

pub(crate) fn grok_hooks_document(hook_command: &str) -> Value {
    let handler = serde_json::json!({
        "type": "command",
        "command": hook_command,
    });
    let mut hooks_obj = serde_json::Map::new();
    for event in GROK_HOOK_EVENTS {
        hooks_obj.insert(
            (*event).to_string(),
            serde_json::json!([{ "hooks": [handler.clone()] }]),
        );
    }
    serde_json::json!({
        "description": GROK_HOOK_MARKER,
        "hooks": Value::Object(hooks_obj),
    })
}

/// Grok check 判定：文件存在、含 PokePoke marker，且每个 `GROK_HOOK_EVENTS`
/// 都至少注册了一条我们的 command handler。事件列表变更（新版本加了事件但旧
/// 装机未重装）会判 false → `connected=false`，让 UI/一键修复可见 —— 对齐
/// kimi 的 `kimi_hooks_configured` 逐事件校验（drift parity）。
pub(crate) fn grok_hooks_configured(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    if !content.contains(GROK_HOOK_MARKER) || !content.contains("--source=grok") {
        return false;
    }
    let Ok(v) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    let Some(hooks) = v.get("hooks").and_then(|h| h.as_object()) else {
        return false;
    };
    GROK_HOOK_EVENTS.iter().all(|event| {
        hooks
            .get(*event)
            .and_then(|groups| groups.as_array())
            .is_some_and(|groups| groups.iter().any(grok_group_has_our_handler))
    })
}

/// `{EVENT: [{"hooks": [{"type": "command", "command": ...}]}]}` group 中是否有
/// 我们的 handler。command 子串匹配 `poke-hook` + `--source=grok`，与 kimi 的
/// `kimi_command_is_ours` 同语义；binary 路径漂移归 `grok_hooks_outdated` 管。
fn grok_group_has_our_handler(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(|arr| arr.as_array())
        .is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                handler
                    .get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|cmd| cmd.contains("poke-hook") && cmd.contains("--source=grok"))
            })
        })
}

pub(crate) fn grok_hooks_outdated(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    if !grok_hooks_configured(path) {
        return false;
    }
    let expected = hook_bin_path().display().to_string();
    !content.contains(&expected)
}

pub(crate) fn cmd_install_grok() {
    if !ensure_binary_installed() {
        return;
    }
    let hooks_path = grok_hooks_path();
    if let Some(parent) = hooks_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            print_json(&serde_json::json!({
                "status": "error",
                "message": format!("Failed to create {}: {}", parent.display(), e),
            }));
            return;
        }
    }

    record_pre_write_snapshot("grok", &hooks_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&hooks_path);

    let hook_command = expected_hook_command("grok");
    let doc = grok_hooks_document(&hook_command);
    let pretty = serde_json::to_string_pretty(&doc).unwrap_or_default();
    if let Err(e) = fs::write(&hooks_path, &pretty) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to write {}: {}", hooks_path.display(), e),
        }));
        return;
    }

    if let Err(e) = verify::verify_grok_writes(&hooks_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Grok install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!(
            "Installed poke-hook to {} and configured {} hooks",
            hook_command,
            GROK_HOOK_EVENTS.len()
        )
    }));
}

pub(crate) fn cmd_uninstall_grok() {
    let hooks_path = grok_hooks_path();
    if !hooks_path.exists() {
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "No grok hook file found, nothing to do"
        }));
        return;
    }

    record_pre_write_snapshot("grok", &hooks_path);

    // pokepoke.json 是 PokePoke 独占文件,直接删除;删成功即终态,无需 verify+rollback
    // (verify_grok_writes 对缺失文件恒 Ok,与 opencode/pi uninstall 同模式)。
    match fs::remove_file(&hooks_path) {
        Ok(()) => print_json(&serde_json::json!({
            "status": "ok",
            "message": "Removed poke-hook from Grok hooks",
        })),
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to remove {}: {}", hooks_path.display(), e),
        })),
    }
}

pub(crate) fn cmd_check_grok() {
    let installed = hook_bin_path().exists();
    let hooks_path = grok_hooks_path();
    let hooks_configured = hooks_path.exists() && grok_hooks_configured(&hooks_path);
    let hooks_outdated = hooks_configured && grok_hooks_outdated(&hooks_path);

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured,
    }));
}

#[cfg(test)]
pub(crate) fn clear_grok_hook_env() {
    // `GROK_HOOK_EVENT` 是进程级 env;grok 相关单测必须清掉,避免污染其它 detect_source case。
    unsafe {
        std::env::remove_var("GROK_HOOK_EVENT");
    }
}
