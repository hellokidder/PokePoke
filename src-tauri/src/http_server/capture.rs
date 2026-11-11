//! ghostty / wezterm capture 决策与 spawn。从 `http_server.rs` 抽出(brief
//! 20260727-split-http-server-rs 第一阶段),行为零变化。
//!
//! capture 是「hook 丢 terminal id 时的补抓」:SessionStart 首次 + retry
//! (UserPromptSubmit/Pending/Stop) 各节奏都走这里,决策条件逐行保持。

use super::{AppState, NotifyEventType};
use crate::log_info;
use crate::sessions::Session;

pub(super) const GHOSTTY_OPPORTUNISTIC_CLI_SOURCES: &[&str] = &[
    "claude-code",
    "codex",
    "opencode",
    "kimi",
    "grok",
    "pi",
    "omp",
];
pub(super) fn ghostty_capture_eligible(
    source: Option<&str>,
    terminal_app: Option<&str>,
    workspace_path: Option<&str>,
) -> bool {
    let workspace_nonempty = workspace_path.is_some_and(|p| !p.is_empty());
    match terminal_app {
        // hook 明确上报 ghostty:cc/codex/cursor 一视同仁放行。cursor CLI 跑在
        // Ghostty 终端时 `TERM=xterm-ghostty` 常驻 env → `terminal_app()` 如实报
        // ghostty(与 source 无关),此时它和 cc/codex 一样在真实终端窗口里,
        // capture 抓 `front window` id 有效。旧逻辑 `source != cursor` 一刀切排除,
        // 是沿用 cursor=Cursor.app 内置终端的旧假设(那种场景 terminal_app 走下面
        // None 分支,仍被拒),对 CLI-in-Ghostty 用法误伤 → terminal_id 永 None →
        // 点击跳转退 cwd L2,同目录多窗口跳错。见 20260529 brief 的假设已失效。
        Some("ghostty") => workspace_nonempty,
        Some(t) if !t.is_empty() => false,
        // terminal_app 缺失:仅白名单 CLI opportunistic 兜底(resume/compact 丢 env,含 pi —— brief 20260804 §4-⑤)。
        // cursor 在此分支仍拒 —— 那是内置终端 / 空 workspace 的噪声场景,capture 只会 REJECT。
        _ => {
            workspace_nonempty
                && source.is_some_and(|s| GHOSTTY_OPPORTUNISTIC_CLI_SOURCES.contains(&s))
        }
    }
}

pub(super) fn ghostty_capture_rejection_reason(
    captured_cwd: &str,
    session_workspace: &str,
) -> Option<&'static str> {
    if captured_cwd.trim().is_empty() {
        return Some("captured_cwd_empty");
    }
    if session_workspace.trim().is_empty() {
        return Some("session_workspace_empty");
    }
    if crate::commands::cwds_compatible(captured_cwd, session_workspace) {
        None
    } else {
        Some("cwd_incompatible")
    }
}

/// capture 是否该拒写 id。纯函数,抽出来为单测。
///
/// `reason` 是 capture 触发原因(`"session_start"` / `"retry-*"`)。2026-06-25
/// 修订:Ghostty shell integration 未上报 OSC7 时 `working directory` 整体不可用,
/// `captured_cwd_empty` 不再可靠代表"抓错 tab"。retry 场景(agent 活跃,前台大概率
/// 是 cc tab)放行落库 id;session_start(cc 刚启动,前台未必是 cc tab)仍拒,宁可
/// 不放也不绑错。`cwd_incompatible` 一律拒 —— 那是 cwd 有值但指向别的 workspace,
/// 明确说明前台是别的 tab,与 cwd 是否可用无关。
pub(super) fn should_reject_capture(reject_reason: Option<&'static str>, reason: &str) -> bool {
    match reject_reason {
        Some("captured_cwd_empty") if reason != "session_start" => false,
        Some(_) => true,
        None => false,
    }
}

pub(super) fn wezterm_capture_eligible(source: Option<&str>, workspace_path: Option<&str>) -> bool {
    workspace_path.is_some_and(|p| !p.is_empty())
        && matches!(
            source,
            Some("claude-code")
                | Some("codex")
                | Some("opencode")
                | Some("pi")
                | Some("omp")
                | Some("cursor")
                | Some("kimi")
                | Some("grok")
        )
}

pub(super) fn wezterm_socket_for_capture(
    req_socket: Option<&str>,
    session_socket: Option<&str>,
) -> Option<String> {
    req_socket
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| session_socket.filter(|s| !s.is_empty()).map(str::to_string))
}

/// WezTerm pane 补 capture:hook 丢了 `WEZTERM_PANE` 但 session 仍持有 socket +
/// workspace 时,用 `wezterm cli list` cwd 唯一匹配补绑。纯决策,无副作用。
pub(super) fn decide_wezterm_capture_reason(
    session: &Session,
    req_herdr_pane_id: Option<&str>,
    req_cmux_surface_id: Option<&str>,
    req_wezterm_socket: Option<&str>,
    is_new: bool,
    event_type: Option<NotifyEventType>,
) -> Option<&'static str> {
    use crate::surface::SurfaceBinding as B;
    if req_herdr_pane_id.is_some()
        || req_cmux_surface_id.is_some()
        || B::herdr_fields(session).is_some()
        || matches!(session.surface, B::Cmux { .. })
    {
        return None;
    }
    if matches!(session.surface, B::Wezterm { .. }) {
        return None;
    }
    let stored_socket = match &session.surface {
        B::Wezterm { unix_socket, .. } => unix_socket.as_deref(),
        _ => None,
    };
    let _socket = wezterm_socket_for_capture(req_wezterm_socket, stored_socket)?;
    if !wezterm_capture_eligible(session.source.as_deref(), session.workspace_path.as_deref()) {
        return None;
    }
    let retry_eligible = matches!(
        event_type,
        Some(NotifyEventType::Running)
            | Some(NotifyEventType::Pending)
            | Some(NotifyEventType::Stop)
    );
    if !(is_new || retry_eligible) {
        return None;
    }
    if is_new {
        Some("session_start")
    } else {
        Some(match event_type {
            Some(NotifyEventType::Running) => "retry-user_prompt_submit",
            Some(NotifyEventType::Pending) => "retry-notification",
            Some(NotifyEventType::Stop) => "retry-stop",
            _ => "retry-unknown",
        })
    }
}

/// brief 20260724-wezterm Phase 2:server 侧 cwd 补 capture,镜像 ghostty retry 节奏。
pub(super) fn spawn_wezterm_capture(
    state: &AppState,
    session: &Session,
    req_wezterm_socket: Option<&str>,
    req_wezterm_executable: Option<&str>,
    reason: &'static str,
) {
    use crate::surface::SurfaceBinding as B;
    let (stored_socket, stored_executable) = match &session.surface {
        B::Wezterm {
            unix_socket,
            executable,
            ..
        } => (unix_socket.as_deref(), executable.as_deref()),
        _ => (None, None),
    };
    let socket = match wezterm_socket_for_capture(req_wezterm_socket, stored_socket) {
        Some(s) => s,
        None => return,
    };
    let executable = req_wezterm_executable
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| stored_executable.map(str::to_string));
    let workspace = match session.workspace_path.as_ref().filter(|w| !w.is_empty()) {
        Some(w) => w.clone(),
        None => return,
    };
    let task_id = session.task_id.clone();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let store = state.store.clone();
        log_info!(
            "wezterm",
            "capture spawning task_id={} reason={}",
            task_id,
            reason
        );
        handle.spawn(async move {
            let socket_for_capture = socket.clone();
            let executable_for_capture = executable.clone();
            let captured = tokio::task::spawn_blocking(move || {
                crate::wezterm::capture_pane_by_workspace(
                    &socket_for_capture,
                    executable_for_capture.as_deref(),
                    &workspace,
                )
            })
            .await
            .ok()
            .flatten();
            match captured {
                Some(pane_id) => {
                    let written = {
                        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                        use crate::surface::SurfaceBinding as B;
                        if store
                            .sessions
                            .iter()
                            .find(|s| s.task_id == task_id)
                            .is_some_and(|s| matches!(s.surface, B::Wezterm { .. }))
                        {
                            false
                        } else {
                            // W5(brief 20260813-surface-binding):写侧切到
                            // set_surface_binding;外层平铺预检(不覆盖已有)保留,
                            // 镜像写入与旧 set_wezterm_binding 逐字节等价。
                            store.set_surface_binding(
                                &task_id,
                                crate::surface::SurfaceBinding::Wezterm {
                                    pane_id: pane_id.clone(),
                                    unix_socket: Some(socket.clone()),
                                    executable: executable.clone(),
                                },
                            )
                        }
                    };
                    log_info!(
                        "wezterm",
                        "captured pane_id task_id={} reason={} pane_id={} written={}",
                        task_id,
                        reason,
                        pane_id,
                        written,
                    );
                }
                None => {
                    log_info!(
                        "wezterm",
                        "pane capture returned None task_id={} reason={} (not running / cwd ambiguous / CLI failed)",
                        task_id,
                        reason,
                    );
                }
            }
        });
    } else {
        log_info!(
            "wezterm",
            "capture skipped (no tokio runtime) task_id={} reason={}",
            task_id,
            reason,
        );
    }
}

/// brief 20260527-ghostty-capture-retry:从 b20520 的 SessionStart-only 路径
/// 抽出来的共享 capture spawner,SessionStart 首次和 UserPromptSubmit/Pending/Stop
/// 上的 retry 都调它。`reason` 串放进 log 便于真机 smoke 时分桶统计命中率。
pub(super) fn spawn_ghostty_capture(
    state: &AppState,
    session: &crate::sessions::Session,
    reason: &'static str,
) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let store = state.store.clone();
        let task_id = session.task_id.clone();
        // 0.0.5+:cwd 校验需要 session.workspace_path,提前 clone
        let session_workspace = session.workspace_path.clone();
        log_info!(
            "ghostty",
            "capture spawning task_id={} reason={}",
            task_id,
            reason
        );
        handle.spawn(async move {
            // 枚举匹配需要 session 的 workspace + task_id 透传给 capture(brief
            // 20260702-ghostty-capture-enumerate):不再赌 front window,按身份精确挑窗口。
            let cap_workspace = session_workspace.clone();
            let cap_task_id = Some(task_id.clone());
            let captured = tokio::task::spawn_blocking(move || {
                crate::commands::capture_ghostty_focused_terminal_id(cap_workspace, cap_task_id)
            })
            .await
            .ok()
            .flatten();
            match captured {
                Some(c) => {
                    // 0.0.5+ cwd 校验:gate 放宽后(7225a45)opportunistic capture
                    // 可能抓到无关 tab 的 id(cc resume 后台 fire + 用户切走 /
                    // iTerm2 用户 Ghostty 有无关 tab 前置)。拒写"cwd 跟 session
                    // workspace 八竿子打不着"的 id,守住 click-jump L1 不会跳错
                    // tab + is_focused 不会误 suppress 别处。
                    let ws = session_workspace.as_deref().unwrap_or("");
                    let reject_reason = ghostty_capture_rejection_reason(&c.cwd, ws);
                    let reject = should_reject_capture(reject_reason, reason);
                    if reject {
                        log_info!(
                            "ghostty",
                            "captured id REJECTED task_id={} reason={} reject_reason={} captured_cwd={} session_workspace={:?} (不落库,避免绑定到错误 Ghostty tab)",
                            task_id, reason, reject_reason.unwrap_or(""), c.cwd, session_workspace,
                        );
                        return;
                    }
                    let written = {
                        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                        store.set_terminal_id_if_unset(&task_id, c.id.clone(), c.cwd.clone())
                    };
                    log_info!(
                        "ghostty",
                        "captured terminal id task_id={} reason={} id={} captured_cwd={} written={}",
                        task_id, reason, c.id, c.cwd, written,
                    );
                }
                None => {
                    log_info!(
                        "ghostty",
                        "terminal id capture returned None task_id={} reason={} (Ghostty not running / sdef missing / TCC denied / front window has no focused terminal)",
                        task_id, reason,
                    );
                }
            }
        });
    } else {
        log_info!(
            "ghostty",
            "capture skipped (no tokio runtime) task_id={} reason={}",
            session.task_id,
            reason,
        );
    }
}

pub(super) fn decide_ghostty_capture_reason(
    session: &Session,
    terminal_app: Option<&str>,
    is_new: bool,
    event_type: Option<NotifyEventType>,
) -> Option<&'static str> {
    let source = session.source.as_deref();
    let capture_eligible =
        ghostty_capture_eligible(source, terminal_app, session.workspace_path.as_deref());
    let retry_eligible_event = matches!(
        event_type,
        Some(NotifyEventType::Running)
            | Some(NotifyEventType::Pending)
            | Some(NotifyEventType::Stop)
    );
    let should_capture =
        capture_eligible && session.terminal_id.is_none() && (is_new || retry_eligible_event);
    if !should_capture {
        return None;
    }
    if is_new {
        Some("session_start")
    } else {
        // 日志加 event_type 便于真机 smoke 时分桶统计 retry 命中率
        Some(match event_type {
            Some(NotifyEventType::Running) => "retry-user_prompt_submit",
            Some(NotifyEventType::Pending) => "retry-notification",
            Some(NotifyEventType::Stop) => "retry-stop",
            _ => "retry-unknown",
        })
    }
}
