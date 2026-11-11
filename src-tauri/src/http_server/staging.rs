//! `/notify` 的 popup 呈现(staging)派发:把"完整 gate 判定 + popup show +
//! sound + 决策 trace 落盘"从 **响应路径** 上摘下来,交给专用 worker 线程串行执行。
//!
//! ## 动机(brief `docs/briefs/20261013-focus-off-response-path.md`)
//!
//! `/notify` 的响应体是 `process_notify` 的返回值,而 gate 里的 focus 判定要跑
//! osascript(App/窗口枚举与 focus 探测,实测 Ghostty 枚举 ~200ms、focus `run_bool`
//! 3×约 40–115ms、Orca CLI 2×约 110ms),把响应推到 0.2–1.9s。
//! 宿主 hook 必须等到响应才能断开(spec:提前断开会让 axum/hyper 取消 in-flight
//! future,通知静默丢失,实测落库 0/6),于是这 0.2–1.9s 全额变成 agent 的 hook 延迟。
//!
//! 但 gate 的结论只决定"弹不弹窗 / 播不播音",不影响响应内容(响应只是 session JSON),
//! 所以整段呈现都可以延后执行:响应立刻返回,gate 在后台按同样顺序跑完。
//!
//! ## 为什么是"单线程 + FIFO channel",而不是每个请求 `tokio::spawn`
//!
//! 1. **顺序**:gate 依赖按时间先后累积的状态(popup cooldown / acked / recently
//!    visited),并发执行会让 `pending → idle` 这类紧凑序列的判定次序翻转;
//! 2. **串行 osascript**:osascript 是全局重资源,并发跑会互相拖慢并把系统打满;
//! 3. **脱离请求 future**:worker 线程独立于连接,即使 hook 提前断开(客户端
//!    关闭 / 2s 超时),staging 仍会跑完并落盘。
//!
//! 派发本身只做一次 channel `send`,不阻塞 `/notify`;`inline` 模式保留原同步
//! 语义(测试与 worker 起不来时的回退),两条路径的 gate 与落盘行为逐字相同。

use super::decision_trace::NotifyDecisionTrace;
use super::popup_gate::evaluate_and_show_popup;
use super::AppState;
use crate::log_error;
use crate::log_info;
use crate::sessions::UpsertResult;
use std::sync::mpsc::{self, Receiver, Sender};

/// 一次 staging 的全部输入。纯数据(无借用),可跨线程移动。
pub struct StageJob {
    /// upsert 结果(gate 判定与 show/sound 都用它)。
    pub result: UpsertResult,
    /// 决策 trace;由本模块负责 `finish()`(每个 /notify 恰好落一条)。
    pub trace: NotifyDecisionTrace,
    /// gate **之前**要跑的 best-effort binding 补全(herdr host 缺 ORCA_PANE_KEY
    /// 时用 Orca CLI 补外层 pane)。这段会跑两次 Orca CLI(实测各 ~110ms,失败
    /// 还会重试),所以从响应路径搬到这里:响应不等它,但它仍在 gate 之前执行,
    /// 保持"binding 先于 gate 读取"的原有顺序。`None` = 本次无需补全。
    pub orca_herdr_enrichment: Option<OrcaHerdrEnrichment>,
    /// **请求时**算好的"阶段转移是否成立"。必须冻结在派发时刻:gate 异步读 store,
    /// 若在 worker 里用 live session 判定,几毫秒内到达的 running→stop 序列会让
    /// 前一条看到后一条的状态,多弹一次窗(见 `popup_gate::compute_stage_eligible`)。
    pub stage_eligible: bool,
}

/// `apply_orca_binding_for_herdr_host_if_missing` 的入参(见 `StageJob` 注释)。
pub struct OrcaHerdrEnrichment {
    pub task_id: String,
    pub workspace_path: Option<String>,
}

/// staging 派发器。
///
/// - `inline()`:在 `/notify` 线程内同步跑完 gate 才返回 —— 与 2026-10-13 之前
///   的行为完全一致,**测试与回退路径**使用;
/// - `spawn_worker()`:生产路径。专用线程按 FIFO 串行执行,响应不再等 osascript。
#[derive(Clone)]
pub struct StagingDispatcher {
    tx: Option<Sender<StageJob>>,
}

impl StagingDispatcher {
    /// 同步执行(测试 / 回退)。构造 `AppState` 的默认值。
    pub fn inline() -> Self {
        Self { tx: None }
    }

    /// 起专用 worker 线程(持有 `AppState` 克隆)并返回派发器。
    ///
    /// 线程随进程存活:发送端由 `AppState` 持有,进程退出时自然结束。
    /// 线程创建失败时**回退为 inline** 并记 ERROR —— 宁可慢,不可丢通知。
    pub fn spawn_worker(state: AppState) -> Self {
        let (tx, rx) = mpsc::channel::<StageJob>();
        let spawned = std::thread::Builder::new()
            .name("notify-staging".into())
            .spawn(move || run_worker(state, rx));
        match spawned {
            Ok(_) => {
                log_info!(
                    "notify",
                    "staging worker started (popup gate 脱离 /notify 响应路径)"
                );
                Self { tx: Some(tx) }
            }
            Err(e) => {
                log_error!(
                    "notify",
                    "staging worker spawn failed: {} — 回退为同步 staging(响应会变慢但行为正确)",
                    e
                );
                Self { tx: None }
            }
        }
    }

    /// 派发一次 staging。worker 模式只做 channel `send`,不阻塞调用方。
    ///
    /// worker 已退出(channel 断开)时同步兜底执行,保证 trace 与 popup 副作用
    /// 不丢。
    pub fn dispatch(&self, state: &AppState, job: StageJob) {
        if let Some(tx) = &self.tx {
            match tx.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(returned)) => {
                    log_error!(
                        "notify",
                        "staging worker 已退出,本次 staging 回退同步执行(响应会变慢)"
                    );
                    run_stage(state, returned);
                    return;
                }
            }
        }
        run_stage(state, job);
    }
}

fn run_worker(state: AppState, rx: Receiver<StageJob>) {
    while let Ok(job) = rx.recv() {
        run_stage(&state, job);
    }
}

/// 执行一次 staging 并落 trace。
///
/// `evaluate_and_show_popup` 的返回值只用于"配额耗尽"分支的响应体,而该响应体与
/// 正常返回逐字相同(`popup_gate.rs` 的 `free_quota_exhausted` 分支调用的就是同一个
/// `to_value_or_internal_error`),且响应已在派发前发出,所以这里直接丢弃返回值。
fn run_stage(state: &AppState, job: StageJob) {
    let StageJob {
        result,
        mut trace,
        orca_herdr_enrichment,
        stage_eligible,
    } = job;

    // gate 之前的 best-effort binding 补全(慢:Orca CLI)。摆在 gate 之前是为了
    // 保持原有顺序 —— gate 读 store 里的 surface binding,补全必须先落库。
    if let Some(enrichment) = &orca_herdr_enrichment {
        crate::commands::focus::apply_orca_binding_for_herdr_host_if_missing(
            &state.store,
            &enrichment.task_id,
            enrichment.workspace_path.as_deref(),
            false,
        );
        // 原实现里 binding 写入与 upsert 共用同一次 emit(写库在 emit 之前),
        // 搬到这里后补一次 emit,保持 UI 能看到新 binding。
        state.events.sessions_updated();
    }

    let _ = evaluate_and_show_popup(state, &result, stage_eligible, &mut trace);
    trace.finish();
}
