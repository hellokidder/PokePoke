//! Source 判定与 task_id / workspace 推导(见 `docs/hook-events.md` 「Source 判定」段)。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use serde_json::Value;
use std::env;
use std::path::PathBuf;

use super::grok;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Source {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Pi,
    Omp,
    Kimi,
    Grok,
    Dsh,
}

impl Source {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Codex => "codex",
            Source::Cursor => "cursor",
            Source::OpenCode => "opencode",
            Source::Pi => "pi",
            Source::Omp => "omp",
            Source::Kimi => "kimi",
            Source::Grok => "grok",
            Source::Dsh => "dsh",
        }
    }

    pub(crate) fn task_id_prefix(&self) -> &'static str {
        match self {
            Source::ClaudeCode => "cc",
            Source::Codex => "codex",
            Source::Cursor => "cursor",
            Source::OpenCode => "opencode",
            Source::Pi => "pi",
            Source::Omp => "omp",
            Source::Kimi => "kimi",
            Source::Grok => "grok",
            Source::Dsh => "dsh",
        }
    }

    pub(crate) fn from_arg(value: &str) -> Option<Source> {
        match value {
            "cc" => Some(Source::ClaudeCode),
            "codex" => Some(Source::Codex),
            "cursor" => Some(Source::Cursor),
            "opencode" => Some(Source::OpenCode),
            "pi" => Some(Source::Pi),
            "omp" => Some(Source::Omp),
            "kimi" => Some(Source::Kimi),
            "grok" => Some(Source::Grok),
            "dsh" => Some(Source::Dsh),
            _ => None,
        }
    }
}

/// 从 argv 中解析 `--source=<x>` flag。位置任意，非法值返回 None。
/// 仅 hook mode 内部使用：CLI 子命令路由（`--install` 等）已在 `main` 优先短路，
/// 不会进入 hook mode 也就不会被本函数处理。
pub(crate) fn parse_source_arg(args: &[String]) -> Option<Source> {
    args.iter()
        .find_map(|a| a.strip_prefix("--source="))
        .and_then(Source::from_arg)
}

/// Source 判定（详见 `docs/hook-events.md` 「Source 判定」段）：
///   1. Grok runner 注入的 `GROK_HOOK_EVENT`（强信号，不可伪造）
///      Grok 默认合并读 `~/.claude/settings.json` / `~/.cursor/hooks.json`，
///      即使用户只接了 CC/Cursor，在 Grok 会话里也会 spawn 带 `--source=cc` 的
///      poke-hook；必须用 env 纠偏，否则会生成幽灵 cc/cursor session。
///   2. payload 独占字段
///        - `workspace_roots` / `workspaceRoots` → Cursor
///        - `turn_id` → Codex
///   3. install-time 钉死的 `--source=<x>` arg
///   4. 兜底 → ClaudeCode
pub(crate) fn detect_source(arg_source: Option<Source>, data: &Value) -> Source {
    if grok::is_grok_hook_runtime() {
        return Source::Grok;
    }
    if data.get("workspace_roots").is_some() || data.get("workspaceRoots").is_some() {
        return Source::Cursor;
    }
    if data.get("turn_id").is_some() {
        return Source::Codex;
    }
    arg_source.unwrap_or(Source::ClaudeCode)
}

/// Pick the first non-empty string value from multiple possible keys.
pub(crate) fn pick_str(data: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = data.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// CC StopFailure 官方 schema 用 `error` 标识类型；保留旧 key 兼容。
pub(crate) fn pick_stop_failure_reason(data: &Value) -> String {
    pick_str(data, &["error", "reason", "stop_reason", "error_type"])
        .unwrap_or_else(|| "unknown".into())
}

/// CC StopFailure 的 `last_assistant_message` 含用户可见的错误原文；
/// pi 扩展(2026-08-04)转发 `detail`(截断 ≤300 字符的错误原文)。
pub(crate) fn pick_stop_failure_detail(data: &Value) -> String {
    pick_str(
        data,
        &["last_assistant_message", "lastAssistantMessage", "detail"],
    )
    .unwrap_or_default()
}

/// Stop payload 的 `background_tasks` 列出未完成的后台任务(子 agent + 后台 shell,带 status)。
/// 有任一非 completed 项 → agent 还在等异步结果,这次 Stop 只是 turn 边界、不是真完成,
/// 不该弹"完成" popup。字段缺失/空数组/全完成 → false(按现状弹)。
/// 详见 memory: project_stop_background_tasks_signal(实验确认 cc 2.1.199 Stop 自带此字段)。
pub(crate) fn has_running_background_tasks(data: &Value) -> bool {
    let tasks = data
        .get("background_tasks")
        .or_else(|| data.get("backgroundTasks"));
    tasks
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .any(|t| t.get("status").and_then(|s| s.as_str()) != Some("completed"))
        })
        .unwrap_or(false)
}

pub(crate) fn pick_notification_type(data: &Value) -> Option<String> {
    pick_str(data, &["notification_type", "notificationType", "level"])
}

pub(crate) fn external_session_id(data: &Value, source: Source) -> Option<String> {
    match source {
        Source::Cursor => pick_str(data, &["conversation_id", "conversationId"])
            .or_else(|| pick_str(data, &["generation_id", "generationId"])),
        _ => pick_str(data, &["session_id", "sessionId"]),
    }
}

pub(crate) fn session_short_key(session_key: &str) -> String {
    // Kimi session id 形如 `session_<uuid>`;旧实现直接 take(8) 得到恒定
    // `session_`(所有 Kimi session 撞同一个 task_id)。strip 后保留完整 uuid:
    // 32bit 截断碰撞概率可忽略,但格式正在改,一次到位——且改动仍在
    // Unreleased(1.1.3 未发布),此刻改无二次升级涟漪。
    // 其他 source(CC/Codex/Cursor/opencode/pi)的 session key 非 uuid 且
    // 8 位格式已随发布版存在,维持 take(8) 不扩大升级涟漪。
    if let Some(uuid) = session_key
        .strip_prefix("session_")
        .or_else(|| session_key.strip_prefix("session-"))
    {
        uuid.to_string()
    } else {
        session_key.chars().take(8).collect()
    }
}

pub(crate) fn build_task_id(data: &Value, source: Source) -> String {
    let session_key = external_session_id(data, source).unwrap_or_else(|| "unknown".into());
    let short_key = session_short_key(&session_key);
    format!("{}-{}", source.task_id_prefix(), short_key)
}

pub(crate) fn resolve_workspace_path(data: &Value, source: Source) -> String {
    match source {
        Source::Cursor => {
            let roots = data
                .get("workspace_roots")
                .or_else(|| data.get("workspaceRoots"));
            roots
                .and_then(|r| r.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_default()
        }
        _ => data["cwd"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| env::current_dir().unwrap_or_default().display().to_string()),
    }
}

pub(crate) fn project_name(cwd: &str) -> String {
    PathBuf::from(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
