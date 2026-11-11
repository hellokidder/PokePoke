//! WezTerm surface short-circuit:CC 跑在 WezTerm pane 里时直接 CLI 聚焦。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! `focus_wezterm_surface`(`wezterm cli activate-pane`)+ `clear_stale_wezterm_binding`
//! (focus 失败清 stale binding)。

use std::sync::{Arc, Mutex};

use crate::sessions::SessionStore;

/// wezterm short-circuit:CC 直接跑在 WezTerm pane 里时,session 存的
/// `wezterm_pane_id` + `wezterm_unix_socket` 喂给 `wezterm cli activate-pane`。
pub(super) fn focus_wezterm_surface(
    pane_id: &str,
    unix_socket: &str,
    executable: Option<&str>,
    workspace_path: Option<&str>,
    terminal_tty: Option<&str>,
) -> bool {
    crate::wezterm::focus_pane(
        pane_id,
        unix_socket,
        executable,
        workspace_path,
        terminal_tty,
    )
}

pub(super) fn clear_stale_wezterm_binding(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    pane_id: &str,
) {
    let cleared = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear_surface_binding_if_matches(
            session_id,
            &crate::surface::SurfaceBinding::Wezterm {
                pane_id: pane_id.to_string(),
                unix_socket: None,
                executable: None,
            },
        );
    crate::log_info!(
        "focus",
        "cleared stale wezterm_binding session_id={} pane_id={} cleared={}",
        session_id,
        pane_id,
        cleared
    );
}
