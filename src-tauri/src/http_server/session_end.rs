//! SessionEnd 事件处理:删 session + 关 popup + 清 acked stage。从
//! `http_server.rs` 抽出(brief 20260727-split-http-server-rs 第二阶段),行为零变化。

use super::notify::{parse_event_type, NotifyEventType, NotifyRequest};
use super::AppState;
use crate::{log_debug, log_info};
use axum::http::StatusCode;

/// SessionEnd 事件:从 store 删 session,关掉对应 popup,清 acked stage。
/// 返回 `Some((status, body))` 表示已处理(调用方直接 return);`None` 表示
/// 非 SessionEnd 事件,继续走 upsert + popup 决策主流程。
pub(super) fn handle_session_end(
    state: &AppState,
    req: &NotifyRequest,
) -> Option<(StatusCode, serde_json::Value)> {
    if parse_event_type(req.event_type.as_deref()) != Some(NotifyEventType::SessionEnd) {
        return None;
    }
    let removed_id = {
        let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.remove_session_by_task_id(&req.task_id).map(|s| s.id)
    };

    if let Some(session_id) = removed_id.as_ref() {
        log_info!(
            "notify",
            "session_end -> close popup task_id={} session_id={}",
            req.task_id,
            session_id,
        );
        state
            .popup
            .close(session_id, crate::popup::PopupCloseReason::SessionEnd);
        state.memory.clear_acked(session_id);
        state.events.sessions_updated();
    } else {
        log_debug!(
            "notify",
            "session_end -> no session to remove task_id={}",
            req.task_id,
        );
    }

    Some((
        StatusCode::OK,
        serde_json::json!({
            "removed": removed_id.is_some(),
            "task_id": req.task_id,
        }),
    ))
}
