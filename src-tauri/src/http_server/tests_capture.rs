//! http_server 单测:capture 决策(ghostty eligible / rejection reason /
//! should_reject_capture / wezterm capture reason)。
//! 从 `http_server.rs` 的 `mod tests` 拆出(brief 20260727-split-http-server-rs
//! 第二阶段),`super::*` 引用路径不变,测试内容零改动。

use super::capture::{
    decide_wezterm_capture_reason, ghostty_capture_eligible, ghostty_capture_rejection_reason,
    should_reject_capture,
};
use super::notify::NotifyEventType;
use crate::sessions::{Session, SessionStatus};

#[test]
fn ghostty_capture_gate_allows_cc_codex_pi_none_fallback_with_workspace() {
    // brief 20260804 §4-⑤:pi 进机会性 capture 兜底(pi-in-tmux-in-Ghostty)。
    for (src, app) in [
        ("claude-code", None),
        ("codex", Some("")),
        ("pi", None),
        ("pi", Some("")),
    ] {
        assert!(ghostty_capture_eligible(Some(src), app, Some("/tmp/proj")));
    }
}

#[test]
fn ghostty_capture_gate_allows_cursor_when_terminal_app_is_ghostty() {
    // cursor CLI 跑在 Ghostty 终端:hook 如实上报 terminal_app=ghostty,
    // 此时应与 cc/codex 一样放行 capture(修复:同目录多窗口点击跳转跳错)。
    assert!(ghostty_capture_eligible(
        Some("cursor"),
        Some("ghostty"),
        Some("/tmp/proj")
    ));
}

#[test]
fn ghostty_capture_gate_rejects_cursor_empty_workspace_and_non_ghostty() {
    // cursor 且 terminal_app 缺失(内置终端 / 丢 env 场景)仍拒 —— 只会 REJECT 噪声。
    assert!(!ghostty_capture_eligible(
        Some("cursor"),
        None,
        Some("/tmp/proj")
    ));
    // ghostty 上报但 workspace 为空:无法做 cwd 校验,拒。
    assert!(!ghostty_capture_eligible(
        Some("cursor"),
        Some("ghostty"),
        Some("")
    ));
    assert!(!ghostty_capture_eligible(
        Some("claude-code"),
        Some("ghostty"),
        Some("")
    ));
    assert!(!ghostty_capture_eligible(
        Some("claude-code"),
        Some("iterm.app"),
        Some("/tmp/proj")
    ));
}

#[test]
fn ghostty_capture_rejection_reason_classifies_empty_cwd() {
    assert_eq!(
        ghostty_capture_rejection_reason("", "/Users/edy/proj"),
        Some("captured_cwd_empty")
    );
}

#[test]
fn ghostty_capture_rejection_reason_classifies_empty_workspace() {
    assert_eq!(
        ghostty_capture_rejection_reason("/Users/edy/proj", ""),
        Some("session_workspace_empty")
    );
}

#[test]
fn ghostty_capture_rejection_reason_accepts_compatible_paths() {
    assert_eq!(
        ghostty_capture_rejection_reason("/Users/edy/proj/src", "/Users/edy/proj"),
        None
    );
}

#[test]
fn ghostty_capture_rejection_reason_classifies_incompatible_paths() {
    assert_eq!(
        ghostty_capture_rejection_reason("/Users/edy/other", "/Users/edy/proj"),
        Some("cwd_incompatible")
    );
}

#[test]
fn should_reject_capture_session_start_rejects_empty_cwd() {
    // session_start:cc 刚启动前台未必是 cc tab,cwd 空时宁可不放也不绑错。
    assert!(should_reject_capture(
        Some("captured_cwd_empty"),
        "session_start"
    ));
}

#[test]
fn should_reject_capture_retry_accepts_empty_cwd() {
    // retry(agent 活跃,前台大概率 cc tab):cwd 空视为属性不可用,放行落库 id。
    assert!(!should_reject_capture(
        Some("captured_cwd_empty"),
        "retry-notification"
    ));
    assert!(!should_reject_capture(
        Some("captured_cwd_empty"),
        "retry-user_prompt_submit"
    ));
    assert!(!should_reject_capture(
        Some("captured_cwd_empty"),
        "retry-stop"
    ));
}

#[test]
fn should_reject_capture_always_rejects_incompatible_cwd() {
    // cwd_incompatible:cwd 有值但指向别的 workspace,明确前台是别的 tab,
    // 与 cwd 是否可用无关,一律拒。
    assert!(should_reject_capture(
        Some("cwd_incompatible"),
        "session_start"
    ));
    assert!(should_reject_capture(
        Some("cwd_incompatible"),
        "retry-notification"
    ));
}

#[test]
fn should_reject_capture_accepts_when_no_reject_reason() {
    // cwd 校验通过(reject_reason=None):不论 reason 都落库。
    assert!(!should_reject_capture(None, "session_start"));
    assert!(!should_reject_capture(None, "retry-stop"));
}

#[test]
fn capture_gate_allows_omp_for_ghostty() {
    // brief 20260923-omp-integration:omp 与 pi 同族(终端里跑 CLI,hook 如实上报
    // terminal_app=ghostty)→ 必须进机会性 capture 白名单,否则 omp session 拿不到
    // terminal_id,点击跳转只能退化到按 tty 定位(多 tab 同 cwd 时会绑错)。
    assert!(ghostty_capture_eligible(
        Some("omp"),
        Some("ghostty"),
        Some("/tmp/proj")
    ));
    // 与 cc/pi 同款:terminal_app 缺失也放行(pi-in-tmux-in-Ghostty 兜底语义)
    assert!(ghostty_capture_eligible(
        Some("omp"),
        None,
        Some("/tmp/proj")
    ));
}

#[test]
fn capture_gate_allows_omp_for_wezterm() {
    // wezterm 侧的 omp 白名单同源:请求带 socket 时应触发 capture(而非被 source 门禁拒)。
    let session = wezterm_session("omp");
    assert!(
        decide_wezterm_capture_reason(
            &session,
            None,
            None,
            Some("/tmp/wez.sock"),
            false,
            Some(NotifyEventType::Running),
        )
        .is_some(),
        "omp 必须被 wezterm capture 门禁放行"
    );
}

/// wezterm capture 测试用的 session 模板(只改 source)。
fn wezterm_session(source: &str) -> Session {
    use chrono::Utc;
    Session {
        id: "s-wez".into(),
        task_id: "cc-wez-cap".into(),
        external_session_id: None,
        title: "t".into(),
        message: "m".into(),
        source: Some(source.into()),
        priority: Default::default(),
        status: SessionStatus::Running,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        terminal_tty: Some("/dev/ttys001".into()),
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted_at: None,
        terminal_id: None,
        terminal_id_captured_cwd: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: Some("/tmp/wez.sock".into()),
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
        surface: Default::default(),
    }
}

#[test]
fn wezterm_capture_reason_triggers_with_req_socket_when_unbound() {
    let session = wezterm_session("claude-code");
    // W6 语义:旧"已存 socket + pane 缺"状态在新模型不可达(Wezterm variant 必带
    // pane;set 双写保证)。可达等价场景 = 无绑定 + 请求带 socket → capture 触发。
    assert_eq!(
        decide_wezterm_capture_reason(
            &session,
            None,
            None,
            Some("/tmp/wez.sock"),
            false,
            Some(NotifyEventType::Running),
        ),
        Some("retry-user_prompt_submit")
    );
}

#[test]
fn wezterm_capture_reason_skips_when_pane_already_bound() {
    use chrono::Utc;
    let session = Session {
        id: "s-wez".into(),
        task_id: "cc-wez-cap".into(),
        external_session_id: None,
        title: "t".into(),
        message: "m".into(),
        source: Some("claude-code".into()),
        priority: Default::default(),
        status: SessionStatus::Running,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
        failure_reason: None,
        aborted_at: None,
        terminal_id: None,
        terminal_id_captured_cwd: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: Some("7".into()),
        wezterm_unix_socket: Some("/tmp/wez.sock".into()),
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
    };
    assert!(decide_wezterm_capture_reason(
        &session,
        None,
        None,
        None,
        false,
        Some(NotifyEventType::Running),
    )
    .is_none());
}
