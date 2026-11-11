//! cmux surface short-circuit:CC 跑在 cmux pane 里时直接 CLI 聚焦。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! `focus_cmux_surface`(`cmux focus-panel` shell-out)+ `clear_stale_cmux_binding`
//! (focus 失败清 stale binding,让下次 SessionStart 重上报)。

use std::sync::{Arc, Mutex};

use crate::sessions::SessionStore;

/// cmux short-circuit:CC 跑在 cmux pane 里时,session 里存的 `cmux_surface_id`
/// 就是 pane 稳定 UUID,直接 shell-out
/// `cmux focus-panel --panel <surface_id> --workspace <workspace_id>` 精准聚焦。
///
/// 优先级放在 target ladder 最前 -- cmux 是外层 macOS App(`com.cmuxterm.app`,
/// 用 libghostty 渲染),ladder 里 iTerm2 / Terminal.app / Ghostty AppleScript
/// 枚举全都看不到 cmux 内部 pty,只有 cmux CLI 自己认得。跟 herdr short-circuit
/// 同型,不进 `decide_focus_target`。
///
/// 为什么必须显式带 `--workspace`(2026-07-14 修):cmux CLI 默认从调用方进程
/// env 读 `$CMUX_WORKSPACE_ID`,PokePoke server 是 `pnpm dev` 起的,进程 env
/// 里没这个变量;跨 workspace 的 panel UUID 会被 cmux 报 `not_found: Surface
/// not found` -- 观感就是"panel 明明还在 cmux 里但跳歪走 ladder 兜底"。
/// hook 上报时把 workspace_id 一起写进 session,这里显式传参消除歧义。
///
/// stale 处理:cmux 未运行 / surface 已关(用户手动关 pane)时 CLI 会失败,
/// caller 需调 `clear_surface_binding_if_matches(Cmux{..})` 清 binding,下次 SessionStart
/// 会重上报新 surface_id。
///
/// cmux 未装:`Command::new("cmux")` 直接 `Err` -> 返 false -> 走原 ladder。
/// fail-safe:不带 cmux binding 的 session 永远不会走到这里。
/// dev 模式注意: `pnpm install:hook` 只编 release pokehook, 但 `tauri dev`
/// 的 ensure_hook_deployed 从 target/debug/pokehook 部署 -- 两者不同步会
/// 导致 dev 时旧 debug hook 覆盖新 release hook. 改 hook 后 dev 必须也跑
/// 一次 `cargo build --bin pokehook`(debug) 同步 target/debug/pokehook.
pub(super) fn focus_cmux_surface(surface_id: &str, workspace_id: Option<&str>) -> bool {
    crate::log_info!(
        "focus",
        "focus_cmux_surface surface_id={} workspace_id={}",
        surface_id,
        workspace_id.unwrap_or("<none>"),
    );
    crate::cmux::focus_panel(surface_id, workspace_id)
}

/// caller 拿到 `focus_cmux_surface` false 时的收尾:清 stale binding,
/// 让下次 SessionStart 重上报的 surface_id 能落库。
pub(super) fn clear_stale_cmux_binding(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    surface_id: &str,
) {
    let cleared = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear_surface_binding_if_matches(
            session_id,
            &crate::surface::SurfaceBinding::Cmux {
                surface_id: surface_id.to_string(),
                workspace_id: None,
            },
        );
    crate::log_info!(
        "focus",
        "cleared stale cmux_binding session_id={} surface_id={} cleared={}",
        session_id,
        surface_id,
        cleared
    );
}
