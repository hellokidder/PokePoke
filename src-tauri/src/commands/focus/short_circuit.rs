//! herdr → cmux → wezterm → orca → otty 优先 short-circuit 链。
//! `focus_session_terminal_blocking` 与 `open_session_source_blocking` 共用,
//! 避免单边修复导致行为漂移。

use crate::sessions::SessionStore;
use std::sync::{Arc, Mutex};

use super::cmux::{clear_stale_cmux_binding, focus_cmux_surface};
use super::herdr::{focus_herdr_host_anchor, HerdrHostAnchorHint};
use super::orca::{clear_stale_orca_binding, focus_orca_pane};
use super::otty::{clear_stale_otty_binding, focus_otty_pane};
use super::tmux::focus_tmux_pane;
use super::wezterm::{clear_stale_wezterm_binding, focus_wezterm_surface};

/// Session 上驱动 short-circuit 链所需的 binding 字段快照。
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionFocusBindings {
    pub workspace_path: Option<String>,
    pub terminal_tty: Option<String>,
    pub herdr_pane_id: Option<String>,
    pub herdr_socket_path: Option<String>,
    /// 来源是 herdr-remote(SSH 转发到远程 herdr)。为 true 时整条短链直接短路,
    /// 不做任何本地宿主锚定(brief 20260807-herdr-remote-jumpback-fix)。
    pub is_herdr_remote: bool,
    pub cmux_surface_id: Option<String>,
    pub cmux_workspace_id: Option<String>,
    pub wezterm_pane_id: Option<String>,
    pub wezterm_unix_socket: Option<String>,
    pub wezterm_executable: Option<String>,
    pub orca_pane_key: Option<String>,
    pub orca_worktree_id: Option<String>,
    pub otty_pane_id: Option<String>,
    pub otty_tab_id: Option<String>,
    pub tmux_pane_id: Option<String>,
    pub tmux_socket_path: Option<String>,
}

/// short-circuit 链执行结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TerminalShortCircuitOutcome {
    pub did_herdr: bool,
    pub did_cmux: bool,
    pub did_wezterm: bool,
    pub did_orca: bool,
    pub did_otty: bool,
}

/// 按优先级尝试 herdr → cmux → wezterm → orca → otty short-circuit。
pub(crate) fn run_terminal_short_circuits(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    bindings: &SessionFocusBindings,
) -> TerminalShortCircuitOutcome {
    // herdr-remote(agent 跑在远程机,SSH 转发 socket):本地不存在宿主终端,
    // 段 A 的本地锚定(herdr client tty / Ghostty herdr tab / cmux / orca)必然
    // 误吸本地别的 herdr 相关 tab(实测:远程 socket 匹配到本地 herdr tab 被切走)。
    // 直接短路,段 B(focus_herdr_if_bound)由调用方独立执行,不受影响。
    if bindings.is_herdr_remote {
        return TerminalShortCircuitOutcome::default();
    }
    let herdr_hint = HerdrHostAnchorHint {
        wezterm_unix_socket: bindings.wezterm_unix_socket.as_deref(),
        wezterm_executable: bindings.wezterm_executable.as_deref(),
        orca_pane_key: bindings.orca_pane_key.as_deref(),
        orca_worktree_id: bindings.orca_worktree_id.as_deref(),
        workspace_path: bindings.workspace_path.as_deref(),
    };
    let did_herdr = bindings.herdr_pane_id.is_some()
        && focus_herdr_host_anchor(bindings.herdr_socket_path.as_deref(), &herdr_hint);

    let did_cmux = if !did_herdr {
        if let Some(surface) = bindings.cmux_surface_id.as_deref() {
            let ok = focus_cmux_surface(surface, bindings.cmux_workspace_id.as_deref());
            if !ok {
                clear_stale_cmux_binding(store, session_id, surface);
            }
            ok
        } else {
            false
        }
    } else {
        false
    };

    let did_wezterm = if !did_herdr && !did_cmux {
        if let (Some(pane), Some(sock)) = (
            bindings.wezterm_pane_id.as_deref(),
            bindings.wezterm_unix_socket.as_deref(),
        ) {
            let ok = focus_wezterm_surface(
                pane,
                sock,
                bindings.wezterm_executable.as_deref(),
                bindings.workspace_path.as_deref(),
                bindings.terminal_tty.as_deref(),
            );
            if !ok
                && crate::wezterm::should_clear_wezterm_binding_after_focus_fail(
                    pane,
                    sock,
                    bindings.wezterm_executable.as_deref(),
                )
            {
                clear_stale_wezterm_binding(store, session_id, pane);
            }
            ok
        } else {
            false
        }
    } else {
        false
    };

    let did_orca = if !did_herdr && !did_cmux && !did_wezterm {
        if let Some(pane_key) = bindings.orca_pane_key.as_deref() {
            let ok = focus_orca_pane(pane_key, bindings.orca_worktree_id.as_deref());
            if !ok
                && super::orca::should_clear_orca_binding_after_focus_fail(
                    pane_key,
                    bindings.orca_worktree_id.as_deref(),
                )
            {
                clear_stale_orca_binding(store, session_id, pane_key);
            }
            ok
        } else {
            false
        }
    } else {
        false
    };

    let did_otty =
        if !did_herdr && !did_cmux && !did_wezterm && !did_orca && bindings.orca_pane_key.is_none()
        {
            if let Some(pane) = bindings.otty_pane_id.as_deref() {
                let ok = focus_otty_pane(
                    pane,
                    bindings.otty_tab_id.as_deref(),
                    bindings.workspace_path.as_deref(),
                );
                if !ok && super::otty::should_clear_otty_binding_after_focus_fail(pane) {
                    clear_stale_otty_binding(store, session_id, pane);
                }
                ok
            } else {
                false
            }
        } else {
            false
        };

    // tmux:最低优先级 binding(brief 20260804-tmux-binding)。best-effort
    // `tmux select-pane` 只切 tmux 内部活动 pane,失败静默;**不 gate ladder**:
    // tmux 没有 pane 级 GUI 定位,外层 tab 激活仍走 tty/terminal_id ladder
    // (Ghostty capture 兜底,见 grok review §2.6)。
    if !did_herdr && !did_cmux && !did_wezterm && !did_orca && !did_otty {
        if let (Some(pane), Some(sock)) = (
            bindings.tmux_pane_id.as_deref(),
            bindings.tmux_socket_path.as_deref(),
        ) {
            let _ = focus_tmux_pane(pane, Some(sock));
        }
    }

    TerminalShortCircuitOutcome {
        did_herdr,
        did_cmux,
        did_wezterm,
        did_orca,
        did_otty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionStore;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[test]
    fn herdr_remote_short_circuits_skipped_entirely() {
        // brief 20260807-herdr-remote-jumpback-fix:herdr-remote session 无本地宿主,
        // 段 A 本地锚定必须整体短路(否则远程 socket 会误吸本地 Ghostty herdr tab)。
        let dir = tempdir().unwrap();
        let store = Arc::new(Mutex::new(SessionStore::load(dir.path().join("s.json"))));
        let bindings = SessionFocusBindings {
            workspace_path: Some("/Users/remote".into()),
            herdr_pane_id: Some("w1:p1".into()),
            herdr_socket_path: Some("/tmp/definitely-not-a-socket.sock".into()),
            is_herdr_remote: true,
            ..Default::default()
        };
        let out = run_terminal_short_circuits(&store, "sess-x", &bindings);
        assert_eq!(out, TerminalShortCircuitOutcome::default());
    }
}
