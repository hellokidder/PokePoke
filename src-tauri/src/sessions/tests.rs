use super::{Priority, SessionStatus, SessionStore, UpsertRequest};
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

pub(super) fn temp_store() -> (TempDir, SessionStore, PathBuf) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    let store = SessionStore::load(path.clone());
    (dir, store, path)
}

pub(super) fn insert_running_session(store: &mut SessionStore) {
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: Some("session-abc".into()),
        title: "Claude Code: poke-poke".into(),
        message: "Working...".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        failure_reason: Some("ignored".into()),
        ..Default::default()
    });
    assert!(result.is_new);
}

pub(super) fn insert_running_session_with_id(store: &mut SessionStore, task_id: &str) {
    let result = store.upsert_session(UpsertRequest {
        task_id: task_id.into(),
        external_session_id: None,
        title: "t".into(),
        message: "m".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: None,
        workspace_path: None,
        ..Default::default()
    });
    assert!(result.is_new);
}

#[test]

fn upsert_new_session_inserts_record_and_ignores_failure_reason_when_not_failed() {
    let (_dir, mut store, _path) = temp_store();
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: Some("session-abc".into()),
        title: "Claude Code: poke-poke".into(),
        message: "Working...".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        failure_reason: Some("rate_limit".into()),
        transcript_path: Some("/tmp/transcript.jsonl".into()),
        ..Default::default()
    });
    assert!(result.is_new);
    assert_eq!(result.prev_status, None);
    assert_eq!(store.sessions.len(), 1);
    assert_eq!(
        store.sessions[0].external_session_id.as_deref(),
        Some("session-abc"),
    );
    assert_eq!(store.sessions[0].status, SessionStatus::Running);
    assert_eq!(store.sessions[0].failure_reason, None);
    assert_eq!(
        store.sessions[0].transcript_path.as_deref(),
        Some("/tmp/transcript.jsonl"),
        "新 session 应记下 transcript_path 供 transcript watcher 等后续使用"
    );
}

#[test]

fn upsert_existing_session_updates_in_place_and_preserves_existing_metadata_when_new_value_is_none()
{
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "Claude Code: renamed".into(),
        message: "Session completed".into(),
        priority: Priority::High,
        status: SessionStatus::Idle,
        failure_reason: Some("ignored".into()),
        ..Default::default()
    });
    assert!(!result.is_new);
    assert_eq!(result.prev_status, Some(SessionStatus::Running));
    assert_eq!(store.sessions.len(), 1);
    assert_eq!(store.sessions[0].title, "Claude Code: renamed");
    assert_eq!(store.sessions[0].message, "Session completed");
    assert_eq!(store.sessions[0].source.as_deref(), Some("claude-code"));
    assert_eq!(
        store.sessions[0].external_session_id.as_deref(),
        Some("session-abc"),
    );
    assert_eq!(store.sessions[0].priority, Priority::High);
    assert_eq!(store.sessions[0].status, SessionStatus::Idle);
    assert_eq!(
        store.sessions[0].terminal_tty.as_deref(),
        Some("/dev/ttys001")
    );
    assert_eq!(
        store.sessions[0].workspace_path.as_deref(),
        Some("/tmp/poke-poke")
    );
    assert_eq!(store.sessions[0].failure_reason, None);
}

#[test]

fn upsert_existing_session_refreshes_transcript_path_when_some_and_preserves_when_none() {
    let (_dir, mut store, _path) = temp_store();
    // 初始用 insert_running_session(transcript_path=None)
    insert_running_session(&mut store);
    assert_eq!(store.sessions[0].transcript_path, None);
    // Some(_) → 覆盖
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "m".into(),
        status: SessionStatus::Running,
        transcript_path: Some("/tmp/first.jsonl".into()),
        ..Default::default()
    });
    assert_eq!(
        store.sessions[0].transcript_path.as_deref(),
        Some("/tmp/first.jsonl")
    );
    // None → 保留旧值(后续事件可能不带这个字段,不该把已知路径清掉)
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "m".into(),
        status: SessionStatus::Running,
        ..Default::default()
    });
    assert_eq!(
        store.sessions[0].transcript_path.as_deref(),
        Some("/tmp/first.jsonl")
    );
    // Some("") → 同样不覆盖(空串归一化到 None)
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "m".into(),
        status: SessionStatus::Running,
        transcript_path: Some(String::new()),
        ..Default::default()
    });
    assert_eq!(
        store.sessions[0].transcript_path.as_deref(),
        Some("/tmp/first.jsonl")
    );
}

#[test]

fn upsert_last_failed_stores_reason_and_clears_it_when_session_returns_to_running() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let failed = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: Some("session-abc".into()),
        title: "Claude Code: poke-poke".into(),
        message: "".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::LastFailed,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        failure_reason: Some("rate_limit".into()),
        ..Default::default()
    });
    assert_eq!(failed.prev_status, Some(SessionStatus::Running));
    assert_eq!(
        store.sessions[0].failure_reason.as_deref(),
        Some("rate_limit")
    );
    let running = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "Claude Code: poke-poke".into(),
        message: "Working again".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        failure_reason: Some("server_error".into()),
        ..Default::default()
    });
    assert_eq!(running.prev_status, Some(SessionStatus::LastFailed));
    assert_eq!(store.sessions[0].failure_reason, None);
}
// T1(brief 20260805-t1-cursor-aborted)四连:
// aborted 置 idle+aborted_at / 窗口内 echo last_failed 不覆盖 /
// Running 清窗口 / 窗口外 last_failed 正常覆盖(直接改 aborted_at
// 模拟时间流逝,ABORTED_ECHO_WINDOW 是 const 不可在测试内调小)。

#[test]

fn upsert_aborted_sets_idle_and_aborted_at() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "Claude Code: poke-poke".into(),
        message: "Agent turn aborted".into(),
        source: Some("cursor".into()),
        status: SessionStatus::Idle,
        aborted: Some(true),
        ..Default::default()
    });
    assert_eq!(result.prev_status, Some(SessionStatus::Running));
    assert_eq!(store.sessions[0].status, SessionStatus::Idle);
    assert!(
        store.sessions[0].aborted_at.is_some(),
        "aborted 事件应写 aborted_at"
    );
    assert!(
        store.sessions[0].is_recently_aborted(),
        "刚取消应在 ABORTED_ECHO_WINDOW 内"
    );
}

#[test]

fn upsert_last_failed_echo_within_aborted_window_keeps_idle() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    // 先发 aborted → idle + aborted_at=now
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Agent turn aborted".into(),
        status: SessionStatus::Idle,
        aborted: Some(true),
        ..Default::default()
    });
    // 40ms 后第二条(hookStatus 缺失/error)→ last_failed,应被当作 echo
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Agent turn ended with error".into(),
        status: SessionStatus::LastFailed,
        failure_reason: Some("api_error".into()),
        ..Default::default()
    });
    assert_eq!(result.prev_status, Some(SessionStatus::Idle));
    assert_eq!(
        store.sessions[0].status,
        SessionStatus::Idle,
        "echo 不应把 idle 覆盖成 last_failed"
    );
    assert_eq!(
        store.sessions[0].failure_reason, None,
        "echo 不写 failure_reason"
    );
    assert!(
        store.sessions[0].aborted_at.is_some(),
        "echo 不清 aborted_at(窗口保持到新一轮)"
    );
}

#[test]

fn upsert_running_clears_aborted_at() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Agent turn aborted".into(),
        status: SessionStatus::Idle,
        aborted: Some(true),
        ..Default::default()
    });
    assert!(store.sessions[0].aborted_at.is_some());
    // 新一轮开始 → 清窗口
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Working again".into(),
        status: SessionStatus::Running,
        ..Default::default()
    });
    assert_eq!(store.sessions[0].aborted_at, None);
}

#[test]

fn upsert_last_failed_outside_aborted_window_overrides() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Agent turn aborted".into(),
        status: SessionStatus::Idle,
        aborted: Some(true),
        ..Default::default()
    });
    // 直接改 aborted_at 到窗口外(1 分钟前),模拟真实时间流逝
    store.sessions[0].aborted_at = Some(Utc::now() - chrono::Duration::minutes(1));
    assert!(!store.sessions[0].is_recently_aborted());
    let result = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        title: "t".into(),
        message: "Agent turn ended with error".into(),
        status: SessionStatus::LastFailed,
        failure_reason: Some("api_error".into()),
        ..Default::default()
    });
    assert_eq!(result.prev_status, Some(SessionStatus::Idle));
    assert_eq!(
        store.sessions[0].status,
        SessionStatus::LastFailed,
        "窗口外应正常覆盖"
    );
    assert_eq!(
        store.sessions[0].failure_reason.as_deref(),
        Some("api_error")
    );
}

#[test]

fn save_and_load_round_trip_preserves_session_fields() {
    let (_dir, mut store, path) = temp_store();
    store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: Some("session-abc".into()),
        title: "Claude Code: poke-poke".into(),
        message: "".into(),
        source: Some("claude-code".into()),
        priority: Priority::High,
        status: SessionStatus::LastFailed,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        failure_reason: Some("authentication_failed".into()),
        transcript_path: Some("/tmp/cc.jsonl".into()),
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
    });
    let reloaded = SessionStore::load(path);
    assert_eq!(reloaded.sessions.len(), 1);
    let session = &reloaded.sessions[0];
    assert_eq!(session.task_id, "cc-123");
    assert_eq!(session.external_session_id.as_deref(), Some("session-abc"));
    assert_eq!(session.priority, Priority::High);
    assert_eq!(session.status, SessionStatus::LastFailed);
    assert_eq!(
        session.failure_reason.as_deref(),
        Some("authentication_failed")
    );
    assert_eq!(session.source.as_deref(), Some("claude-code"));
    assert_eq!(session.terminal_tty.as_deref(), Some("/dev/ttys001"));
    assert_eq!(session.workspace_path.as_deref(), Some("/tmp/poke-poke"));
    assert_eq!(session.transcript_path.as_deref(), Some("/tmp/cc.jsonl"));
}

#[test]

fn load_returns_empty_when_file_is_missing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.json");
    let store = SessionStore::load(path);
    assert!(store.sessions.is_empty());
}

#[test]

fn load_returns_empty_when_file_is_damaged() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    std::fs::write(&path, "{not valid json").unwrap();
    let store = SessionStore::load(path);
    assert!(store.sessions.is_empty());
}

#[test]

fn load_supports_legacy_success_and_failure_status_aliases() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    let now = Utc::now().to_rfc3339();
    let data = json!([
        {
            "id": "session-1",
            "task_id": "cc-123",
            "external_session_id": "cc-session-123",
            "title": "Claude Code: poke-poke",
            "message": "done",
            "source": "claude-code",
            "priority": "normal",
            "status": "success",
            "created_at": now,
            "updated_at": now,
            "terminal_tty": "/dev/ttys001",
            "workspace_path": "/tmp/poke-poke",
            "failure_reason": null
        },
        {
            "id": "session-2",
            "task_id": "cc-456",
            "external_session_id": "cc-session-456",
            "title": "Claude Code: poke-poke",
            "message": "failed",
            "source": "claude-code",
            "priority": "high",
            "status": "failure",
            "created_at": now,
            "updated_at": now,
            "terminal_tty": "/dev/ttys002",
            "workspace_path": "/tmp/poke-poke",
            "failure_reason": "rate_limit"
        }
    ]);
    std::fs::write(&path, data.to_string()).unwrap();
    let store = SessionStore::load(path);
    assert_eq!(store.sessions.len(), 2);
    assert_eq!(store.sessions[0].status, SessionStatus::Idle);
    assert_eq!(
        store.sessions[0].external_session_id.as_deref(),
        Some("cc-session-123"),
    );
    assert_eq!(store.sessions[1].status, SessionStatus::LastFailed);
    assert_eq!(
        store.sessions[1].failure_reason.as_deref(),
        Some("rate_limit")
    );
}

#[test]

fn remove_session_by_task_id_returns_removed_session_and_updates_store() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let removed = store.remove_session_by_task_id("cc-123").unwrap();
    assert_eq!(removed.task_id, "cc-123");
    assert!(store.sessions.is_empty());
}
// 2026-05-25 加固:focus_ghostty 发现 stored id stale 时反向清掉,
// 后续 click 走 L2 cwd 兜底,而不是稳定跳错。

#[test]

fn clear_terminal_id_no_op_when_session_missing() {
    let (_dir, mut store, _path) = temp_store();
    assert!(!store.clear_terminal_id_if_matches("no-such-session", "id-x"));
}

#[test]

fn clear_terminal_id_no_op_when_currently_none() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    // upsert 默认 terminal_id=None,这一调用应被忽略,不变 None
    assert!(!store.clear_terminal_id_if_matches(&sid, "id-x"));
    assert_eq!(store.sessions[0].terminal_id, None);
}

#[test]

fn clear_terminal_id_no_op_when_value_does_not_match() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_terminal_id_if_unset("cc-123", "id-a".into(), "/tmp/poke-poke".into());
    assert_eq!(store.sessions[0].terminal_id.as_deref(), Some("id-a"));
    assert_eq!(
        store.sessions[0].terminal_id_captured_cwd.as_deref(),
        Some("/tmp/poke-poke")
    );
    // expected 跟当前不等 → 拒清,防 race(中间被改过)
    assert!(!store.clear_terminal_id_if_matches(&sid, "id-different"));
    assert_eq!(store.sessions[0].terminal_id.as_deref(), Some("id-a"));
}

#[test]

fn clear_terminal_id_clears_when_value_matches() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let sid = store.sessions[0].id.clone();
    store.set_terminal_id_if_unset("cc-123", "id-stale".into(), "/tmp/poke-poke".into());
    assert!(store.clear_terminal_id_if_matches(&sid, "id-stale"));
    assert_eq!(store.sessions[0].terminal_id, None);
    assert_eq!(store.sessions[0].terminal_id_captured_cwd, None);
    // 清空后再清是 no-op
    assert!(!store.clear_terminal_id_if_matches(&sid, "id-stale"));
}

#[test]

fn reconcile_by_title_overwrites_misbound_id() {
    // 错绑纠正:cwd first-match 先把 session 绑到邻居 tab(wrong-uuid),
    // title 对账命中后必须能覆盖成正确的 right-uuid。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_terminal_id_if_unset("cc-123", "wrong-uuid".into(), "/Users/edy".into());
    assert_eq!(store.sessions[0].terminal_id.as_deref(), Some("wrong-uuid"));
    let changed = store.reconcile_terminal_id(
        "cc-123",
        "right-uuid".into(),
        "/Users/edy".into(),
        true, // by_title
    );
    assert!(changed);
    assert_eq!(store.sessions[0].terminal_id.as_deref(), Some("right-uuid"));
    assert_eq!(
        store.sessions[0].terminal_id_captured_cwd.as_deref(),
        Some("/Users/edy")
    );
}

#[test]

fn reconcile_by_cwd_does_not_overwrite_existing() {
    // 弱证据(cwd 兜底)不得覆盖已有绑定——cwd 本就可能撞车。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_terminal_id_if_unset("cc-123", "existing-uuid".into(), "/Users/edy".into());
    let changed = store.reconcile_terminal_id(
        "cc-123",
        "cwd-guess-uuid".into(),
        "/Users/edy".into(),
        false, // by_cwd
    );
    assert!(!changed);
    assert_eq!(
        store.sessions[0].terminal_id.as_deref(),
        Some("existing-uuid")
    );
}

#[test]

fn reconcile_by_cwd_writes_when_unset() {
    // 弱证据在空位上可以写。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    let changed =
        store.reconcile_terminal_id("cc-123", "cwd-uuid".into(), "/Users/edy".into(), false);
    assert!(changed);
    assert_eq!(store.sessions[0].terminal_id.as_deref(), Some("cwd-uuid"));
}

#[test]

fn reconcile_same_value_is_noop() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.set_terminal_id_if_unset("cc-123", "same-uuid".into(), "/Users/edy".into());
    // 同值即便 by_title 也不算改动(幂等省写)。
    assert!(!store.reconcile_terminal_id("cc-123", "same-uuid".into(), "/Users/edy".into(), true));
}

#[test]

fn reconcile_refuses_duplicate_on_different_tty() {
    // 防撞保护:title 强证据也不能把一个 UUID 绑到两个不同 tty 的 session。
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store); // cc-123 @ ttys001
    store.upsert_session(UpsertRequest {
        task_id: "cc-456".into(),
        external_session_id: Some("session-def".into()),
        title: "other".into(),
        message: "Working...".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys002".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        ..Default::default()
    });
    store.set_terminal_id_if_unset("cc-123", "shared-uuid".into(), "/Users/edy".into());
    // cc-456 想用同一 UUID,但 tty 不同 → 拒写,即便 by_title。
    let changed =
        store.reconcile_terminal_id("cc-456", "shared-uuid".into(), "/Users/edy".into(), true);
    assert!(!changed);
    let other = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-456")
        .unwrap();
    assert_eq!(other.terminal_id, None);
}

#[test]

fn reconcile_missing_task_is_noop() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    assert!(!store.reconcile_terminal_id(
        "cc-nonexistent",
        "uuid".into(),
        "/Users/edy".into(),
        true
    ));
}

#[test]

fn set_terminal_id_rejects_duplicate_on_different_tty() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.upsert_session(UpsertRequest {
        task_id: "cc-456".into(),
        external_session_id: Some("session-def".into()),
        title: "Claude Code: other".into(),
        message: "Working...".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys002".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        ..Default::default()
    });
    assert!(store.set_terminal_id_if_unset("cc-123", "shared-id".into(), "/tmp/poke-poke".into()));
    assert!(!store.set_terminal_id_if_unset("cc-456", "shared-id".into(), "/tmp/poke-poke".into()));
    let other = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-456")
        .unwrap();
    assert_eq!(other.terminal_id, None);
    assert_eq!(other.terminal_id_captured_cwd, None);
}

#[test]

fn set_terminal_id_allows_duplicate_on_same_tty() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    store.upsert_session(UpsertRequest {
        task_id: "cc-456".into(),
        external_session_id: Some("session-def".into()),
        title: "Claude Code: same tty".into(),
        message: "Working...".into(),
        source: Some("claude-code".into()),
        status: SessionStatus::Running,
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/poke-poke".into()),
        ..Default::default()
    });
    assert!(store.set_terminal_id_if_unset("cc-123", "shared-id".into(), "/tmp/poke-poke".into()));
    assert!(store.set_terminal_id_if_unset("cc-456", "shared-id".into(), "/tmp/poke-poke".into()));
}
// ==================== herdr fields ====================

#[test]

fn session_deserializes_herdr_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    let now = Utc::now().to_rfc3339();
    let data = json!([{
        "id":"s-1","task_id":"cc-abc","external_session_id":"abcd1234-5678",
        "title":"t","message":"m","source":"claude-code","priority":"normal",
        "status":"running","created_at":now,"updated_at":now,
        "herdr_pane_id":"w3:p1","herdr_workspace_id":"w3",
        "herdr_socket_path":"/Users/me/.config/herdr/herdr.sock"
    }]);
    std::fs::write(&path, data.to_string()).unwrap();
    let store = SessionStore::load(path);
    let s = &store.sessions[0];
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w3:p1"));
    assert_eq!(s.herdr_workspace_id.as_deref(), Some("w3"));
    assert_eq!(
        s.herdr_socket_path.as_deref(),
        Some("/Users/me/.config/herdr/herdr.sock")
    );
}

#[test]

fn session_deserializes_without_herdr_fields_defaults_to_none() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    let now = Utc::now().to_rfc3339();
    let data = json!([{
        "id":"s-1","task_id":"cc-abc","title":"t","message":"m",
        "priority":"normal","status":"running",
        "created_at":now,"updated_at":now
    }]);
    std::fs::write(&path, data.to_string()).unwrap();
    let store = SessionStore::load(path);
    let s = &store.sessions[0];
    assert!(s.herdr_pane_id.is_none());
    assert!(s.herdr_workspace_id.is_none());
    assert!(s.herdr_socket_path.is_none());
}

#[test]

fn session_deserializes_without_warp_field_defaults_to_none() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sessions.json");
    let now = Utc::now().to_rfc3339();
    let data = json!([{
        "id":"s-1","task_id":"cc-abc","title":"t","message":"m",
        "priority":"normal","status":"running",
        "created_at":now,"updated_at":now
    }]);
    std::fs::write(&path, data.to_string()).unwrap();
    let store = SessionStore::load(path);
    assert!(store.sessions[0].terminal_app.is_none());
}

#[test]

fn terminal_app_only_overwrites_when_some() {
    let (_dir, mut store, _path) = temp_store();
    insert_running_session(&mut store);
    // 先写 warpterminal(与 SessionStart 上报一致)
    let r1 = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: Some("session-abc".into()),
        title: "t".into(),
        message: "m".into(),
        source: Some("claude-code".into()),
        priority: Priority::Normal,
        status: SessionStatus::Running,
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        transcript_path: None,
        aborted: None,
        terminal_app: Some("warpterminal".into()),
        dsh_nav_socket: None,
    });
    assert_eq!(r1.session.terminal_app.as_deref(), Some("warpterminal"));
    // 后续事件不带(旧 hook / env 识别失败)→ 不覆盖既有值
    let r2 = store.upsert_session(UpsertRequest {
        task_id: "cc-123".into(),
        external_session_id: None,
        title: "t".into(),
        message: "m2".into(),
        source: None,
        priority: Priority::Normal,
        status: SessionStatus::Running,
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        transcript_path: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
    });
    assert_eq!(r2.session.terminal_app.as_deref(), Some("warpterminal"));
}
