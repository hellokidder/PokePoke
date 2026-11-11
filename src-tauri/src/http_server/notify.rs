//! `/notify` 请求模型与处理编排。从 `http_server.rs` 抽出(brief
//! 20260727-split-http-server-rs 第二阶段),行为零变化。
//!
//! `process_notify` 是 `/notify` 状态流转的编排入口:dedupe → session_end →
//! upsert → 终端 binding 落库 → capture 触发 → popup gate。解析与绑定逻辑在
//! 本模块,capture / popup_gate / session_end / title 各阶段在兄弟模块。

use super::capture::{
    decide_ghostty_capture_reason, decide_wezterm_capture_reason, spawn_ghostty_capture,
    spawn_wezterm_capture,
};
use super::decision_trace::NotifyDecisionTrace;
use super::session_end::handle_session_end;
use super::state::DEDUPE_WINDOW;
use super::title::resolve_title_for_notify;
use super::{to_value_or_internal_error, AppState};
use crate::codex_appserver::is_codex_desktop_session;
use crate::commands::focus::{
    apply_orca_binding, apply_otty_binding, decide_orca_capture_reason, decide_otty_capture_reason,
    spawn_orca_capture, spawn_otty_capture,
};
use crate::commands::integration::is_notify_source_disconnected;
use crate::log_debug;
use crate::log_info;
use crate::popup::PopupCloseReason;
use crate::popup_decision::should_close_popup_on_running;
use crate::sessions::{Priority, SessionStatus, UpsertRequest, UpsertResult};
use axum::http::StatusCode;
use serde::Deserialize;

/// /notify wire 协议版本(W9,brief 20260813-protocol-version)。缺省(不传)= v1。
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// 兼容检查(纯函数,单测友好):不传(None)或 <= 当前版本 → true;
/// 版本超前(>1,未来版本 hook 打旧 server)→ false,调用方 log_warn 一次
/// 后**照常处理**(检测点不是拦截点)。
pub(super) fn protocol_compatible(version: Option<u32>) -> bool {
    version.is_none_or(|v| v <= PROTOCOL_VERSION)
}

/// 提取去重 signature。100ms 内同 task_id + 同 signature → short-circuit(§3.2)。
///
/// signature = `event_type + status`(全 lowercase),缺失字段自动省略;两者都空时
/// 退化为 `"default"`(兜底,几乎不会命中,upstream 会填至少一个)。
///
/// v1.0.1 review M1:此前只取 `event_type`,`stop/idle` 与 `stop/last_failed` 得到
/// 同一 signature。第三方 wrapper 转发 hook 时 100ms 内先后送出 Stop 再 StopFailure,
/// 真实的 API 失败状态(last_failed + failure_reason + 弹窗声音 + 外推)会被吞。
/// 把 status 拼进 signature 尾部即可,同 event/同 status 的原去重仍生效。
pub(super) fn dedupe_signature(req: &NotifyRequest) -> String {
    let et = req.event_type.as_deref().unwrap_or("").to_ascii_lowercase();
    let st = req.status.as_deref().unwrap_or("").to_ascii_lowercase();
    match (et.is_empty(), st.is_empty()) {
        (false, false) => format!("{}|{}", et, st),
        (false, true) => et,
        (true, false) => st,
        _ => "default".to_string(),
    }
}
#[derive(Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub message: String,
    pub task_id: String,
    // W9(brief 20260813-protocol-version):wire 协议版本,可选;缺省视为 v1。
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub external_session_id: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub terminal_tty: Option<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    // 配合 status=last_failed 使用；其他状态会被 upsert_session 忽略。
    #[serde(default)]
    pub failure_reason: Option<String>,
    // T1(brief 20260805-t1-cursor-aborted):cursor stop|idle(aborted) 事件标记。
    // upsert_session 据此写 aborted_at;5s 窗口内的 stop|last_failed 是同一轮
    // 取消的 artifact echo(状态保持 idle),窗口内音效抑制(不播"完成"音)。
    // 其余事件(pending/running/正常 stop/error)不携带。
    #[serde(default)]
    pub aborted: Option<bool>,
    // 终端宿主 app(`TERM_PROGRAM` 小写归一化),仅 SessionStart 上报。
    // server 端用 `terminal_app == "ghostty"` 触发 Ghostty terminal id 异步 capture。
    // 其它值(iterm.app / apple_terminal / vscode / 空 / None)直接忽略。
    #[serde(default)]
    pub terminal_app: Option<String>,
    // cc/codex hook 自带的 transcript jsonl 路径;Claude transcript watcher 等读摘要用。
    #[serde(default)]
    pub transcript_path: Option<String>,
    // brief 20260709-herdr-jump-support:CC 跑在 herdr pane 里时 pokehook 从环境
    // 变量采集的三件套。三字段全有才写入 session 的 herdr_binding。
    #[serde(default)]
    pub herdr_pane_id: Option<String>,
    #[serde(default)]
    pub herdr_workspace_id: Option<String>,
    #[serde(default)]
    pub herdr_socket_path: Option<String>,
    // brief 20260709-cmux-jump-support:CC 跑在 cmux pane 里时 pokehook 从 env
    // 采集的两件套(surface_id + workspace_id)。两字段全有才写入 cmux_binding;
    // 跳回时走 short-circuit `cmux focus-panel --panel <surface_id>`。
    #[serde(default)]
    pub cmux_surface_id: Option<String>,
    #[serde(default)]
    pub cmux_workspace_id: Option<String>,
    // brief 20260724-wezterm-jump-support:直接跑在 WezTerm pane 里的 CC。
    #[serde(default)]
    pub wezterm_pane_id: Option<String>,
    #[serde(default)]
    pub wezterm_unix_socket: Option<String>,
    #[serde(default)]
    pub wezterm_executable: Option<String>,
    // brief 20260727-otty-support:直接跑在 Otty pane 里的 CC。
    #[serde(default)]
    pub otty_pane_id: Option<String>,
    // brief 20260804-tmux-binding:直接跑在原生 tmux pane 里的 agent
    // (`TMUX_PANE` + `TMUX` 两件套,pokehook 采集)。两字段全有才写 tmux_binding。
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
    #[serde(default)]
    pub tmux_socket_path: Option<String>,
    // brief 20260728-orca-support:直接跑在 Orca worktree pane 里的 agent。
    // brief 20261104-dsh-integration:DSH host 半插件自建的本地控制通道。
    #[serde(default)]
    pub dsh_nav_socket: Option<String>,
    #[serde(default)]
    pub orca_pane_key: Option<String>,
    #[serde(default)]
    pub orca_worktree_id: Option<String>,
    #[serde(default)]
    pub orca_tab_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum NotifyEventType {
    Running,
    Pending,
    Stop,
    SessionEnd,
}
pub(super) fn parse_event_type(event_type: Option<&str>) -> Option<NotifyEventType> {
    match event_type.map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "running" => Some(NotifyEventType::Running),
        Some(ref s) if s == "pending" => Some(NotifyEventType::Pending),
        Some(ref s) if s == "stop" => Some(NotifyEventType::Stop),
        Some(ref s) if s == "session_end" => Some(NotifyEventType::SessionEnd),
        _ => None,
    }
}

pub(super) fn parse_status(
    status: Option<&str>,
    event_type: Option<&NotifyEventType>,
) -> SessionStatus {
    match status.map(|s| s.to_ascii_lowercase()) {
        Some(ref s) if s == "running" => SessionStatus::Running,
        Some(ref s) if s == "idle" || s == "success" => SessionStatus::Idle,
        Some(ref s) if s == "last_failed" || s == "failure" || s == "failed" => {
            SessionStatus::LastFailed
        }
        None => match event_type {
            Some(NotifyEventType::Running) => SessionStatus::Running,
            Some(NotifyEventType::Pending) => SessionStatus::Pending,
            Some(NotifyEventType::Stop) => SessionStatus::Idle,
            _ => SessionStatus::Pending,
        },
        _ => SessionStatus::Pending,
    }
}

/// upsert move req 之前一次性取出的终端相关字段(binding 落库 + capture 触发共用)。
/// `NotifyRequest` 的部分字段会被 upsert_session move,这里在 move 之前 clone
/// 打包,避免 binding / capture 阶段逐个访问已部分 move 的 req。
struct NotifyPrefetch {
    herdr_pane_id: Option<String>,
    herdr_workspace_id: Option<String>,
    herdr_socket_path: Option<String>,
    orca_pane_key: Option<String>,
    orca_worktree_id: Option<String>,
    orca_tab_id: Option<String>,
    cmux_surface_id: Option<String>,
    cmux_workspace_id: Option<String>,
    wezterm_pane_id: Option<String>,
    wezterm_unix_socket: Option<String>,
    wezterm_executable: Option<String>,
    otty_pane_id: Option<String>,
    tmux_pane_id: Option<String>,
    tmux_socket_path: Option<String>,
    terminal_app: Option<String>,
}

impl NotifyPrefetch {
    /// 提取 binding / capture 所需的终端字段(必须在 upsert move req 之前调用)。
    fn collect(req: &NotifyRequest) -> NotifyPrefetch {
        NotifyPrefetch {
            herdr_pane_id: req.herdr_pane_id.clone(),
            herdr_workspace_id: req.herdr_workspace_id.clone(),
            herdr_socket_path: req.herdr_socket_path.clone(),
            orca_pane_key: req.orca_pane_key.clone(),
            orca_worktree_id: req.orca_worktree_id.clone(),
            orca_tab_id: req.orca_tab_id.clone(),
            cmux_surface_id: req.cmux_surface_id.clone(),
            cmux_workspace_id: req.cmux_workspace_id.clone(),
            wezterm_pane_id: req.wezterm_pane_id.clone(),
            wezterm_unix_socket: req.wezterm_unix_socket.clone(),
            wezterm_executable: req.wezterm_executable.clone(),
            otty_pane_id: req.otty_pane_id.clone(),
            tmux_pane_id: req.tmux_pane_id.clone(),
            tmux_socket_path: req.tmux_socket_path.clone(),
            terminal_app: req.terminal_app.clone(),
        }
    }
}

/// 六类终端 binding 落库:herdr / cmux / wezterm / otty / tmux / orca。
/// 优先级与互斥规则逐行保持原 process_notify 内联实现(行为零变化)。
///
/// 返回值:gate **之前**需要补全的 best-effort 项(herdr host 缺 ORCA_PANE_KEY
/// 时用 Orca CLI 补外层 pane)。那段是两次同步 CLI 调用(实测各 ~110ms),不再在
/// 响应路径上跑,改由 staging 在 gate 之前执行 —— 响应变快,顺序不变。
fn apply_terminal_bindings(
    state: &AppState,
    result: &UpsertResult,
    task_id: &str,
    req_source: Option<String>,
    p: &NotifyPrefetch,
) -> Option<super::staging::OrcaHerdrEnrichment> {
    // brief 20260709-herdr-jump-support:三字段全有才写入 herdr_binding;
    // 缺一律不落 binding(避免半状态干扰跳回分流)。
    if let (Some(pane), Some(ws), Some(sock)) = (
        p.herdr_pane_id.as_deref(),
        p.herdr_workspace_id.as_deref(),
        p.herdr_socket_path.as_deref(),
    ) {
        let changed = {
            let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
            store.set_surface_binding(
                task_id,
                crate::surface::SurfaceBinding::Herdr {
                    pane_id: pane.to_string(),
                    workspace_id: Some(ws.to_string()),
                    socket_path: Some(sock.to_string()),
                },
            )
        };
        if changed {
            crate::log_info!(
                "notify",
                "set_surface_binding herdr task_id={} pane={} workspace={} sock={}",
                task_id,
                pane,
                ws,
                sock
            );
        }
    }

    // brief 20260728-orca-support:herdr host 缺 ORCA_PANE_KEY 时要用 Orca CLI 补
    // 外层 pane —— 那是两次同步 CLI 调用(实测各 ~110ms),不能放在 `/notify` 的
    // 响应路径上。这里只做判定,真正的补全交给 staging 在 gate 之前执行
    // (见 `staging::run_stage` 与 brief 20261013-focus-off-response-path)。
    // 修回历史:2026-10-13 之前此处直接调用,响应被拖慢 ~215ms。
    let orca_herdr_enrichment = if p.herdr_pane_id.is_some()
        && req_source.as_deref() != Some("herdr-remote")
        && p.orca_pane_key.is_none()
    {
        Some(super::staging::OrcaHerdrEnrichment {
            task_id: task_id.to_string(),
            workspace_path: result.session.workspace_path.clone(),
        })
    } else {
        None
    };

    // brief 20260709-cmux-jump-support:两字段全有才写入 cmux_binding。无 herdr
    // binding 时走 short-circuit;有 herdr 时由 herdr 段 A 定位外层宿主。
    if p.herdr_pane_id.is_none() {
        if let (Some(surface), Some(ws)) =
            (p.cmux_surface_id.as_deref(), p.cmux_workspace_id.as_deref())
        {
            let changed = {
                let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
                store.set_surface_binding(
                    task_id,
                    crate::surface::SurfaceBinding::Cmux {
                        surface_id: surface.to_string(),
                        workspace_id: Some(ws.to_string()),
                    },
                )
            };
            if changed {
                crate::log_info!(
                    "notify",
                    "set_surface_binding cmux task_id={} surface_id={} workspace_id={}",
                    task_id,
                    surface,
                    ws
                );
            }
        } else if let (Some(pane), Some(sock)) = (
            p.wezterm_pane_id.as_deref(),
            p.wezterm_unix_socket.as_deref(),
        ) {
            // brief 20260724-wezterm:herdr / cmux 优先;仅直接跑在 WezTerm 里才写 binding。
            let executable = p.wezterm_executable.clone();
            let changed = {
                let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
                store.set_surface_binding(
                    task_id,
                    crate::surface::SurfaceBinding::Wezterm {
                        pane_id: pane.to_string(),
                        unix_socket: Some(sock.to_string()),
                        executable,
                    },
                )
            };
            if changed {
                crate::log_info!(
                    "notify",
                    "set_surface_binding wezterm task_id={} pane_id={} socket={}",
                    task_id,
                    pane,
                    sock
                );
            }
        } else if let Some(pane) = p.otty_pane_id.as_deref() {
            // brief 20260727-otty:仅直接跑在 Otty 里才写 binding。
            apply_otty_binding(&state.store, task_id, pane);
        } else if let (Some(pane), Some(sock)) =
            (p.tmux_pane_id.as_deref(), p.tmux_socket_path.as_deref())
        {
            // brief 20260804-tmux-binding:仅直接跑在原生 tmux 里才写 binding
            // (优先级最低,herdr/cmux/wezterm/otty 任一命中不采)。
            let changed = {
                let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
                store.set_surface_binding(
                    task_id,
                    crate::surface::SurfaceBinding::Tmux {
                        pane_id: pane.to_string(),
                        socket_path: Some(sock.to_string()),
                    },
                )
            };
            if changed {
                crate::log_info!(
                    "notify",
                    "set_surface_binding tmux task_id={} pane_id={} socket={}",
                    task_id,
                    pane,
                    sock
                );
            }
        }
    }

    // Orca host hint:与 herdr binding 并存(herdr-in-Orca 段 A 用),不走 herdr 互斥链。
    // worktree_id 为空时不写 binding:hook 仅在非空时上报;空串会打破幂等守卫导致每次 notify 写盘。
    if let (Some(pane_key), Some(worktree_id)) = (
        p.orca_pane_key.as_deref(),
        p.orca_worktree_id
            .as_deref()
            .filter(|worktree_id| !worktree_id.is_empty()),
    ) {
        apply_orca_binding(
            &state.store,
            task_id,
            pane_key,
            worktree_id,
            p.orca_tab_id.clone(),
        );
    }

    orca_herdr_enrichment
}

/// capture 触发决策 + spawn:ghostty / wezterm / otty / orca 四类。
/// 各 decide_* 的 eligible 条件逐行保持原 process_notify 内联实现(行为零变化)。
fn run_capture_phases(
    state: &AppState,
    result: &UpsertResult,
    event_type: Option<NotifyEventType>,
    p: &NotifyPrefetch,
) {
    // Ghostty terminal id 异步 capture(brief 20260520-ghostty-terminal-focus):
    // 仅 SessionStart(hook 端只在 SessionStart 上报 terminal_app)+ 新 session +
    // 还没 capture 过 时触发。osascript 是 blocking I/O,丢 spawn_blocking 跑,
    // 不阻塞 /notify response。试探失败(Ghostty 没在跑 / 没 sdef / TCC 拒绝 /
    // 用户瞬间切走 tab)都让 session.terminal_id 保持 None,focus 时走 cwd L2 兜底。
    // diag(可临时观测):/notify 收到的 terminal_app 一律落 info,定位
    // "Ghostty session 但 terminal_app 不是 ghostty" 之类的 schema 不一致
    if result.is_new {
        log_info!(
            "ghostty",
            "diag: is_new task_id={} terminal_app={:?} terminal_id={:?}",
            result.session.task_id,
            p.terminal_app,
            result.session.terminal_id,
        );
    }
    // Ghostty terminal id capture 触发决策(详见 capture::decide_ghostty_capture_reason)。
    if let Some(reason) = decide_ghostty_capture_reason(
        &result.session,
        p.terminal_app.as_deref(),
        result.is_new,
        event_type,
    ) {
        spawn_ghostty_capture(state, &result.session, reason);
    }

    if let Some(reason) = decide_wezterm_capture_reason(
        &result.session,
        p.herdr_pane_id.as_deref(),
        p.cmux_surface_id.as_deref(),
        p.wezterm_unix_socket.as_deref(),
        result.is_new,
        event_type,
    ) {
        spawn_wezterm_capture(
            state,
            &result.session,
            p.wezterm_unix_socket.as_deref(),
            p.wezterm_executable.as_deref(),
            reason,
        );
    }

    // Otty capture:herdr/cmux/wezterm 都不存在 + otty binding 缺失或只有
    // pane_id(缺 tab_id) → spawn blocking `otty pane list --json` 做 cwd
    // 唯一匹配补全 tab_id。比 Ghostty reconcile 简单得多:一次 CLI 调用拿到全
    // 量 pane 的 cwd/id,不需要 AppleScript enumerate。
    let otty_retry = matches!(
        event_type,
        Some(NotifyEventType::Running)
            | Some(NotifyEventType::Pending)
            | Some(NotifyEventType::Stop)
    );
    if let Some(reason) = decide_otty_capture_reason(
        &result.session,
        p.herdr_pane_id.as_deref(),
        p.cmux_surface_id.as_deref(),
        p.wezterm_pane_id.as_deref(),
        p.orca_pane_key.as_deref(),
        result.is_new,
        otty_retry,
    ) {
        spawn_otty_capture(&state.store, &result.session, reason);
    }

    let orca_retry = matches!(
        event_type,
        Some(NotifyEventType::Running)
            | Some(NotifyEventType::Pending)
            | Some(NotifyEventType::Stop)
    );
    if let Some(reason) = decide_orca_capture_reason(
        &result.session,
        p.herdr_pane_id.as_deref(),
        p.cmux_surface_id.as_deref(),
        p.wezterm_pane_id.as_deref(),
        p.otty_pane_id.as_deref(),
        result.is_new,
        orca_retry,
    ) {
        spawn_orca_capture(&state.store, &result.session, reason);
    }
}

/// 用户重新发起一轮(Running)时的收尾:关残留 popup + 清 acked stage +
/// 失效 codex thread 快照(原 process_notify 尾部内联块,行为零变化)。
fn handle_running_transition(state: &AppState, result: &UpsertResult) {
    // popup 关闭:用户重新发起了一轮(状态切回 Running),把这条 session 上
    // 残留的 popup 关掉。触发条件是"上一轮 stage-ending 状态 → Running":
    //   Idle / LastFailed / Pending → Running
    // 语义:上一轮结束后弹的 popup 已经达成提醒使命,新一轮开始时顺手收掉。
    let close_popup_now = should_close_popup_on_running(
        result.is_new,
        &result.session.status,
        result.prev_status.as_ref(),
    );

    if close_popup_now {
        log_info!(
            "notify",
            "stage-end -> Running, close popup task_id={} session_id={} prev={:?}",
            result.session.task_id,
            result.session.id,
            result.prev_status,
        );
        state
            .popup
            .close(&result.session.id, PopupCloseReason::StageEndRunning);
    }

    // 进入 Running 视为"用户开了新一轮活",清掉之前认领的 stage,让下一次
    // Idle / LastFailed / Pending 能正常弹。覆盖两类场景:
    //   - Pending → Running(用户回到终端给了输入,close_popup_now 也命中)
    //   - Idle → Running(agent 自动续上,close_popup_now 也命中)
    //   - 新 session 直接 Running(is_new=true,close 不动,但也要清旧残留)
    // 注意 close_popup_now 里 Running → Running 不命中(prev=Running),但那
    // 种场景下 ack 本来就该是空的,clear no-op 无害。
    if result.session.status == SessionStatus::Running {
        state.memory.clear_acked(&result.session.id);
        if is_codex_desktop_session(&result.session) {
            if let Some(thread_id) = result.session.external_session_id.as_deref() {
                // 新一轮 Running:resume 快照 idle 已过期,等下次 status/changed。
                state.codex_thread_status.clear(thread_id);
            }
        }
    }
}

/// 入口前置守卫:集成断开 source 拒绝 + 100ms (task_id, signature) 去重
/// short-circuit。返回 `Some((status, body))` 表示该事件已被吞掉(调用方直接
/// return);`None` 表示继续主流程。顺序与原 process_notify 内联实现一致。
fn early_reject(
    state: &AppState,
    req: &NotifyRequest,
    signature: &str,
    trace: &mut NotifyDecisionTrace,
) -> Option<(StatusCode, serde_json::Value)> {
    if is_notify_source_disconnected(&state.settings, req.source.as_deref()) {
        log_debug!(
            "notify",
            "ignored task_id={} source={:?} (integration disconnected)",
            req.task_id,
            req.source,
        );
        trace.integration_disconnected = true;
        return Some((
            StatusCode::OK,
            serde_json::json!({
                "ignored": true,
                "reason": "integration_disconnected",
                "task_id": req.task_id,
            }),
        ));
    }

    // 100ms (task_id, signature) 内存去重窗口(scope-audit §3.2 + §8.2)。
    if state
        .memory
        .dedupe_is_duplicate(&req.task_id, signature, DEDUPE_WINDOW)
    {
        log_debug!(
            "notify",
            "dedupe hit task_id={} signature={}",
            req.task_id,
            signature,
        );
        trace.dedupe_duplicate = true;
        return Some((
            StatusCode::OK,
            serde_json::json!({
                "deduped": true,
                "task_id": req.task_id,
            }),
        ));
    }
    None
}

pub fn process_notify(state: &AppState, req: NotifyRequest) -> (StatusCode, serde_json::Value) {
    let priority = match req.priority.as_deref() {
        Some("high") => Priority::High,
        _ => Priority::Normal,
    };
    let event_type = parse_event_type(req.event_type.as_deref());

    // dev_mode 诊断(默认关):每次 /notify 入口落一行,还原 popup show/close 决策序列。
    log_debug!(
        "notify",
        "in task_id={} source={:?} event_type={:?} status={:?}",
        req.task_id,
        req.source,
        req.event_type,
        req.status,
    );

    // W9(brief 20260813-protocol-version):版本超前(未来 hook 打旧 server)→
    // 记 WARN 一次,照常处理(不拒收)。
    if !protocol_compatible(req.protocol_version) {
        crate::log_warn!(
            "notify",
            "protocol_version ahead task_id={} source={:?} version={:?} server_version={} (兼容处理,不拒收)",
            req.task_id,
            req.source,
            req.protocol_version,
            PROTOCOL_VERSION,
        );
    }

    // W2(brief 20260813-decision-trace):每条 /notify 产出一条结构化决策记录。
    // gate 段由 evaluate_and_show_popup 填入;本函数**每个 return 点**都要
    // finish() 落盘(漏 return 点 = 漏记录)。观测失败静默,不影响主链路。
    let mut trace = NotifyDecisionTrace::new(&req);

    // 集成断开 / 100ms 去重:命中直接吞掉,不进入状态流转。
    let signature = dedupe_signature(&req);
    if let Some(resp) = early_reject(state, &req, &signature, &mut trace) {
        trace.finish();
        return resp;
    }

    // SessionEnd 独立处理(删 session + 关 popup),非 SessionEnd 继续主流程。
    if let Some(resp) = handle_session_end(state, &req) {
        trace.session_end_handled = true;
        trace.finish();
        return resp;
    }

    // status 解析:缺失时按 event_type 推导,兼容 success/failure/failed 老字符串。
    let status = parse_status(req.status.as_deref(), event_type.as_ref());

    // upsert 会 move req 的部分字段;binding / capture 阶段要用的终端字段
    // 在 move 之前一次性取出(取值与原实现逐字段一致)。
    let prefetch = NotifyPrefetch::collect(&req);
    let task_id = req.task_id.clone();
    // codex 桌面端会话:优先用 codex 对话真实 title (thread_name),查不到兜底
    // hook 上报的 workspace 目录名。CLI(有 tty)不变。
    let title = resolve_title_for_notify(&req);
    // req.source 被 upsert move 之前先 clone(herdr-remote 门控用)。
    let req_source = req.source.clone();

    let result = {
        let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.upsert_session(UpsertRequest {
            task_id: req.task_id,
            external_session_id: req.external_session_id,
            title,
            message: req.message,
            source: req.source,
            priority,
            status,
            terminal_tty: req.terminal_tty,
            workspace_path: req.workspace_path,
            failure_reason: req.failure_reason,
            transcript_path: req.transcript_path,
            aborted: req.aborted,
            terminal_app: req.terminal_app.clone(),
            dsh_nav_socket: req.dsh_nav_socket.clone(),
        })
    };

    trace.set_session_context(&result);

    // Codex 桌面端:尽早 thread/resume 订阅,减少 popup 决策时 thread 未知窗口。
    if is_codex_desktop_session(&result.session) {
        if let Some(thread_id) = result.session.external_session_id.as_deref() {
            state.codex_appserver.request_resume(thread_id);
        }
    }

    // 终端 binding 落库(herdr / cmux / wezterm / otty / tmux / orca)。
    // 返回 gate 前需要补全的 best-effort 项(慢探测,由 staging 在 gate 前执行)。
    let orca_herdr_enrichment =
        apply_terminal_bindings(state, &result, &task_id, req_source, &prefetch);

    // capture 触发(ghostty / wezterm / otty / orca)。
    run_capture_phases(state, &result, event_type, &prefetch);

    state.events.sessions_updated();

    // 新一轮 Running:关旧 popup + 清 acked + 失效 codex thread 快照。
    handle_running_transition(state, &result);

    // popup 呈现(gate 判定含 focus osascript 探测 + show/sound + trace 落盘)走
    // staging 派发:生产模式下在专用线程 FIFO 串行执行,`/notify` 不再等它。
    // 响应体与 inline 模式逐字相同(见 staging::run_stage 注释),所以先构造响应,
    // 再把 result / trace 交给 staging。
    // "阶段转移是否成立"是请求时属性,在派发前算好一起带过去(见
    // popup_gate::compute_stage_eligible 的注释)。
    // 参见 brief docs/briefs/20261013-focus-off-response-path.md。
    let stage_eligible = super::popup_gate::compute_stage_eligible(&result);
    let response =
        to_value_or_internal_error(&result.session, &result.session.task_id, result.is_new);
    state.staging.dispatch(
        state,
        super::staging::StageJob {
            result,
            trace,
            orca_herdr_enrichment,
            stage_eligible,
        },
    );
    response
}
