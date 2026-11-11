#[cfg(test)]
mod focus_target_tests {
    use super::super::{decide_focus_target, FocusTarget};

    #[test]
    fn cc_with_tty_goes_terminal() {
        assert_eq!(
            decide_focus_target(
                Some("claude-code"),
                Some("/dev/ttys001"),
                Some("/tmp/a"),
                None,
                None
            ),
            FocusTarget::Terminal {
                tty: "/dev/ttys001".into()
            }
        );
    }

    #[test]
    fn dsh_without_tty_goes_dsh_session() {
        assert_eq!(
            decide_focus_target(
                Some("dsh"),
                None,
                Some("/tmp/a"),
                None,
                Some("session-bed025d2-2c09-4387-96f5-bcba51d2dd92")
            ),
            FocusTarget::DshSession {
                session_id: "session-bed025d2-2c09-4387-96f5-bcba51d2dd92".into()
            }
        );
    }

    #[test]
    fn cursor_with_tty_goes_terminal() {
        // iTerm2 里跑 `cursor-agent` 时 hook 抓到 tty → 精确定位 tab
        assert_eq!(
            decide_focus_target(
                Some("cursor"),
                Some("/dev/ttys002"),
                Some("/tmp/a"),
                None,
                None
            ),
            FocusTarget::Terminal {
                tty: "/dev/ttys002".into()
            }
        );
    }

    #[test]
    fn cursor_without_tty_goes_gui() {
        // Cursor GUI 聊天触发 hook tty=None → 拉 Cursor GUI
        assert_eq!(
            decide_focus_target(Some("cursor"), None, Some("/tmp/a"), None, None),
            FocusTarget::CursorGui {
                path: "/tmp/a".into()
            }
        );
    }

    #[test]
    fn ghostty_id_without_tty_goes_ghostty() {
        assert_eq!(
            decide_focus_target(
                Some("claude-code"),
                None,
                Some("/tmp/a"),
                Some("ghost-id-1"),
                None
            ),
            FocusTarget::Ghostty {
                terminal_id: "ghost-id-1".into()
            }
        );
    }

    #[test]
    fn cursor_with_empty_tty_goes_gui() {
        assert_eq!(
            decide_focus_target(Some("cursor"), Some(""), Some("/tmp/a"), None, None),
            FocusTarget::CursorGui {
                path: "/tmp/a".into()
            }
        );
    }

    #[test]
    fn cc_without_tty_goes_finder() {
        assert_eq!(
            decide_focus_target(Some("claude-code"), None, Some("/tmp/a"), None, None),
            FocusTarget::Finder {
                path: "/tmp/a".into()
            }
        );
    }

    #[test]
    fn codex_without_tty_goes_finder() {
        assert_eq!(
            decide_focus_target(Some("codex"), None, Some("/tmp/a"), None, None),
            FocusTarget::Finder {
                path: "/tmp/a".into()
            }
        );
    }

    #[test]
    fn codex_desktop_with_thread_goes_codex_app() {
        // ChatGPT 桌面端会话:codex 无 tty,但带 thread UUID(external_session_id)。
        // 应走 CodexDesktop(`codex://threads/<id>`),不落 Finder no-op。
        assert_eq!(
            decide_focus_target(
                Some("codex"),
                None,
                Some("/tmp/a"),
                None,
                Some("019f5fbb-1872-7172-a623-e51fa7f31d06")
            ),
            FocusTarget::CodexDesktop {
                thread_id: "019f5fbb-1872-7172-a623-e51fa7f31d06".into()
            }
        );
    }

    #[test]
    fn codex_cli_with_tty_beats_desktop_thread() {
        // codex CLI 在终端跑时有 tty,即便也带 thread UUID,终端精确定位优先。
        assert_eq!(
            decide_focus_target(
                Some("codex"),
                Some("/dev/ttys005"),
                Some("/tmp/a"),
                None,
                Some("019f5fbb-1872-7172-a623-e51fa7f31d06")
            ),
            FocusTarget::Terminal {
                tty: "/dev/ttys005".into()
            }
        );
    }

    #[test]
    fn cursor_without_tty_and_path_goes_nothing() {
        assert_eq!(
            decide_focus_target(Some("cursor"), None, None, None, None),
            FocusTarget::Nothing
        );
    }

    #[test]
    fn cc_without_tty_and_path_goes_nothing() {
        assert_eq!(
            decide_focus_target(Some("claude-code"), None, None, None, None),
            FocusTarget::Nothing
        );
    }

    #[test]
    fn unknown_source_with_tty_still_goes_terminal() {
        assert_eq!(
            decide_focus_target(None, Some("/dev/ttys003"), Some("/tmp/a"), None, None),
            FocusTarget::Terminal {
                tty: "/dev/ttys003".into()
            }
        );
    }

    #[test]
    fn unknown_source_without_tty_goes_finder() {
        // source=None 当作非 cursor，走 Finder，保持现状兼容
        assert_eq!(
            decide_focus_target(None, None, Some("/tmp/a"), None, None),
            FocusTarget::Finder {
                path: "/tmp/a".into()
            }
        );
    }
}

/// 被覆盖窗口的 selected tab 换掉)。修复后 Phase 1 只存 `targetWindow`,Phase 2
/// 在 `tabs of front window` 里重新解析。这里跑不了真 osascript,但能断言脚本
/// 不再含"跨翻转复用引用"的反模式,防止后人无意改回。
#[cfg(test)]
mod focus_script_structure_tests {
    use super::super::ghostty::{focus_ghostty_by_id_script, focus_ghostty_l2_cwd_script};
    use super::super::iterm2::focus_iterm2_script;
    use super::super::terminal_app::focus_terminal_app_script;

    #[test]
    fn l1_script_does_not_store_tab_or_terminal_in_phase1() {
        let s = focus_ghostty_by_id_script("UUID-1");
        // Phase 1 遍历块里不得出现 targetTab / targetTerminal 赋值。
        // 取第一个 'activate' 之前的片段作为 Phase 1。
        let phase1 = s.split("activate\n").next().unwrap_or(&s);
        assert!(
            !phase1.contains("set targetTab to"),
            "Phase 1 must not store targetTab (stale-ref bug): {phase1}"
        );
        assert!(
            !phase1.contains("set targetTerminal to"),
            "Phase 1 must not store targetTerminal (stale-ref bug): {phase1}"
        );
        assert!(
            phase1.contains("set targetWindow to aWindow"),
            "Phase 1 must still locate targetWindow"
        );
    }

    #[test]
    fn l1_script_re_resolves_via_front_window_in_phase2() {
        let s = focus_ghostty_by_id_script("UUID-1");
        // Phase 2(activate window 之后)必须在 tabs of front window 里重新解析。
        let after_activate = s.split("activate window targetWindow").nth(1).unwrap_or("");
        assert!(
            after_activate.contains("tabs of front window"),
            "Phase 2 must re-resolve via `tabs of front window`, got: {after_activate}"
        );
        // 仍要用 select tab + focus 完成跳转,且读回验证用 front window。
        assert!(after_activate.contains("select tab theTab"));
        assert!(after_activate.contains("focus theTerm"));
        assert!(after_activate.contains("focused terminal of selected tab of front window"));
    }

    #[test]
    fn l1_script_embeds_escaped_id() {
        let s = focus_ghostty_by_id_script("my-id-123");
        assert!(s.contains("\"my-id-123\""), "script must embed the id");
    }

    #[test]
    fn l2_script_does_not_store_tab_or_terminal_in_phase1() {
        let s = focus_ghostty_l2_cwd_script("/Users/edy/proj");
        let phase1 = s.split("activate\n").next().unwrap_or(&s);
        assert!(
            !phase1.contains("set targetTab to"),
            "L2 Phase 1 must not store targetTab (stale-ref bug): {phase1}"
        );
        assert!(
            !phase1.contains("set targetTerminal to"),
            "L2 Phase 1 must not store targetTerminal (stale-ref bug): {phase1}"
        );
        assert!(phase1.contains("set targetWindow to aWindow"));
    }

    #[test]
    fn l2_script_re_resolves_via_front_window_in_phase2() {
        let s = focus_ghostty_l2_cwd_script("/Users/edy/proj");
        let after_activate = s.split("activate window targetWindow").nth(1).unwrap_or("");
        assert!(
            after_activate.contains("tabs of front window"),
            "L2 Phase 2 must re-resolve via `tabs of front window`, got: {after_activate}"
        );
        assert!(after_activate.contains("select tab theTab"));
        assert!(after_activate.contains("focus theTerm"));
    }

    #[test]
    fn l2_script_embeds_escaped_path() {
        let s = focus_ghostty_l2_cwd_script("/Users/edy/proj");
        assert!(
            s.contains("\"/Users/edy/proj\""),
            "script must embed the path"
        );
    }

    // ---- 2026-07-14 herdr-in-iTerm2 回跳偶发失败加固 ----
    // focus_iterm2 / focus_terminal_app 必须包 try/on error,否则 select 偶发抛异常
    // 会被 osascript 非零退出吞成 false 且无日志(根因 A)。这里断言脚本结构,防回归。

    #[test]
    fn focus_iterm2_script_has_on_error_fallback() {
        let s = focus_iterm2_script("/dev/ttys023");
        assert!(s.contains("try"), "must wrap body in try: {s}");
        assert!(s.contains("on error"), "must have on-error handler: {s}");
        // on error 分支必须 activate(兜底拉 iTerm2 前台,不能裸 return false)
        let on_error_part = s.split("on error").nth(1).unwrap_or("");
        assert!(
            on_error_part.contains("activate"),
            "on-error must activate as fallback: {s}"
        );
    }

    #[test]
    fn focus_iterm2_script_returns_three_states() {
        let s = focus_iterm2_script("/dev/ttys023");
        // 精确命中 / 没找到 / 异常兜底 三态,run_bool 只能给 bool,这里要三态
        assert!(s.contains(r#"return "ok""#), "must return ok on hit: {s}");
        assert!(
            s.contains(r#"return "miss""#),
            "must return miss when tty not found: {s}"
        );
        assert!(
            s.contains(r#"return "activated""#),
            "must return activated on error fallback: {s}"
        );
    }

    #[test]
    fn focus_iterm2_script_embeds_tty() {
        let s = focus_iterm2_script("/dev/ttys042");
        assert!(
            s.contains("\"/dev/ttys042\""),
            "script must embed the tty: {s}"
        );
    }

    #[test]
    fn focus_terminal_app_script_has_on_error_fallback() {
        let s = focus_terminal_app_script("/dev/ttys023");
        assert!(s.contains("try"), "must wrap body in try: {s}");
        assert!(s.contains("on error"), "must have on-error handler: {s}");
        let on_error_part = s.split("on error").nth(1).unwrap_or("");
        assert!(
            on_error_part.contains("activate"),
            "on-error must activate as fallback: {s}"
        );
    }

    #[test]
    fn focus_terminal_app_script_returns_three_states() {
        let s = focus_terminal_app_script("/dev/ttys023");
        assert!(s.contains(r#"return "ok""#), "must return ok on hit: {s}");
        assert!(
            s.contains(r#"return "miss""#),
            "must return miss when tty not found: {s}"
        );
        assert!(
            s.contains(r#"return "activated""#),
            "must return activated on error fallback: {s}"
        );
    }

    // ---- 2026-07-24 dual herdr instance routing ----

    #[test]
    fn ghostty_herdr_session_title_filter_dual_instance() {
        assert!(crate::herdr::ghostty_tab_title_matches_named_session(
            "herdr --session work",
            Some("work")
        ));
        assert!(!crate::herdr::ghostty_tab_title_matches_named_session(
            "herdr",
            Some("work")
        ));
        assert!(!crate::herdr::ghostty_tab_title_matches_named_session(
            "herdr --session work",
            None
        ));
        assert!(crate::herdr::ghostty_tab_title_matches_named_session(
            "herdr", None
        ));
    }
}
