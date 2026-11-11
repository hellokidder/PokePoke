//! http_server:UDS HTTP server 与 `/notify` 处理入口。
//!
//! 职责拆分(brief 20260727-split-http-server-rs):
//! - 本文件:server bootstrap(bind_or_detect_peer / serve_with_listener / start)+
//!   axum handler(handle_notify / handle_list)+ 响应序列化(to_value_or_internal_error)
//!   + 子 mod 声明与重导出。
//! - `state.rs`:进程内共享状态(AppState / DedupeWindow / cooldown / acked 等)。
//! - `notify.rs`:`/notify` 请求模型 + 解析 + process_notify 编排 + 终端 binding 落库。
//! - `capture.rs`:ghostty / wezterm capture 决策与 spawn。
//! - `popup_gate.rs`:popup 触发决策与 show/sound 副作用。
//! - `session_end.rs`:SessionEnd 事件处理。
//! - `title.rs`:codex 桌面端 thread title 解析。
//! - `license_daily.rs`:license 每日配额校验 + 例行验证。

use crate::sessions::Session;
use crate::{log_error, log_info, log_warn, transport};
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};

mod capture;
mod decision_trace;
mod notify;
mod popup_gate;
mod session_end;
mod staging;
mod state;
mod title;
// 状态层(NotificationGate/EventEmitter/AppState/RecentlyVisited/AckedStages/
// PopupCooldown/DedupeWindow + DEDUPE_WINDOW 常量)抽到 state.rs,这里重导出
// 保持外部 `crate::http_server::AppState` 等引用不断。
pub use state::*;
// notify.rs 的对外面:外部(herdr_remote / transcript_watch / 集成测试)经
// `crate::http_server::{process_notify, NotifyRequest}` 引用,这里重导出。
pub use notify::{process_notify, NotifyRequest};
// staging:popup 呈现派发(响应路径与 gate 解耦,见模块注释)
pub use staging::StagingDispatcher;
// capture.rs 经 `super::NotifyEventType` 引用(拆分前在同一文件,拆分后转发)。
use notify::NotifyEventType;

#[cfg(test)]
#[path = "http_server/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "http_server/tests_capture.rs"]
mod tests_capture;
#[cfg(test)]
#[path = "http_server/tests_popup.rs"]
mod tests_popup;
#[cfg(test)]
#[path = "http_server/tests_staging.rs"]
mod tests_staging;
#[cfg(test)]
#[path = "http_server/tests_state.rs"]
mod tests_state;

/// 同步版本的绑定 + 单实例判定,setup 阶段调用。
///
/// - 成功 bind → `Some(listener)`,调用方随后 spawn `serve_with_listener` 跑 accept。
/// - 对等实例已活 → `None`,调用方**必须** `app.exit(0)` 立刻退出主进程,
///   不能继续进 setup 后续步骤,否则半活实例会去写共享 `sessions.json` /
///   `settings.json`(review H4)。
/// - bind 硬错误 → `None` + log(视为 fatal;不 panic,让 GUI 至少能起来告警)。
///
/// v1.0.1 之前 `start()` 里同时做 bind + accept,BindOutcome::PeerAlive 只
/// 让 async task return,主进程仍全程启动 → 违反 §9b「第二实例端口被占即退出」
/// 硬边界。修法:把 bind 提到 setup 同步跑,PeerAlive 观察到就 exit。
pub fn bind_or_detect_peer() -> Option<std::os::unix::net::UnixListener> {
    let pokepoke_dir = transport::default_pokepoke_dir();
    let sock_path = transport::sock_path(&pokepoke_dir);
    match transport::bind_unix(&pokepoke_dir) {
        Ok(transport::BindOutcome::Bound(l)) => Some(l),
        Ok(transport::BindOutcome::PeerAlive) => {
            log_warn!(
                "transport",
                "UDS at {} already owned by another PokePoke instance — will exit main process",
                sock_path.display()
            );
            None
        }
        Err(e) => {
            log_error!(
                "transport",
                "UDS bind failed at {}: {}",
                sock_path.display(),
                e
            );
            None
        }
    }
}

/// 用已绑定的 std UnixListener 起 axum accept 循环。async,由调用方 spawn。
pub async fn serve_with_listener(state: AppState, std_listener: std::os::unix::net::UnixListener) {
    let router = Router::new()
        .route("/notify", post(handle_notify))
        .route("/sessions", get(handle_list))
        .with_state(state);

    let listener = match tokio::net::UnixListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            log_error!("transport", "UDS listener tokio conversion failed: {}", e);
            return;
        }
    };

    log_info!("transport", "UDS accept loop started");
    let _ = axum::serve(listener, router).await;
}

/// 启动 HTTP server,绑定固定路径 UDS(`<pokepoke_dir>/sock`)。
/// 单实例语义:若已有对等实例 → log + return,主进程退出由调用方决定。
///
/// 由 `docs/briefs/20260423-uds-fixed-path.md` 把 §3.1 的 TCP localhost
/// 修正为极简 UDS;多实例发现表 + per-pid sock 等 §3.1 让出项不变。
///
/// v1.0.1 review H4 之后新的调用方式是 `bind_or_detect_peer()` + `serve_with_listener()`
/// 两步。这个 `start()` 保留给测试 / 老路径兼容,setup 侧改用两步方案。
pub async fn start(state: AppState) {
    let Some(listener) = bind_or_detect_peer() else {
        return;
    };
    serve_with_listener(state, listener).await;
}

async fn handle_notify(
    State(state): State<AppState>,
    Json(req): Json<NotifyRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let (code, body) = process_notify(&state, req);
    (code, Json(body))
}

/// 序列化 session 为 JSON,失败时记录 error 并返回 500。
/// is_new=true → CREATED, else OK。
/// 避免 serde_json::to_value unwrap 导致 handler panic 切断响应。
fn to_value_or_internal_error(
    session: &Session,
    task_id: &str,
    is_new: bool,
) -> (StatusCode, serde_json::Value) {
    let code = if is_new {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    match serde_json::to_value(session) {
        Ok(val) => (code, val),
        Err(e) => {
            log_error!(
                "notify",
                "serialize session FAILED task_id={} err={}",
                task_id,
                e,
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({"error": "serialization_failed"}),
            )
        }
    }
}

async fn handle_list(State(state): State<AppState>) -> Json<Vec<Session>> {
    let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
    Json(store.get_all().to_vec())
}
