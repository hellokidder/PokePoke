//! http_server 单测:notify 解析 / dedupe / process_notify(binding 落库与
//! 编排)+ 测试基础设施(test_state / req_with / spy trait 实现)。
//! 从 `http_server.rs` 的 `mod tests` 拆出(brief 20260727-split-http-server-rs
//! 第二阶段),`super::*` 引用路径不变,测试内容零改动。

use super::notify::{
    dedupe_signature, parse_event_type, parse_status, process_notify, NotifyEventType,
    NotifyRequest,
};
use super::title::resolve_title_for_notify;
use super::{AppState, EventEmitter, NotificationGate};
use crate::popup::{FocusOracle, PopupController};
use crate::sessions::{Session, SessionStatus, SessionStore};
use crate::settings::SettingsStore;
use crate::sound::SoundPlayer;
use std::sync::{Arc, Mutex};

#[test]
fn parse_status_supports_case_insensitive_values_and_aliases() {
    assert_eq!(parse_status(Some("RUNNING"), None), SessionStatus::Running);
    assert_eq!(parse_status(Some("success"), None), SessionStatus::Idle);
    assert_eq!(
        parse_status(Some("FAILED"), None),
        SessionStatus::LastFailed
    );
    assert_eq!(
        parse_status(Some("last_failed"), None),
        SessionStatus::LastFailed
    );
}

#[test]
fn parse_status_defaults_to_pending_for_missing_or_unknown_values() {
    assert_eq!(parse_status(None, None), SessionStatus::Pending);
    assert_eq!(parse_status(Some("unknown"), None), SessionStatus::Pending);
}

#[test]
fn parse_event_type_supports_known_values_and_unknown_fallback() {
    assert_eq!(
        parse_event_type(Some("running")),
        Some(NotifyEventType::Running)
    );
    assert_eq!(
        parse_event_type(Some("PENDING")),
        Some(NotifyEventType::Pending)
    );
    assert_eq!(parse_event_type(Some("stop")), Some(NotifyEventType::Stop));
    assert_eq!(
        parse_event_type(Some("session_end")),
        Some(NotifyEventType::SessionEnd),
    );
    assert_eq!(parse_event_type(Some("other")), None);
}

#[test]
fn process_notify_ignores_disconnected_integration_source() {
    use axum::http::StatusCode;

    let state = test_state();
    {
        let mut s = state.settings.lock().unwrap_or_else(|p| p.into_inner());
        s.settings.disconnected_sources.push("cursor".into());
        s.save();
    }

    let mut req = req_with("cursor-abc", Some("sessionStart"), Some("running"));
    req.source = Some("cursor".into());

    let (code, body) = process_notify(&state, req);
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ignored"], true);
    assert_eq!(body["reason"], "integration_disconnected");
    assert!(state
        .store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_all()
        .is_empty());
}

#[test]
fn parse_status_can_fall_back_to_event_type_when_status_missing() {
    assert_eq!(
        parse_status(None, Some(&NotifyEventType::Running)),
        SessionStatus::Running,
    );
    assert_eq!(
        parse_status(None, Some(&NotifyEventType::Pending)),
        SessionStatus::Pending,
    );
    assert_eq!(
        parse_status(None, Some(&NotifyEventType::Stop)),
        SessionStatus::Idle,
    );
}

#[test]
fn explicit_status_has_priority_over_event_type_fallback() {
    assert_eq!(
        parse_status(Some("last_failed"), Some(&NotifyEventType::Stop)),
        SessionStatus::LastFailed,
    );
}

fn req_with(task_id: &str, event_type: Option<&str>, status: Option<&str>) -> NotifyRequest {
    NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: task_id.into(),
        external_session_id: None,
        source: None,
        priority: None,
        event_type: event_type.map(|s| s.into()),
        status: status.map(|s| s.into()),
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    }
}

#[test]
fn resolve_title_codex_cli_keeps_hook_title() {
    // codex CLI(有 tty)不走 thread_name 解析,保持 hook 上报的 title
    let mut req = req_with("t1", Some("SessionStart"), Some("running"));
    req.source = Some("codex".into());
    req.terminal_tty = Some("/dev/ttys001".into());
    req.title = "myproj".into();
    assert_eq!(resolve_title_for_notify(&req), "myproj");
}

#[test]
fn resolve_title_codex_desktop_falls_back_when_thread_not_found() {
    // codex 桌面端(无 tty)+ 不存在的 thread id -> 兜底 hook title。
    // (真实 thread_name 命中依赖 ~/.codex/session_index.jsonl,难单测,这里测兜底)
    let mut req = req_with("t1", Some("SessionStart"), Some("running"));
    req.source = Some("codex".into());
    req.terminal_tty = None;
    req.external_session_id = Some("00000000-0000-0000-0000-000000000000".into());
    req.title = "fallback-dir".into();
    assert_eq!(resolve_title_for_notify(&req), "fallback-dir");
}

#[test]
fn dedupe_signature_combines_event_type_and_status() {
    // review M1:同 event_type 不同 status(stop/idle vs stop/last_failed)不再
    // 得到同 signature,避免 100ms 内先 Stop 后 StopFailure 时 API 失败被吞。
    let r = req_with("t1", Some("Stop"), Some("idle"));
    assert_eq!(dedupe_signature(&r), "stop|idle");
    let r2 = req_with("t1", Some("Stop"), Some("last_failed"));
    assert_eq!(dedupe_signature(&r2), "stop|last_failed");
    assert_ne!(dedupe_signature(&r), dedupe_signature(&r2));
}

#[test]
fn dedupe_signature_event_type_only_when_no_status() {
    // 只有 event_type、没 status:signature 就是 event_type(向后兼容原去重)
    let r = req_with("t1", Some("Stop"), None);
    assert_eq!(dedupe_signature(&r), "stop");
}

#[test]
fn dedupe_signature_falls_back_to_status_when_event_type_missing() {
    let r = req_with("t1", None, Some("LAST_FAILED"));
    assert_eq!(dedupe_signature(&r), "last_failed");
}

#[test]
fn dedupe_signature_returns_default_when_both_missing() {
    let r = req_with("t1", None, None);
    assert_eq!(dedupe_signature(&r), "default");
}

// ==================== NotifyRequest herdr fields ====================

#[test]
fn notify_request_deserializes_herdr_fields() {
    let raw = r#"{
            "title":"t","message":"m","task_id":"cc-abc",
            "source":"claude-code",
            "herdr_pane_id":"w3:p1",
            "herdr_workspace_id":"w3",
            "herdr_socket_path":"/tmp/herdr.sock"
        }"#;
    let req: super::NotifyRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(req.herdr_pane_id.as_deref(), Some("w3:p1"));
    assert_eq!(req.herdr_workspace_id.as_deref(), Some("w3"));
    assert_eq!(req.herdr_socket_path.as_deref(), Some("/tmp/herdr.sock"));
}

#[test]
fn notify_request_without_herdr_fields_defaults_to_none() {
    let raw = r#"{"title":"t","message":"m","task_id":"cc-abc","source":"claude-code"}"#;
    let req: super::NotifyRequest = serde_json::from_str(raw).unwrap();
    assert!(req.herdr_pane_id.is_none());
}

// ==================== process_notify herdr tests ====================

// Test implementations of required traits
pub(super) struct TestPopupController;
impl PopupController for TestPopupController {
    fn show(&self, _session: &Session) -> bool {
        false
    }
    fn close(&self, _session_id: &str, _reason: crate::popup::PopupCloseReason) {}
    fn has_session_popup(&self, _session_id: &str) -> bool {
        false
    }
}

pub(super) struct TestSoundPlayer;
impl SoundPlayer for TestSoundPlayer {
    fn play_alert(&self, _status: crate::sessions::SessionStatus) {}
}

pub(super) struct TestFocusOracle;
impl FocusOracle for TestFocusOracle {
    // 既有测试全部走 running → Skip,从不到达 already_focused gate;
    // 决策 trace 测试用 Stop→Idle 会走到这里 —— 两个方法都有默认实现且互相
    // 委托,不覆盖会无限递归,这里显式返回 false(未聚焦)。
    fn is_focused(&self, _evidence: &crate::surface::SurfaceEvidence) -> bool {
        false
    }
}

/// 恒返回「已聚焦」的 stub —— 让 /notify 走 SuppressWithSound(抑制弹窗、
/// 保留声音)分支,用于验证重复 pending 静默(brief 20260819-suppress-with-sound-repeat-pending)。
pub(super) struct FocusedOracle;
impl FocusOracle for FocusedOracle {
    fn is_focused(&self, _evidence: &crate::surface::SurfaceEvidence) -> bool {
        true
    }
}

/// 测试用事件发射器：只关心 `sessions-updated`（空实现即可）。
pub(super) struct TestEventEmitter;

impl TestEventEmitter {
    pub(super) fn new() -> Self {
        Self
    }
}

impl EventEmitter for TestEventEmitter {
    fn sessions_updated(&self) {}
}

pub(super) struct TestNotificationGate;
impl NotificationGate for TestNotificationGate {
    fn allow_notifications(&self) -> bool {
        true
    }
}

fn test_state() -> AppState {
    test_state_with(Arc::new(TestPopupController))
}

/// 可指定 popup stub 的 AppState builder（原先带 license 初始状态参数，
/// 2026-11 移除付费墙后 license 维度消失）。
pub(super) fn test_state_with(popup: Arc<dyn PopupController>) -> AppState {
    test_state_with_events(popup, Arc::new(TestEventEmitter::new()))
}

pub(super) fn test_state_with_events(
    popup: Arc<dyn PopupController>,
    events: Arc<TestEventEmitter>,
) -> AppState {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();

    let store = Arc::new(Mutex::new(SessionStore::load(
        dir.path().join("sessions.json"),
    )));
    let settings = Arc::new(Mutex::new(SettingsStore::load(
        dir.path().join("settings.json"),
    )));

    AppState {
        store,
        popup,
        sound: Arc::new(TestSoundPlayer),
        focus: Arc::new(TestFocusOracle),
        events,
        gate: Arc::new(TestNotificationGate),
        memory: Arc::new(crate::http_server::SessionMemory::default()),
        auto_approval: Arc::new(crate::codex_rollout::AutoApprovalCache::default()),
        codex_thread_status: Arc::new(crate::codex_appserver::CodexThreadStatusCache::default()),
        codex_appserver: crate::codex_appserver::CodexAppServerHandle::default(),
        settings,
        quiet_mode: None,
        staging: crate::http_server::StagingDispatcher::inline(),
    }
}

#[test]
fn process_notify_persists_herdr_binding_when_all_three_env_vars_present() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-xyz".into(),
        external_session_id: Some("xyz01234-abcd".into()),
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: Some("w3:p1".into()),
        herdr_workspace_id: Some("w3".into()),
        herdr_socket_path: Some("/tmp/herdr.sock".into()),
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-xyz")
        .unwrap();
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w3:p1"));
    assert_eq!(s.herdr_workspace_id.as_deref(), Some("w3"));
    assert_eq!(s.herdr_socket_path.as_deref(), Some("/tmp/herdr.sock"));
}

#[test]
fn process_notify_herdr_binding_skips_cmux_in_same_request() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-herdr-cmux".into(),
        external_session_id: Some("xyz99999-abcd".into()),
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: Some("w1:p1".into()),
        herdr_workspace_id: Some("w1".into()),
        herdr_socket_path: Some("/tmp/work/herdr.sock".into()),
        cmux_surface_id: Some("SURF-STALE".into()),
        cmux_workspace_id: Some("WS-STALE".into()),
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-herdr-cmux")
        .unwrap();
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w1:p1"));
    assert!(s.cmux_surface_id.is_none());
    assert!(s.cmux_workspace_id.is_none());
}

#[test]
fn process_notify_persists_tmux_binding_when_both_fields_present() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-tmux".into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: Some("%12".into()),
        tmux_socket_path: Some("/tmp/tmux-501/default".into()),
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-tmux")
        .unwrap();
    assert_eq!(s.tmux_pane_id.as_deref(), Some("%12"));
    assert_eq!(s.tmux_socket_path.as_deref(), Some("/tmp/tmux-501/default"));
}

#[test]
fn process_notify_herdr_binding_skips_tmux_in_same_request() {
    // tmux 优先级最低:herdr 三件套在场时不落 tmux binding。
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-herdr-tmux".into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: Some("w1:p1".into()),
        herdr_workspace_id: Some("w1".into()),
        herdr_socket_path: Some("/tmp/work/herdr.sock".into()),
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: Some("%12".into()),
        tmux_socket_path: Some("/tmp/tmux-501/default".into()),
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-herdr-tmux")
        .unwrap();
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w1:p1"));
    assert!(s.tmux_pane_id.is_none());
    assert!(s.tmux_socket_path.is_none());
}

#[test]
fn process_notify_skips_herdr_binding_when_any_field_missing() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-half".into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: Some("w3:p1".into()),
        herdr_workspace_id: None, // Missing one
        herdr_socket_path: Some("/tmp/herdr.sock".into()),
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-half")
        .unwrap();
    assert!(s.herdr_pane_id.is_none(), "缺字段应整体不落 binding");
}

// ==================== cmux binding tests ====================

#[test]
fn notify_request_deserializes_cmux_fields() {
    let raw = r#"{
            "title":"t","message":"m","task_id":"cc-abc",
            "source":"claude-code",
            "cmux_surface_id":"FABEB780-8D56-4548-8D92-1E4CB60B7E68",
            "cmux_workspace_id":"4C78AC55-7559-49CF-8C75-784F722C1491"
        }"#;
    let req: super::NotifyRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(
        req.cmux_surface_id.as_deref(),
        Some("FABEB780-8D56-4548-8D92-1E4CB60B7E68")
    );
    assert_eq!(
        req.cmux_workspace_id.as_deref(),
        Some("4C78AC55-7559-49CF-8C75-784F722C1491")
    );
}

#[test]
fn notify_request_without_cmux_fields_defaults_to_none() {
    let raw = r#"{"title":"t","message":"m","task_id":"cc-abc","source":"claude-code"}"#;
    let req: super::NotifyRequest = serde_json::from_str(raw).unwrap();
    assert!(req.cmux_surface_id.is_none());
    assert!(req.cmux_workspace_id.is_none());
}

#[test]
fn process_notify_persists_cmux_binding_when_both_env_vars_present() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-cmux".into(),
        external_session_id: Some("cmux0000-abcd".into()),
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: Some("cmux".into()),
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: Some("SURF-1".into()),
        cmux_workspace_id: Some("WS-1".into()),
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-cmux")
        .unwrap();
    assert_eq!(s.cmux_surface_id.as_deref(), Some("SURF-1"));
    assert_eq!(s.cmux_workspace_id.as_deref(), Some("WS-1"));
}

#[test]
fn process_notify_persists_wezterm_binding_when_pane_and_socket_present() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-wez".into(),
        external_session_id: Some("wez00000-abcd".into()),
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: Some("7".into()),
        wezterm_unix_socket: Some("/tmp/wez.sock".into()),
        wezterm_executable: Some("/Applications/WezTerm.app/Contents/MacOS/wezterm".into()),
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    let (_code, _body) = process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-wez")
        .unwrap();
    assert_eq!(s.wezterm_pane_id.as_deref(), Some("7"));
    assert_eq!(s.wezterm_unix_socket.as_deref(), Some("/tmp/wez.sock"));
    assert_eq!(
        s.wezterm_executable.as_deref(),
        Some("/Applications/WezTerm.app/Contents/MacOS/wezterm")
    );
}

#[test]
fn process_notify_skips_cmux_binding_when_any_field_missing() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-cmux-half".into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        aborted: None,
        terminal_app: Some("cmux".into()),
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: Some("SURF-1".into()),
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None, // 缺一个
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-cmux-half")
        .unwrap();
    assert!(
        s.cmux_surface_id.is_none(),
        "缺 cmux_workspace_id 应整体不落 binding"
    );
}

#[test]
fn process_notify_skips_wezterm_binding_when_socket_missing() {
    let state = test_state();
    let req = NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: "cc-wez-half".into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("SessionStart".into()),
        status: Some("running".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: Some("7".into()),
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    };
    process_notify(&state, req);
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    let s = store
        .sessions
        .iter()
        .find(|s| s.task_id == "cc-wez-half")
        .unwrap();
    assert!(
        s.wezterm_pane_id.is_none(),
        "缺 wezterm_unix_socket 应整体不落 binding"
    );
}

// ==================== /notify decision trace(W2,brief 20260813-decision-trace) ====================
// 决策环文件在 test 构建下可用 POKEPOKE_TEST_NOTIFY_DECISIONS 重定向(见
// runtime_metrics::notify_decisions_path)。env 是进程级共享,相关测试必须串行;
// 其余 process_notify 测试可能并发往同一文件追加,断言一律按 task_id 过滤。

pub(super) static TRACE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn trace_test_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "pokepoke-notify-decisions-{}.jsonl",
        std::process::id()
    ))
}

/// 重定向决策环到临时文件并清空,返回路径。
pub(super) fn redirect_trace_file() -> std::path::PathBuf {
    let path = trace_test_path();
    let _ = std::fs::remove_file(&path);
    std::env::set_var("POKEPOKE_TEST_NOTIFY_DECISIONS", &path);
    path
}

/// 读取决策环中属于指定 task_id 的 trace 行(容忍并发测试的无关行)。
pub(super) fn trace_lines_for(path: &std::path::Path, task_id: &str) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| l.contains(&format!("\"task_id\":\"{task_id}\"")))
        .map(str::to_string)
        .collect()
}

#[test]
fn process_notify_writes_decision_trace_with_gate_verdicts() {
    use axum::http::StatusCode;
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = redirect_trace_file();

    let state = test_state();
    let (code, _body) = process_notify(&state, req_with("cc-trace-1", Some("Stop"), Some("idle")));
    // 新 session → CREATED(201);断言与既有 process_notify 行为一致。
    assert_eq!(code, StatusCode::CREATED);

    let lines = trace_lines_for(&path, "cc-trace-1");
    assert_eq!(lines.len(), 1, "trace 应恰好一条:\n{lines:#?}");
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    // 请求级字段
    assert_eq!(v["task_id"], "cc-trace-1");
    assert_eq!(v["event_type"], "Stop");
    assert!(!v["ts"].as_str().unwrap().is_empty());
    // session 上下文(upsert 后)
    assert!(!v["session_id"].as_str().unwrap().is_empty());
    assert_eq!(v["status"], "idle");
    assert_eq!(v["prev_status"], "");
    assert_eq!(v["is_new"], true);
    // gate verdict:新 session Stop→Idle 应全链路走到 show + 播音
    assert_eq!(v["dedupe_duplicate"], false);
    assert_eq!(v["session_end_handled"], false);
    assert_eq!(v["stage_eligible"], true);
    assert_eq!(v["codex_suppress_reason"], "");
    assert_eq!(v["already_focused"], false);
    assert_eq!(v["quiet_mode_active"], false);
    assert_eq!(v["gate_allow"], true);
    assert_eq!(v["recently_visited"], false);
    assert_eq!(v["stage_acked"], false);
    assert_eq!(v["cooldown_available"], true);
    assert_eq!(v["presentation"], "show");
    assert_eq!(v["suppress_reason"], "");
    assert_eq!(v["still_in_store"], true);
    assert_eq!(v["sound_played"], "played");
}

#[test]
fn process_notify_dedupe_short_circuit_still_writes_trace() {
    use axum::http::StatusCode;
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = redirect_trace_file();

    let state = test_state();
    // 100ms 去重窗口内同 task_id + 同 signature 连发两次:第二次被吞。
    let (code1, _b1) = process_notify(&state, req_with("cc-trace-d1", Some("Stop"), Some("idle")));
    // 新 session → CREATED(201)。
    assert_eq!(code1, StatusCode::CREATED);
    let (code2, body2) =
        process_notify(&state, req_with("cc-trace-d1", Some("Stop"), Some("idle")));
    assert_eq!(code2, StatusCode::OK);
    assert_eq!(body2["deduped"], true);

    let lines = trace_lines_for(&path, "cc-trace-d1");
    assert_eq!(lines.len(), 2, "去重吞掉的请求也必须产 trace:\n{lines:#?}");
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(first["dedupe_duplicate"], false);
    assert_eq!(first["stage_eligible"], true);
    assert_eq!(second["dedupe_duplicate"], true, "第二条应标记 dedupe 命中");
    // dedupe 短路:不进入 upsert / popup gate → session 字段保持缺省
    assert_eq!(second["session_id"], "");
    assert_eq!(second["stage_eligible"], false);
    assert_eq!(second["presentation"], "");
}

#[test]
fn process_notify_session_end_writes_trace_with_flag() {
    use axum::http::StatusCode;
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = redirect_trace_file();

    let state = test_state();
    let (code, _body) = process_notify(&state, req_with("cc-trace-e1", Some("session_end"), None));
    assert_eq!(code, StatusCode::OK);

    let lines = trace_lines_for(&path, "cc-trace-e1");
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["session_end_handled"], true);
    assert_eq!(v["dedupe_duplicate"], false);
    assert_eq!(v["stage_eligible"], false, "SessionEnd 分支不进 popup gate");
}

#[test]
fn protocol_compatible_tri_state() {
    // W9(brief 20260813-protocol-version):不传(None)/ v1 → 兼容;v2+ → 超前。
    assert!(super::notify::protocol_compatible(None));
    assert!(super::notify::protocol_compatible(Some(1)));
    assert!(!super::notify::protocol_compatible(Some(2)));
}

#[test]
fn process_notify_accepts_ahead_protocol_version() {
    // 版本超前不拒收:照常处理(检测点不是拦截点)。
    use axum::http::StatusCode;
    let state = test_state();
    let mut req = req_with("cc-pv2", Some("Stop"), Some("idle"));
    req.protocol_version = Some(2);
    let (code, body) = process_notify(&state, req);
    assert_eq!(code, StatusCode::CREATED, "版本超前仍 201 正常处理");
    assert_eq!(body["task_id"], "cc-pv2");
}

#[test]
fn process_notify_suppress_with_sound_skips_repeat_pending() {
    // brief 20260819-suppress-with-sound-repeat-pending:已聚焦时重复 pending
    // (prev==Pending)不重复响 waiting 音,首次(prev==None/Running)仍响。
    use axum::http::StatusCode;
    use std::time::Duration;
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = redirect_trace_file();

    let mut state = test_state();
    state.focus = Arc::new(FocusedOracle);

    // 首次 pending(新 session,prev=None):已聚焦 → suppress_popup_keep_sound + 播音。
    let (code1, _b1) = process_notify(
        &state,
        req_with("cc-rep-p1", Some("Notification"), Some("pending")),
    );
    assert_eq!(code1, StatusCode::CREATED);

    // 等过 100ms dedupe 窗口后重发 pending(prev=Some(Pending)):应静默不播音。
    std::thread::sleep(Duration::from_millis(120));
    let (code2, _b2) = process_notify(
        &state,
        req_with("cc-rep-p1", Some("Notification"), Some("pending")),
    );
    assert_eq!(code2, StatusCode::OK);

    let lines = trace_lines_for(&path, "cc-rep-p1");
    assert_eq!(lines.len(), 2, "两次 /notify 各一条 trace:\n{lines:#?}");
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(first["presentation"], "suppress_popup_keep_sound");
    assert_eq!(first["sound_played"], "played", "首次 pending 应播音");
    assert_eq!(second["presentation"], "suppress_popup_keep_sound");
    assert_eq!(
        second["sound_played"], "skipped_repeat_pending",
        "重复 pending 应静默"
    );
}
