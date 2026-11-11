use crate::http_server::{process_notify, AppState, NotifyRequest};
use crate::sessions::{Session, SessionStatus};
use crate::{log_debug, log_warn};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

pub const CLAUDE_INTERRUPT_MARKER: &str = "[Request interrupted by user]";
pub const CLAUDE_INTERRUPTION_PENDING_MESSAGE: &str =
    "Claude Code conversation may have been interrupted. Click to inspect or continue.";

const DEFAULT_TAIL_BYTES: u64 = 64 * 1024;
const WATCH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptionMarker {
    pub key: String,
}

#[derive(Default)]
pub struct TranscriptInterruptionState {
    seen: Mutex<HashSet<String>>,
}

impl TranscriptInterruptionState {
    pub fn mark_if_new(&self, task_id: &str, marker_key: &str) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
        seen.insert(format!("{}:{}", task_id, marker_key))
    }
}

pub fn detect_unrecovered_interruption(path: &Path) -> io::Result<Option<InterruptionMarker>> {
    detect_unrecovered_interruption_with_limit(path, DEFAULT_TAIL_BYTES)
}

fn detect_unrecovered_interruption_with_limit(
    path: &Path,
    max_tail_bytes: u64,
) -> io::Result<Option<InterruptionMarker>> {
    let content = read_tail_to_string(path, max_tail_bytes)?;
    Ok(detect_unrecovered_interruption_in_lines(content.lines()))
}

fn read_tail_to_string(path: &Path, max_tail_bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_tail_bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut content = String::new();
    file.read_to_string(&mut content)?;

    if start > 0 {
        if let Some((_, rest)) = content.split_once('\n') {
            return Ok(rest.to_string());
        }
        return Ok(content);
    }
    Ok(content)
}

fn detect_unrecovered_interruption_in_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<InterruptionMarker> {
    let mut candidate: Option<InterruptionMarker> = None;

    for (index, line) in lines.into_iter().enumerate() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if is_interrupt_marker(&value) {
            candidate = Some(InterruptionMarker {
                key: marker_key(&value, index),
            });
            continue;
        }

        if candidate.is_some() && is_recovery_activity(&value) {
            candidate = None;
        }
    }

    candidate
}

fn is_interrupt_marker(value: &Value) -> bool {
    text_content(value)
        .as_deref()
        .is_some_and(|text| text.trim() == CLAUDE_INTERRUPT_MARKER)
}

fn is_recovery_activity(value: &Value) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("assistant") => true,
        Some("user") => text_content(value)
            .as_deref()
            .is_some_and(|text| text.trim() != CLAUDE_INTERRUPT_MARKER),
        _ => false,
    }
}

fn text_content(value: &Value) -> Option<String> {
    let content = value.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    let parts = content.as_array()?.iter().filter_map(|part| {
        let obj = part.as_object()?;
        match obj.get("type").and_then(Value::as_str) {
            Some("text") => obj.get("text").and_then(Value::as_str).map(str::to_string),
            Some("tool_result") => obj
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
    });

    let joined = parts.collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn marker_key(value: &Value, index: usize) -> String {
    value
        .get("uuid")
        .and_then(Value::as_str)
        .or_else(|| value.get("timestamp").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| format!("line-{}", index))
}

fn eligible_session(session: &Session) -> bool {
    session.status == SessionStatus::Running
        && session
            .source
            .as_deref()
            .is_some_and(|source| source.eq_ignore_ascii_case("claude-code"))
        && session
            .transcript_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
}

fn interruption_notify_request(session: &Session) -> NotifyRequest {
    NotifyRequest {
        title: session.title.clone(),
        message: CLAUDE_INTERRUPTION_PENDING_MESSAGE.into(),
        task_id: session.task_id.clone(),
        protocol_version: Some(1), // W9:wire 协议版本(重发路径)
        external_session_id: session.external_session_id.clone(),
        source: session.source.clone(),
        priority: None,
        event_type: Some("pending".into()),
        status: Some("pending".into()),
        terminal_tty: session.terminal_tty.clone(),
        workspace_path: session.workspace_path.clone(),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: session.transcript_path.clone(),
        // W6(brief 20260813-surface-readers):内层绑定从 surface 读(重发 wire
        // 字段,process_notify 会幂等重放);herdr 走共存特例 helper。
        herdr_pane_id: crate::surface::SurfaceBinding::herdr_fields(session)
            .map(|(pane, _, _)| pane.to_string()),
        herdr_workspace_id: crate::surface::SurfaceBinding::herdr_fields(session)
            .and_then(|(_, ws, _)| ws.map(str::to_string)),
        herdr_socket_path: crate::surface::SurfaceBinding::herdr_fields(session)
            .and_then(|(_, _, sock)| sock.map(str::to_string)),
        cmux_surface_id: match &session.surface {
            crate::surface::SurfaceBinding::Cmux { surface_id, .. } => Some(surface_id.clone()),
            _ => None,
        },
        cmux_workspace_id: match &session.surface {
            crate::surface::SurfaceBinding::Cmux { workspace_id, .. } => workspace_id.clone(),
            _ => None,
        },
        wezterm_pane_id: match &session.surface {
            crate::surface::SurfaceBinding::Wezterm { pane_id, .. } => Some(pane_id.clone()),
            _ => None,
        },
        wezterm_unix_socket: match &session.surface {
            crate::surface::SurfaceBinding::Wezterm { unix_socket, .. } => unix_socket.clone(),
            _ => None,
        },
        wezterm_executable: match &session.surface {
            crate::surface::SurfaceBinding::Wezterm { executable, .. } => executable.clone(),
            _ => None,
        },
        otty_pane_id: match &session.surface {
            crate::surface::SurfaceBinding::Otty { pane_id, .. } => Some(pane_id.clone()),
            _ => None,
        },
        tmux_pane_id: match &session.surface {
            crate::surface::SurfaceBinding::Tmux { pane_id, .. } => Some(pane_id.clone()),
            _ => None,
        },
        tmux_socket_path: match &session.surface {
            crate::surface::SurfaceBinding::Tmux { socket_path, .. } => socket_path.clone(),
            _ => None,
        },
        orca_pane_key: crate::surface::SurfaceBinding::orca_fields(session)
            .map(|(pk, _, _)| pk.to_string()),
        orca_worktree_id: crate::surface::SurfaceBinding::orca_fields(session)
            .and_then(|(_, wt, _)| wt.map(str::to_string)),
        orca_tab_id: crate::surface::SurfaceBinding::orca_fields(session)
            .and_then(|(_, _, tab)| tab.map(str::to_string)),
    }
}

pub fn scan_once(state: &AppState, seen: &TranscriptInterruptionState) -> usize {
    // Snapshot sessions while holding the store lock briefly.
    let sessions = {
        let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.get_all().to_vec()
    };

    let mut detected = 0;

    for session in sessions.iter().filter(|s| eligible_session(s)) {
        let Some(path) = session.transcript_path.as_deref() else {
            continue;
        };

        match detect_unrecovered_interruption(Path::new(path)) {
            Ok(Some(marker)) if seen.mark_if_new(&session.task_id, &marker.key) => {
                log_warn!(
                    "transcript_watch",
                    "Claude transcript interruption marker detected task_id={} marker_key={} path={}",
                    session.task_id,
                    marker.key,
                    path
                );
                // process_notify acquires store/popup-related locks internally;
                // scan_once must NOT hold the store lock across this call.
                process_notify(state, interruption_notify_request(session));
                detected += 1;
            }
            Ok(Some(marker)) => {
                log_debug!(
                    "transcript_watch",
                    "interrupt marker already handled task_id={} marker_key={}",
                    session.task_id,
                    marker.key
                );
            }
            Ok(None) => {}
            Err(e) => {
                log_debug!(
                    "transcript_watch",
                    "transcript scan skipped task_id={} path={} err={}",
                    session.task_id,
                    path,
                    e
                );
            }
        }
    }

    detected
}

pub fn spawn_transcript_interruption_watcher(state: AppState) {
    let seen = TranscriptInterruptionState::default();
    crate::supervisor::spawn_periodic("transcript-watch", WATCH_INTERVAL, move || {
        // scan_once 内含 process_notify(建 popup / 走锁),并发下任一 panic
        // 若直接冒出会静默杀死 watcher 线程(interruption 提醒永久丢失)。
        // 原内联 catch_unwind 已由 supervisor::spawn_periodic 统一兜住:
        // 记 ERROR 后下一 tick 继续,watcher 永不退出。
        let _ = scan_once(&state, &seen);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::{Priority, Session};
    use chrono::Utc;
    use std::io::Write;

    fn write_temp_jsonl(lines: &[&str]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("NamedTempFile 创建失败");
        for line in lines {
            writeln!(file, "{}", line).expect("writeln 失败");
        }
        file
    }

    #[test]
    fn detects_unrecovered_interrupt_marker() {
        let file = write_temp_jsonl(&[
            r#"{"type":"assistant","uuid":"a1","message":{"role":"assistant","content":[{"type":"text","text":"working"}]}}"#,
            r#"{"type":"user","uuid":"u-interrupt","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"system","uuid":"s1"}"#,
        ]);

        let marker = detect_unrecovered_interruption(file.path()).expect("中断标记检测应成功");
        assert_eq!(
            marker,
            Some(InterruptionMarker {
                key: "u-interrupt".into()
            })
        );
    }

    #[test]
    fn ignores_interrupt_when_user_continues_after_marker() {
        let file = write_temp_jsonl(&[
            r#"{"type":"user","uuid":"u-interrupt","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"user","uuid":"u-continue","message":{"role":"user","content":"继续"}}"#,
        ]);

        let marker = detect_unrecovered_interruption(file.path()).expect("中断标记检测应成功");
        assert_eq!(marker, None);
    }

    #[test]
    fn ignores_interrupt_when_assistant_recovers_after_marker() {
        let file = write_temp_jsonl(&[
            r#"{"type":"user","uuid":"u-interrupt","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"assistant","uuid":"a-recover","message":{"role":"assistant","content":[{"type":"text","text":"Recovered."}]}}"#,
        ]);

        let marker = detect_unrecovered_interruption(file.path()).expect("中断标记检测应成功");
        assert_eq!(marker, None);
    }

    #[test]
    fn skips_malformed_json_lines() {
        let file = write_temp_jsonl(&[
            "not json",
            r#"{"type":"user","uuid":"u-interrupt","message":{"role":"user","content":"[Request interrupted by user]"}}"#,
        ]);

        let marker = detect_unrecovered_interruption(file.path()).expect("中断标记检测应成功");
        assert_eq!(
            marker,
            Some(InterruptionMarker {
                key: "u-interrupt".into()
            })
        );
    }

    #[test]
    fn mark_if_new_dedupes_by_task_and_marker_key() {
        let state = TranscriptInterruptionState::default();
        assert!(state.mark_if_new("cc-abc", "u1"));
        assert!(!state.mark_if_new("cc-abc", "u1"));
        assert!(state.mark_if_new("cc-abc", "u2"));
        assert!(state.mark_if_new("cc-def", "u1"));
    }

    #[test]
    fn read_tail_preserves_partial_content_when_no_newline_in_tail() {
        let mut file = tempfile::NamedTempFile::new().expect("NamedTempFile 创建失败");
        // Write content longer than what we'll read in the tail
        write!(file, "0123456789abcdef").expect("write 失败"); // 16 bytes
        file.flush().expect("write 失败");

        // Read only last 10 bytes (which won't contain any newline)
        let content = read_tail_to_string(file.path(), 10).expect("read_tail_to_string 应成功");
        assert_eq!(content, "6789abcdef");
    }

    #[test]
    fn detect_interruption_works_with_partial_tail_no_newline() {
        let mut file = tempfile::NamedTempFile::new().expect("NamedTempFile 创建失败");
        // Write a complete line first, then a partial line without newline
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","message":{{"role":"user","content":"hello"}}}}"#
        )
        .expect("writeln 应成功");
        // Write the interrupt marker WITHOUT a trailing newline
        write!(
            file,
            r#"{{"type":"user","uuid":"u-interrupt","message":{{"role":"user","content":[{{"type":"text","text":"[Request interrupted by user]"}}]}}}}"#
        )
        .expect("write 应成功");
        file.flush().expect("flush 失败");

        // Limit to just enough bytes to include the interrupt line but split before any newline
        // The interrupt line is ~130 bytes, so 200 should capture it without the first line
        let marker = detect_unrecovered_interruption_with_limit(file.path(), 200)
            .expect("中断标记检测(limit)应成功");
        assert_eq!(
            marker,
            Some(InterruptionMarker {
                key: "u-interrupt".into()
            })
        );
    }

    fn session(
        status: SessionStatus,
        source: Option<&str>,
        transcript_path: Option<&str>,
    ) -> Session {
        Session {
            id: "session-id".into(),
            task_id: "cc-abc".into(),
            external_session_id: Some("external".into()),
            title: "project".into(),
            message: "Working".into(),
            source: source.map(str::to_string),
            priority: Priority::Normal,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            terminal_tty: Some("/dev/ttys001".into()),
            workspace_path: Some("/tmp/project".into()),
            failure_reason: None,
            aborted_at: None,
            terminal_id: None,
            terminal_id_captured_cwd: None,
            transcript_path: transcript_path.map(str::to_string),
            herdr_pane_id: None,
            herdr_workspace_id: None,
            herdr_socket_path: None,
            cmux_surface_id: None,
            cmux_workspace_id: None,
            wezterm_pane_id: None,
            wezterm_unix_socket: None,
            wezterm_executable: None,
            otty_pane_id: None,
            otty_tab_id: None,
            otty_window_id: None,
            tmux_pane_id: None,
            tmux_socket_path: None,
            orca_pane_key: None,
            orca_worktree_id: None,
            orca_tab_id: None,
            terminal_app: None,
            dsh_nav_socket: None,
            surface: Default::default(), // W5:内层绑定真源,见 brief 20260813-surface-binding
        }
    }

    #[test]
    fn eligibility_requires_running_claude_code_with_transcript_path() {
        assert!(eligible_session(&session(
            SessionStatus::Running,
            Some("claude-code"),
            Some("/tmp/t.jsonl")
        )));
        assert!(!eligible_session(&session(
            SessionStatus::Pending,
            Some("claude-code"),
            Some("/tmp/t.jsonl")
        )));
        assert!(!eligible_session(&session(
            SessionStatus::Running,
            Some("codex"),
            Some("/tmp/t.jsonl")
        )));
        assert!(!eligible_session(&session(
            SessionStatus::Running,
            Some("claude-code"),
            None
        )));
    }

    #[test]
    fn interruption_notify_request_uses_neutral_pending_message() {
        let s = session(
            SessionStatus::Running,
            Some("claude-code"),
            Some("/tmp/t.jsonl"),
        );

        let req = interruption_notify_request(&s);
        assert_eq!(req.event_type.as_deref(), Some("pending"));
        assert_eq!(req.status.as_deref(), Some("pending"));
        assert_eq!(req.message, CLAUDE_INTERRUPTION_PENDING_MESSAGE);
        assert!(!req.message.to_lowercase().contains("user interrupted"));
        assert_eq!(req.task_id, "cc-abc");
        assert_eq!(req.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    }
}
