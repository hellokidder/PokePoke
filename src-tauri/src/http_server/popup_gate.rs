//! popup 触发决策与 show/sound 副作用。从 `http_server.rs` 抽出(brief
//! 20260727-split-http-server-rs 第二阶段),行为零变化。
//!
//! `evaluate_and_show_popup` 是 popup gate 的核心:stage eligible 时跑完整
//! gate(already_focused / cooldown / acked)并执行 show/sound。

use super::decision_trace::NotifyDecisionTrace;
use super::play_notify_sound;
use super::AppState;
use crate::log_info;
use crate::popup_decision::{
    evaluate_popup_presentation, is_monitor_backfill_idle, is_stage_popup_eligible, PopupGateInput,
    PopupPresentation, PopupSuppressReason, StageTransitionInput,
};
use crate::sessions::SessionStatus;
use crate::surface::SurfaceEvidence;
use axum::http::StatusCode;

/// 呈现决策的 wire 风格字符串(与 log_popup_focus_gate 的 decision 对齐)。
fn presentation_str(p: PopupPresentation) -> &'static str {
    match p {
        PopupPresentation::AttemptShow => "show",
        // 语义是"抑制 popup、保留声音",不是"抑制声音"(brief 20260802-popup-sound)。
        PopupPresentation::SuppressWithSound => "suppress_popup_keep_sound",
        PopupPresentation::SuppressSilent => "suppress_silent",
        PopupPresentation::Skip => "skip",
    }
}

fn session_for_popup_decision(
    state: &AppState,
    result: &crate::sessions::UpsertResult,
) -> crate::sessions::Session {
    state
        .store
        .lock()
        .ok()
        .and_then(|store| {
            store
                .get_all()
                .iter()
                .find(|s| s.id == result.session.id)
                .cloned()
        })
        .unwrap_or_else(|| result.session.clone())
}

/// popup 触发决策:stage eligible 时跑完整 gate(already_focused / cooldown / acked /
/// )并执行 show/sound 副作用。返回 `Some` = 提前返回
/// ,`None` = 正常走完,调用方用标准 status code 返回。
///
/// W2(brief 20260813-decision-trace):每个 gate 判定结果填入 `trace`(纯观测,
/// 判定行为零变化;缺省 = 该 gate 未走到)。
/// 阶段转移是否成立 —— 这是**请求时**属性,必须在 `/notify` 线程里算完再交给
/// staging,不能等到 worker 里用 live session 判定。
///
/// 原因(2026-10-13 实测):gate 搬到 staging 后读的是 store 里的**当前** session;
/// 当 `running` 与 `stop` 在几毫秒内相继到达时,先到的 `running` 会看到后到的
/// `idle` 状态,被误判 `stage_eligible=true` 并**凭空多弹一次窗**
/// (实测 trace:首个 running 事件 `stage_eligible=true presentation=show`)。
/// 用 upsert 快照(`result.session`)判定后,每条请求的判定回到与请求时一致。
pub(super) fn compute_stage_eligible(result: &crate::sessions::UpsertResult) -> bool {
    let session = &result.session;
    // brief 20260813-herdr-triple-popup:监视器源重建(is_new+Idle)是 store 丢失
    // 后补喂的回溯状态,不是"进入 idle"迁移 → 直接 skip,不弹不响。
    if is_monitor_backfill_idle(result.is_new, &session.status, session.source.as_deref()) {
        log_info!(
            "notify",
            "stage skip monitor-backfill-idle task_id={} session_id={}",
            session.task_id,
            session.id,
        );
        return false;
    }
    is_stage_popup_eligible(&StageTransitionInput {
        status: &session.status,
        prev_status: result.prev_status.as_ref(),
        is_new: result.is_new,
        message: &session.message,
        prev_message: result.prev_message.as_deref(),
    })
}

/// popup 触发决策:stage eligible 时跑完整 gate(already_focused / cooldown / acked /
/// )并执行 show/sound 副作用。返回 `Some` = 提前返回
/// ,`None` = 正常走完,调用方用标准 status code 返回。
///
/// `stage_eligible` 由调用方在**请求时**算好传入(见 `compute_stage_eligible`),
/// 因为本函数现在跑在 staging worker 里,读到的 session 已可能被后续请求推进。
///
/// W2(brief 20260813-decision-trace):每个 gate 判定结果填入 `trace`(纯观测,
/// 判定行为零变化;缺省 = 该 gate 未走到)。
pub(super) fn evaluate_and_show_popup(
    state: &AppState,
    result: &crate::sessions::UpsertResult,
    stage_eligible: bool,
    trace: &mut NotifyDecisionTrace,
) -> Option<(StatusCode, serde_json::Value)> {
    let session = session_for_popup_decision(state, result);
    trace.stage_eligible = stage_eligible;

    if stage_eligible {
        // Codex 自动审批抑制(brief 20260715):Layer A rollout 判是否 auto_review 轮;
        // Layer B app-server thread status 判回环(active) vs 真结束(idle)。
        // Pending:guardian 自审 PermissionRequest 在 thread 未知时降级抑制;
        // 仅 waitingOnUserInput 时仍弹。
        let auto_approved = state.auto_approval.is_auto_approved(&session.id);
        let thread_id = session.external_session_id.as_deref();
        let thread_status = thread_id.and_then(|tid| state.codex_thread_status.get_entry(tid));
        if let Some(reason) = crate::codex_appserver::codex_auto_popup_suppress_reason(
            &session,
            auto_approved,
            thread_status,
        ) {
            trace.codex_suppress_reason = reason.to_string();
            if let Some(metrics) = crate::runtime_metrics::global() {
                metrics.record_popup_suppress(reason);
            }
            log_info!(
                "notify",
                "show popup SUPPRESSED task_id={} session_id={} reason={} status={:?} auto_approved={} thread_id={:?} thread_status={:?} thread_provenance={:?} tty={:?}",
                session.task_id,
                session.id,
                reason,
                session.status,
                auto_approved,
                thread_id,
                thread_status.map(|e| e.status),
                thread_status.map(|e| e.provenance),
                session.terminal_tty,
            );
            state.events.sessions_updated();
            return None;
        }
        let surface_evidence = SurfaceEvidence::from_session(&session);
        let resolved_surface = crate::surface::resolve_surface(&surface_evidence);
        crate::surface::log_resolution(
            "surface",
            "already_focused",
            &surface_evidence,
            &resolved_surface,
        );
        let already_focused = state
            .focus
            .is_focused_resolved(&surface_evidence, &resolved_surface);
        let focus_suppression_enabled = state
            .settings
            .lock()
            .map(|s| s.settings.focus_suppression_enabled)
            .unwrap_or(true);
        let quiet_mode_active = state
            .quiet_mode
            .as_ref()
            .map(|q| q.is_active())
            .unwrap_or(false);
        let gate_allow = state.gate.allow_notifications();
        let recently_visited = state.memory.was_recently(&session.id);
        // Pending 不受 AckedStages 抑制:用户点过 permission popup 只是"去看了一眼",
        // 授权未必给、agent 仍卡在 Pending 等输入。若 mark acked 后永久抑制直到
        // Running,用户错过首次后 agent 会无声卡死。配合 is_stage_popup_eligible
        // 对 Pending 永远 eligible,permission 每次 retry 都能再弹,频率仅由
        // PopupCooldown(3s) 兜底。Idle/LastFailed 仍走原 acked 语义(认领过就别烦)。
        let stage_acked = session.status != SessionStatus::Pending
            && state.memory.is_acked(&session.id, &session.status);
        // cooldown 不可用的两类原因合并判断(都直接 false,不进 try_acquire):
        //   1. already_focused && focus_suppression_enabled —— 前台已对,且开了"前台时抑制"
        //   2. !gate_allow || recently_visited || stage_acked —— 全局开关关 / 刚访问过 / 已认领阶段
        // 其余情况才尝试获取 cooldown 槽。
        let cooldown_available = if (already_focused && focus_suppression_enabled)
            || !gate_allow
            || recently_visited
            || stage_acked
        {
            false
        } else {
            state.memory.popup_try_acquire(&session.id)
        };

        // W2:gate verdict 全部填入 trace(纯观测)。
        trace.already_focused = already_focused;
        trace.focus_suppression_enabled = focus_suppression_enabled;
        trace.quiet_mode_active = quiet_mode_active;
        trace.gate_allow = gate_allow;
        trace.recently_visited = recently_visited;
        trace.stage_acked = stage_acked;
        trace.cooldown_available = cooldown_available;

        let (presentation, suppress_reason) = evaluate_popup_presentation(
            stage_eligible,
            &PopupGateInput {
                already_focused,
                focus_suppression_enabled,
                gate_allow,
                recently_visited,
                stage_acked,
                cooldown_available,
            },
        );

        crate::popup_decision::log_popup_focus_gate(
            &session.task_id,
            &session.id,
            presentation,
            already_focused,
            &surface_evidence,
            &resolved_surface,
            suppress_reason,
        );

        // W2:呈现决策 + 抑制原因填入 trace。
        trace.presentation = presentation_str(presentation).to_string();
        trace.suppress_reason = suppress_reason
            .map(PopupSuppressReason::as_str)
            .unwrap_or_default()
            .to_string();

        match presentation {
            PopupPresentation::AttemptShow => {
                // race guard(2026-07-14):upsert 拿到 session clone 后到 show 之间,
                // 并发的 SessionEnd 可能已按 task_id 删掉该 session(尤其 cursor 在
                // ttys 上 ~250ms 内连发 session_end x2 + idle x2)。stale clone 仍带旧
                // session.id,popup 按 session.id 建 window -> 给"已删的 session"弹幽灵
                // popup(同 task_id 出现两个并存 popup、位置漂移、Cmd-N 跳歪)。show 前重查
                // store,已被删就当 SUPPRESSED 跳过,不建 window 也不放 sound。
                let still_in_store = state
                    .store
                    .lock()
                    .map(|s| s.session_exists(&session.id))
                    .unwrap_or(false);
                trace.still_in_store = still_in_store;
                if !still_in_store {
                    if let Some(metrics) = crate::runtime_metrics::global() {
                        metrics.record_popup_suppress("session_removed_before_show");
                    }
                    log_info!(
                        "notify",
                        "show popup SUPPRESSED task_id={} session_id={} reason=session_removed_before_show",
                        session.task_id,
                        session.id,
                    );
                    state.events.sessions_updated();
                    return None;
                }
                log_info!(
                    "notify",
                    "show popup task_id={} session_id={} status={:?} prev={:?} auto_approved={} thread_id={:?} thread_status={:?} tty={:?}",
                    session.task_id,
                    session.id,
                    session.status,
                    result.prev_status,
                    auto_approved,
                    thread_id,
                    thread_status.map(|e| e.status),
                    session.terminal_tty,
                );
                let popup_created = state.popup.show(&session);
                trace.popup_created = popup_created;
                // 统一播音(brief 20260805-pending-idle-completed-sound-swallowed):
                // 新 popup 或 refresh-only 的 **stage 变化**(prev != status)都响——
                // Pending 弹窗在屏时到达的 Idle 走 refresh-only 也会播 completed 音,
                // 不再被"popup 已在屏"吞掉;纯内容刷新(同 status)保持静默。
                // 播音本身经 play_notify_sound 节流(stage-ending 消耗槽位)。
                let stage_changed = result.prev_status.as_ref() != Some(&session.status);
                if popup_created || stage_changed {
                    // W2:play_notify_sound 只回 bool;抑制原因由与它相同的判定条件
                    // 推导(quiet → aborted → sound_cooldown,见 state.rs 实现),
                    // 纯观测标签,不影响任何判定。
                    let played = play_notify_sound(state, &session);
                    trace.sound_played = if played {
                        "played".to_string()
                    } else if quiet_mode_active {
                        "suppressed:quiet_mode".to_string()
                    } else if session.is_recently_aborted() {
                        "suppressed:aborted".to_string()
                    } else {
                        "suppressed:sound_cooldown".to_string()
                    };
                } else {
                    log_info!(
                        "notify",
                        "show popup refresh-only (no sound) task_id={} session_id={}",
                        session.task_id,
                        session.id,
                    );
                    trace.sound_played = "skipped_refresh_only".to_string();
                }
            }
            PopupPresentation::SuppressWithSound => {
                // 已聚焦时弹窗被抑制、只保留声音。但 CC 对同一次 permission prompt
                // 会连发多条 pending 事件(间隔 >100ms dedupe 窗口,prev 仍为 Pending),
                // 无条件播音会让 waiting 音重复响(用户反馈"一次权限请求响两次")。
                // 只在状态变化时响:重复 pending(prev==Pending)静默,与 AttemptShow
                // 分支的 stage_changed 语义对齐。stage-ending 的重复事件仍由
                // play_notify_sound 的 sound_cooldown(3s)节流,此处不重复处理。
                let repeat_pending = session.status == SessionStatus::Pending
                    && result.prev_status.as_ref() == Some(&SessionStatus::Pending);
                if repeat_pending {
                    trace.sound_played = "skipped_repeat_pending".to_string();
                    log_info!(
                        "notify",
                        "show popup SUPPRESSED task_id={} reason={} (repeat pending, no sound) tty={:?}",
                        session.task_id,
                        suppress_reason
                            .map(PopupSuppressReason::as_str)
                            .unwrap_or("unknown"),
                        session.terminal_tty,
                    );
                    return None;
                }
                if !quiet_mode_active {
                    // quiet 已守卫,aborted / sound_cooldown / play 统一走
                    // play_notify_sound(含 Pending 不消耗槽位规则,见 helper 注释)。
                    let played = play_notify_sound(state, &session);
                    trace.sound_played = if played {
                        "played".to_string()
                    } else if session.is_recently_aborted() {
                        "suppressed:aborted".to_string()
                    } else {
                        "suppressed:sound_cooldown".to_string()
                    };
                } else if quiet_mode_active {
                    trace.sound_played = "suppressed:quiet_mode".to_string();
                    log_info!(
                        "notify",
                        "sound SUPPRESSED task_id={} reason=quiet_mode tty={:?}",
                        session.task_id,
                        session.terminal_tty,
                    );
                }
                log_info!(
                    "notify",
                    "show popup SUPPRESSED task_id={} reason={} tty={:?}",
                    session.task_id,
                    suppress_reason
                        .map(PopupSuppressReason::as_str)
                        .unwrap_or("unknown"),
                    session.terminal_tty,
                );
            }
            PopupPresentation::SuppressSilent => {
                // brief 20260805-pending-idle-completed-sound-swallowed(kimi 评审线路):
                // cooldown 抑制的 stage-ending 事件降级"只播音不弹窗"——Pending 弹窗
                // 3s 内的 Idle(完成)不该被窗口节流连带吞掉完成音(用户反馈:任务完成
                // 却只听到 waiting 音)。只对 reason=cooldown 生效:gate_off /
                // recently_visited / stage_acked 是"用户已知道"型抑制,保持静默;
                // Pending 也保持静默(屏上已有 popup,重复 waiting 是噪音)。
                if suppress_reason == Some(PopupSuppressReason::Cooldown)
                    && matches!(
                        session.status,
                        SessionStatus::Idle | SessionStatus::LastFailed
                    )
                    && !quiet_mode_active
                {
                    let played = play_notify_sound(state, &session);
                    trace.sound_played = if played {
                        "played".to_string()
                    } else if session.is_recently_aborted() {
                        "suppressed:aborted".to_string()
                    } else {
                        "suppressed:sound_cooldown".to_string()
                    };
                } else {
                    // 静默抑制:不播音。quiet 兜底时原因记为 quiet_mode。
                    trace.sound_played = if quiet_mode_active {
                        "suppressed:quiet_mode".to_string()
                    } else {
                        "not_played".to_string()
                    };
                }
                log_info!(
                    "notify",
                    "show popup SUPPRESSED task_id={} reason={} tty={:?}",
                    session.task_id,
                    suppress_reason
                        .map(PopupSuppressReason::as_str)
                        .unwrap_or("unknown"),
                    session.terminal_tty,
                );
            }
            PopupPresentation::Skip => {}
        }
    }

    None
}
