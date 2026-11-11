//! surface 单测:SurfaceKind / resolve_surface / evidence 解析(从 surface.rs
//! 拆出,编排者批准;surface_binding_tests.rs 另管 SurfaceBinding 相关)。

use super::*;

fn evidence() -> SurfaceEvidence {
    SurfaceEvidence {
        source: None,
        terminal_app: None,
        dsh_nav_socket: None,
        task_id: "task-1".to_string(),
        session_id: "session-1".to_string(),
        external_session_id: None,
        workspace_path: None,
        terminal_tty: None,
        terminal_id: None,
        terminal_id_captured_cwd: None,
        herdr_pane_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
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
    }
}

#[test]
fn tty_resolves_to_exact_terminal_surface() {
    let mut evidence = evidence();
    evidence.terminal_tty = Some("/dev/ttys001".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::TerminalTty);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "Terminal");
    assert_eq!(surface.reason, "exact_tty");
    assert_eq!(
        surface.capabilities,
        SurfaceCapabilities {
            can_focus: true,
            can_detect_focused: true,
            can_locate_screen: true,
            can_check_liveness: true,
        }
    );
}

#[test]
fn ghostty_id_resolves_to_exact_ghostty_surface_without_tty() {
    let mut evidence = evidence();
    evidence.terminal_id = Some("ghostty-terminal-id".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::GhosttyTerminal);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "Ghostty");
    assert_eq!(surface.reason, "exact_ghostty_terminal_id");
    assert_eq!(
        surface.capabilities,
        SurfaceCapabilities {
            can_focus: true,
            can_detect_focused: true,
            can_locate_screen: true,
            can_check_liveness: true,
        }
    );
}

#[test]
fn cursor_without_tty_resolves_to_probable_workspace_surface() {
    let mut evidence = evidence();
    evidence.source = Some("cursor".to_string());
    evidence.workspace_path = Some("/tmp/workspace".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::CursorWorkspace);
    assert_eq!(surface.confidence, SurfaceConfidence::Probable);
    assert_eq!(surface.display_name, "Cursor");
    assert_eq!(surface.reason, "cursor_gui_fallback");
    assert_eq!(
        surface.capabilities,
        SurfaceCapabilities {
            can_focus: true,
            can_detect_focused: true,
            can_locate_screen: false,
            can_check_liveness: true,
        }
    );
}

#[test]
fn cmux_binding_with_tty_enables_focus_detection() {
    let mut evidence = evidence();
    evidence.terminal_tty = Some("/dev/ttys030".to_string());
    evidence.cmux_surface_id = Some("ED15CEBF-44D6-4B03-B7CE-851393933ADE".to_string());

    let surface = resolve_surface(&evidence);

    assert!(surface.capabilities.can_detect_focused);
}

#[test]
fn wezterm_binding_enables_focus_detection() {
    let mut evidence = evidence();
    evidence.wezterm_pane_id = Some("7".to_string());
    evidence.wezterm_unix_socket = Some("/tmp/wez.sock".to_string());

    let surface = resolve_surface(&evidence);

    assert!(surface.capabilities.can_detect_focused);
}

#[test]
fn non_cursor_workspace_resolves_to_finder_fallback() {
    let mut evidence = evidence();
    evidence.source = Some("claude".to_string());
    evidence.workspace_path = Some("/tmp/workspace".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::FinderWorkspace);
    assert_eq!(surface.confidence, SurfaceConfidence::Fallback);
    assert_eq!(surface.display_name, "Finder");
    assert_eq!(surface.reason, "finder_fallback");
    assert_eq!(
        surface.capabilities,
        SurfaceCapabilities {
            can_focus: true,
            can_detect_focused: false,
            can_locate_screen: false,
            can_check_liveness: false,
        }
    );
}

#[test]
fn cursor_cli_with_tty_resolves_to_exact_terminal() {
    // Cursor CLI session has a tty → should resolve as TerminalTty,
    // not CursorWorkspace. The tty path is more precise (iTerm2/Terminal
    // exact match + Ghostty fallback in focus_terminal).
    let mut evidence = evidence();
    evidence.source = Some("cursor".to_string());
    evidence.terminal_tty = Some("/dev/ttys005".to_string());
    evidence.workspace_path = Some("/tmp/workspace".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::TerminalTty);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.reason, "exact_tty");
}

#[test]
fn ghostty_with_both_tty_and_terminal_id_prefers_tty() {
    // When a Ghostty session has both tty and terminal_id, the resolver
    // prefers tty (TerminalTty). This is the "stale terminal_id" safety
    // net: the terminal_id might point to a recycled Ghostty tab, but the
    // tty is still valid and the focus_terminal path has a built-in
    // Ghostty L1/L2 fallback anyway.
    let mut evidence = evidence();
    evidence.source = Some("claude-code".to_string());
    evidence.terminal_tty = Some("/dev/ttys003".to_string());
    evidence.terminal_id = Some("ghostty-abc-123".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::TerminalTty);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.reason, "exact_tty");
}

#[test]
fn empty_strings_are_treated_as_missing_evidence() {
    let evidence = SurfaceEvidence {
        source: Some("".to_string()),
        terminal_app: Some("".to_string()),
        dsh_nav_socket: None,
        task_id: "task-1".to_string(),
        session_id: "session-1".to_string(),
        external_session_id: Some("".to_string()),
        workspace_path: Some("".to_string()),
        terminal_tty: Some("".to_string()),
        terminal_id: Some("".to_string()),
        terminal_id_captured_cwd: Some("".to_string()),
        herdr_pane_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
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
    };

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::Unknown);
    assert_eq!(surface.confidence, SurfaceConfidence::Unknown);
    assert_eq!(surface.display_name, "Unknown");
    assert_eq!(surface.reason, "missing_surface_evidence");
    assert_eq!(
        surface.capabilities,
        SurfaceCapabilities {
            can_focus: false,
            can_detect_focused: false,
            can_locate_screen: false,
            can_check_liveness: false,
        }
    );
}

#[test]
fn codex_desktop_without_tty_uses_thread_id() {
    // ChatGPT 桌面端会话:codex 跑在 Electron 里,无 tty / terminal_id,但带
    // thread UUID(external_session_id)。应走 CodexDesktop deep link,不落 Finder no-op。
    let mut evidence = evidence();
    evidence.source = Some("codex".to_string());
    evidence.external_session_id = Some("019f5fbb-1872-7172-a623-e51fa7f31d06".to_string());
    evidence.workspace_path = Some("/Users/edy/Documents/Codex/hi".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::CodexDesktop);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "ChatGPT");
    assert_eq!(surface.reason, "codex_desktop_thread");
    assert!(surface.capabilities.can_focus);
    assert!(!surface.capabilities.can_check_liveness);
}

#[test]
fn dsh_session_without_tty_uses_dsh_surface() {
    // DSH 是 GUI/Web 宿主:无 tty,身份是 hook 上报的 session id。
    let mut evidence = evidence();
    evidence.source = Some("dsh".to_string());
    evidence.external_session_id = Some("session-bed025d2-2c09-4387-96f5-bcba51d2dd92".into());
    evidence.workspace_path = Some("/Users/edy/Documents/proj".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::DshSession);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "DeepSeek Harness");
    assert_eq!(surface.reason, "dsh_session");
    assert!(surface.capabilities.can_focus);
    assert!(!surface.capabilities.can_detect_focused);
    assert!(!surface.capabilities.can_check_liveness);
}

#[test]
fn dsh_socket_enables_focus_detection() {
    let mut evidence = evidence();
    evidence.source = Some("dsh".to_string());
    evidence.external_session_id = Some("session-bed025d2".into());
    evidence.dsh_nav_socket = Some("/Users/edy/.pokepoke/dsh-123.sock".into());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::DshSession);
    assert!(
        surface.capabilities.can_detect_focused,
        "有 socket 时 popup gate / watcher 才能查渲染进程状态"
    );
}

#[test]
fn codex_cli_with_tty_still_uses_terminal_not_desktop() {
    // codex CLI 在终端跑时有 tty,即便也带 thread UUID,也应走 TerminalTty
    // (终端精确定位 > 桌面端 deep link),不能误入 CodexDesktop。
    let mut evidence = evidence();
    evidence.source = Some("codex".to_string());
    evidence.external_session_id = Some("019f5fbb-1872-7172-a623-e51fa7f31d06".to_string());
    evidence.terminal_tty = Some("/dev/ttys005".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::TerminalTty);
    assert_eq!(surface.reason, "exact_tty");
}

#[test]
fn otty_binding_without_tty_resolves_to_otty_terminal() {
    let mut evidence = evidence();
    evidence.otty_pane_id = Some("pane-1".to_string());
    evidence.workspace_path = Some("/tmp/workspace".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::OttyTerminal);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "Otty");
    assert_eq!(surface.reason, "exact_otty_pane");
    assert!(surface.capabilities.can_detect_focused);
}

#[test]
fn orca_binding_without_tty_resolves_to_orca_terminal() {
    let mut evidence = evidence();
    evidence.orca_pane_key = Some("tab-a:leaf-b".to_string());
    evidence.workspace_path = Some("/tmp/workspace".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::OrcaTerminal);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.display_name, "Orca");
    assert_eq!(surface.reason, "exact_orca_pane");
    assert!(surface.capabilities.can_detect_focused);
}

#[test]
fn orca_binding_takes_priority_over_otty_when_both_present() {
    let mut evidence = evidence();
    evidence.orca_pane_key = Some("tab-a:leaf-b".to_string());
    evidence.otty_pane_id = Some("pane-1".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::OrcaTerminal);
}

#[test]
fn orca_binding_takes_priority_over_tty_when_both_present() {
    let mut evidence = evidence();
    evidence.orca_pane_key = Some("tab-a:leaf-b".to_string());
    evidence.terminal_tty = Some("/dev/ttys039".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::OrcaTerminal);
    assert_eq!(surface.reason, "exact_orca_pane");
}

#[test]
fn otty_binding_takes_priority_over_tty_when_both_present() {
    let mut evidence = evidence();
    evidence.otty_pane_id = Some("pane-1".to_string());
    evidence.terminal_tty = Some("/dev/ttys039".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::OttyTerminal);
    assert_eq!(surface.reason, "exact_otty_pane");
}

#[test]
fn codex_desktop_without_thread_id_falls_through() {
    // codex 桌面端会话若 thread UUID 缺失(hook 解析失败 / 老数据),应回落
    // 既有 FinderWorkspace no-op,而不是假装能跳。
    let mut evidence = evidence();
    evidence.source = Some("codex".to_string());
    evidence.workspace_path = Some("/Users/edy/Documents/Codex/hi".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::FinderWorkspace);
}

#[test]
fn tmux_binding_without_tty_resolves_to_tmux_surface() {
    let mut evidence = evidence();
    evidence.tmux_pane_id = Some("%12".to_string());
    evidence.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.reason, "exact_tmux_pane");
    assert_eq!(surface.display_name, "tmux");
    assert!(
        surface.capabilities.can_detect_focused,
        "binding 类应可判聚焦"
    );
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
}

#[test]
fn tmux_binding_with_tty_enables_focus_detection() {
    // 带 tty 的 tmux session 走 TerminalTty;can_detect_focused 由
    // terminal_focus_capabilities 里的 tmux binding 支撑。
    let mut evidence = evidence();
    evidence.terminal_tty = Some("/dev/ttys039".to_string());
    evidence.tmux_pane_id = Some("%12".to_string());
    evidence.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());

    let surface = resolve_surface(&evidence);

    assert_eq!(surface.kind, SurfaceKind::TerminalTty);
    assert_eq!(surface.reason, "exact_tty");
    assert!(surface.capabilities.can_detect_focused);
}

#[test]
fn tmux_binding_lower_priority_than_wezterm_otty_orca() {
    // tmux 优先级最低:wezterm/otty/orca binding 在场时(无 tty)解析为它们的 surface。
    let mut with_wezterm = evidence();
    with_wezterm.tmux_pane_id = Some("%12".to_string());
    with_wezterm.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());
    with_wezterm.wezterm_pane_id = Some("7".to_string());
    with_wezterm.wezterm_unix_socket = Some("/tmp/wez.sock".to_string());
    assert_eq!(resolve_surface(&with_wezterm).reason, "exact_wezterm_pane");

    let mut with_otty = evidence();
    with_otty.tmux_pane_id = Some("%12".to_string());
    with_otty.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());
    with_otty.otty_pane_id = Some("pane-1".to_string());
    assert_eq!(resolve_surface(&with_otty).reason, "exact_otty_pane");

    let mut with_orca = evidence();
    with_orca.tmux_pane_id = Some("%12".to_string());
    with_orca.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());
    with_orca.orca_pane_key = Some("tab-a:leaf-b".to_string());
    assert_eq!(resolve_surface(&with_orca).reason, "exact_orca_pane");
}

#[test]
fn tmux_binding_lower_priority_than_herdr_cmux_with_tty() {
    // herdr/cmux 带 tty 的 session 走 TerminalTty,先于 herdr/tmux branch 命中
    // (tty 分支在 herdr/tmux binding 分支之前,与既有行为一致)。
    let mut with_herdr = evidence();
    with_herdr.terminal_tty = Some("/dev/ttys039".to_string());
    with_herdr.herdr_pane_id = Some("w1:p1".to_string());
    with_herdr.tmux_pane_id = Some("%12".to_string());
    with_herdr.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());
    assert_eq!(resolve_surface(&with_herdr).reason, "exact_tty");

    let mut with_cmux = evidence();
    with_cmux.terminal_tty = Some("/dev/ttys039".to_string());
    with_cmux.cmux_surface_id = Some("SURF".to_string());
    with_cmux.tmux_pane_id = Some("%12".to_string());
    with_cmux.tmux_socket_path = Some("/tmp/tmux-501/default".to_string());
    assert_eq!(resolve_surface(&with_cmux).reason, "exact_tty");
}

// ==== herdr binding(brief 20260810-surface-herdr-branch)====

/// herdr 双字段在场(无 tty)→ HerdrPane,本地 can_detect_focused=true
/// (focus-watcher 启动,聚焦 pane 自动回收 popup)。
#[test]
fn herdr_binding_resolves_to_herdr_pane() {
    let mut ev = evidence();
    ev.herdr_pane_id = Some("w1:p1".to_string());
    ev.herdr_socket_path = Some("/Users/edy/.config/herdr/herdr.sock".to_string());

    let surface = resolve_surface(&ev);
    assert_eq!(surface.kind, SurfaceKind::HerdrPane);
    assert_eq!(surface.confidence, SurfaceConfidence::Exact);
    assert_eq!(surface.reason, "exact_herdr_binding");
    assert!(
        surface.capabilities.can_detect_focused,
        "本地 herdr 可判聚焦"
    );
    assert!(surface.capabilities.can_focus);
}

/// herdr-remote:聚焦不可判(20260807 决策),can_detect_focused=false
/// 避免 watcher 每 500ms osascript 空转。
#[test]
fn herdr_remote_cannot_detect_focused() {
    let mut ev = evidence();
    ev.source = Some("herdr-remote".to_string());
    ev.herdr_pane_id = Some("w1:p1".to_string());
    ev.herdr_socket_path = Some("/Users/edy/.pokepoke/herdr-remote-hello.sock".to_string());

    let surface = resolve_surface(&ev);
    assert_eq!(surface.kind, SurfaceKind::HerdrPane);
    assert!(!surface.capabilities.can_detect_focused, "远程聚焦不可判");
}

/// herdr binding 优先于 workspace fallback:之前落 FinderWorkspace,
/// 是本次修复的误导日志来源。
#[test]
fn herdr_beats_workspace_fallback() {
    let mut ev = evidence();
    ev.workspace_path = Some("/Users/edy/PMaker".to_string());
    ev.herdr_pane_id = Some("w8:pM".to_string());
    ev.herdr_socket_path = Some("/Users/edy/.config/herdr/herdr.sock".to_string());

    let surface = resolve_surface(&ev);
    assert_eq!(surface.kind, SurfaceKind::HerdrPane);
    assert_ne!(surface.kind, SurfaceKind::FinderWorkspace);
}

/// herdr-in-Orca:orca binding 分支在最前,与 herdr binding 并存时 orca 优先
/// (段 A 的 Orca 定位依赖 orca_pane_key,不能被 herdr 分支截走)。
#[test]
fn orca_priority_over_herdr_binding() {
    let mut ev = evidence();
    ev.herdr_pane_id = Some("w1:p1".to_string());
    ev.herdr_socket_path = Some("/Users/edy/.config/herdr/herdr.sock".to_string());
    ev.orca_pane_key = Some("wt1:tab1".to_string());

    let surface = resolve_surface(&ev);
    assert_eq!(surface.kind, SurfaceKind::OrcaTerminal);
}

/// herdr binding 低于 tty:带 tty 的 herdr session(理论上不会同时出现,
/// 防御性)仍走 TerminalTty。
#[test]
fn tty_priority_over_herdr_binding() {
    let mut ev = evidence();
    ev.terminal_tty = Some("/dev/ttys039".to_string());
    ev.herdr_pane_id = Some("w1:p1".to_string());
    ev.herdr_socket_path = Some("/Users/edy/.config/herdr/herdr.sock".to_string());

    assert_eq!(resolve_surface(&ev).kind, SurfaceKind::TerminalTty);
}
