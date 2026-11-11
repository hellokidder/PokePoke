//! Popup 阶段资格与门控决策 — 把 `should_show_popup` + 多道抑制关卡
//! 收成单一入口,日志统一打 `reason_code` 便于排障。

use crate::sessions::SessionStatus;

/// popup 被抑制的原因码。与 `log_info!("show popup SUPPRESSED reason=...")` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupSuppressReason {
    /// 状态转移不满足 stage-ending / pending 提醒条件
    NotStageEligible,
    AlreadyFocused,
    GateOff,
    RecentlyVisited,
    StageAcked,
    Cooldown,
}

impl PopupSuppressReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStageEligible => "not_stage_eligible",
            Self::AlreadyFocused => "already_focused",
            Self::GateOff => "gate_off",
            Self::RecentlyVisited => "recently_visited",
            Self::StageAcked => "stage_acked",
            Self::Cooldown => "cooldown",
        }
    }
}

/// popup 呈现决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupPresentation {
    /// 不满足阶段提醒条件,不尝试弹窗
    Skip,
    /// 尝试 show;是否 refresh-only 由 PopupController.show 返回值决定
    AttemptShow,
    /// 抑制弹窗但播放提示音(终端已聚焦场景)
    SuppressWithSound,
    /// 抑制弹窗且不播音
    SuppressSilent,
}

pub struct StageTransitionInput<'a> {
    pub status: &'a SessionStatus,
    pub prev_status: Option<&'a SessionStatus>,
    pub is_new: bool,
    // message / prev_message 当前未被 is_stage_popup_eligible 读取:Pending 分支
    // 改成"永远 eligible"后不再依赖 message 比较,Idle/LastFailed 分支本就只看
    // status 迁移。保留字段 + 调用侧仍填充,是为了未来若给 Idle/LastFailed 加
    // "message 变化即新提醒"判定时不必再改调用链。see 2026-06-25 permission 重弹修订。
    #[allow(dead_code)]
    pub message: &'a str,
    #[allow(dead_code)]
    pub prev_message: Option<&'a str>,
}

pub struct PopupGateInput {
    pub already_focused: bool,
    pub focus_suppression_enabled: bool,
    pub gate_allow: bool,
    pub recently_visited: bool,
    pub stage_acked: bool,
    pub cooldown_available: bool,
}

/// 用户重新发起一轮时是否应关闭残留 popup。
pub fn should_close_popup_on_running(
    is_new: bool,
    status: &SessionStatus,
    prev: Option<&SessionStatus>,
) -> bool {
    !is_new
        && *status == SessionStatus::Running
        && prev.is_some_and(|prev| {
            matches!(
                prev,
                SessionStatus::Idle | SessionStatus::LastFailed | SessionStatus::Pending
            )
        })
}

/// 是否属于"阶段结束 / 新 pending"的提醒资格。
pub fn is_stage_popup_eligible(input: &StageTransitionInput<'_>) -> bool {
    // Pending → Idle(Stop 覆盖)也在这里放行:CC 在权限审批期间 turn 不会结束,
    // Stop 只在审批解决后(批准→turn 完成 / 拒绝→turn 结束)到达,此时弹"完成"
    // 是正确信号。payload 无法区分"审批未决时被 Esc 中断"的异常流,该场景记
    // followup(docs/briefs/20260801-kimi-review-round2.md)。
    let is_stage_end_transition = matches!(
        *input.status,
        SessionStatus::Idle | SessionStatus::LastFailed
    ) && (input.is_new
        || input
            .prev_status
            .map(|prev| prev != input.status)
            .unwrap_or(true));
    // Pending 永远 eligible:agent 等用户授权/输入是"需要人介入"的硬提醒,
    // 不该被 pending→pending 同 message 抑制。真实场景:cc 卡在 permission
    // 等 approve,hook 每 4-15 分钟 retry 一次 notification,message 不变,
    // 旧逻辑把 retry 全吞了 → 用户错过首次 popup 后再无提醒,agent 一直卡死。
    // 频率由调用侧 PopupCooldown(3s) + AckedStages 对 Pending 放行(see
    // http_server)共同控制,这里只管"该不该提醒"。
    let is_pending_transition = *input.status == SessionStatus::Pending;

    is_stage_end_transition || is_pending_transition
}

/// 监视器源的"新 session 首条即 idle"是否是回溯状态、不该弹(brief
/// 20260813-herdr-triple-popup)。product-spec §模块 3 触发条件只列
/// "进入 idle 且 prev ≠ idle"与"新建直接 pending";对 agent hook 源
/// (subagent Stop 无前置 Running 是真实完成信号,scenario 05)is_new+Idle
/// 保留弹窗,对监视器源(herdr / herdr-remote)则是 store 丢失后重建的噪音 ——
/// brief 20260810 首观测静默原则的 server 侧安全网(监视器 known-map 防不住
/// reaper 误删后的重建 race)。
pub fn is_monitor_backfill_idle(
    is_new: bool,
    status: &SessionStatus,
    source: Option<&str>,
) -> bool {
    is_new
        && *status == SessionStatus::Idle
        && matches!(source, Some("herdr") | Some("herdr-remote"))
}

/// 综合阶段资格 + 门控,输出 popup 呈现方式与抑制原因。
pub fn evaluate_popup_presentation(
    stage_eligible: bool,
    gates: &PopupGateInput,
) -> (PopupPresentation, Option<PopupSuppressReason>) {
    if !stage_eligible {
        return (
            PopupPresentation::Skip,
            Some(PopupSuppressReason::NotStageEligible),
        );
    }

    let focus_suppressed = gates.already_focused && gates.focus_suppression_enabled;

    if focus_suppressed {
        return (
            PopupPresentation::SuppressWithSound,
            Some(PopupSuppressReason::AlreadyFocused),
        );
    }
    if !gates.gate_allow {
        return (
            PopupPresentation::SuppressSilent,
            Some(PopupSuppressReason::GateOff),
        );
    }
    if gates.recently_visited {
        return (
            PopupPresentation::SuppressSilent,
            Some(PopupSuppressReason::RecentlyVisited),
        );
    }
    if gates.stage_acked {
        return (
            PopupPresentation::SuppressSilent,
            Some(PopupSuppressReason::StageAcked),
        );
    }
    if !gates.cooldown_available {
        return (
            PopupPresentation::SuppressSilent,
            Some(PopupSuppressReason::Cooldown),
        );
    }

    (PopupPresentation::AttemptShow, None)
}

/// ① 弹前 gate INFO:decision + surface + bindings,抑制与弹出都打。
pub fn log_popup_focus_gate(
    task_id: &str,
    session_id: &str,
    presentation: PopupPresentation,
    already_focused: bool,
    evidence: &crate::surface::SurfaceEvidence,
    resolved: &crate::surface::ResolvedSurface,
    suppress_reason: Option<PopupSuppressReason>,
) {
    let decision = match presentation {
        PopupPresentation::AttemptShow => "show",
        // 注意：这里语义是"抑制 popup、保留声音"，不是"抑制声音"。
        // 旧字符串 "suppress_sound" 与行为相反，排障时误导（brief 20260802-popup-sound）。
        PopupPresentation::SuppressWithSound => "suppress_popup_keep_sound",
        PopupPresentation::SuppressSilent => "suppress_silent",
        PopupPresentation::Skip => "skip",
    };
    crate::surface::log_focus_gate_decision(
        task_id,
        session_id,
        decision,
        already_focused,
        evidence,
        resolved,
        suppress_reason.map(PopupSuppressReason::as_str),
    );
    if let Some(metrics) = crate::runtime_metrics::global() {
        metrics.record_focus_gate(decision);
        if let Some(reason) = suppress_reason {
            metrics.record_popup_suppress(reason.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionStatus;

    #[test]
    fn stage_eligible_running_to_idle() {
        let input = StageTransitionInput {
            status: &SessionStatus::Idle,
            prev_status: Some(&SessionStatus::Running),
            is_new: false,
            message: "",
            prev_message: Some(""),
        };
        assert!(is_stage_popup_eligible(&input));
    }

    #[test]
    fn stage_ineligible_running_to_running() {
        let input = StageTransitionInput {
            status: &SessionStatus::Running,
            prev_status: Some(&SessionStatus::Running),
            is_new: false,
            message: "",
            prev_message: Some(""),
        };
        assert!(!is_stage_popup_eligible(&input));
    }

    #[test]
    fn stage_eligible_new_idle_kept_for_agent_hook_sources() {
        // subagent Stop 无前置 Running(is_new+Idle)是真实完成信号,Fix B 只对
        // 监视器源生效,agent hook 源照旧弹。
        let input = StageTransitionInput {
            status: &SessionStatus::Idle,
            prev_status: None,
            is_new: true,
            message: "",
            prev_message: None,
        };
        assert!(is_stage_popup_eligible(&input));
    }

    // ==================== is_monitor_backfill_idle(brief 20260813-herdr-triple-popup) ====================

    #[test]
    fn monitor_backfill_idle_true_for_monitor_sources_new_idle() {
        for src in ["herdr", "herdr-remote"] {
            assert!(
                is_monitor_backfill_idle(true, &SessionStatus::Idle, Some(src)),
                "source={src} 的新建 idle 应判回溯补喂"
            );
        }
    }

    #[test]
    fn monitor_backfill_idle_false_outside_new_idle_monitor_combination() {
        // 非新建:真实迁移(running→idle 的补喂),照常走 stage 判定。
        assert!(!is_monitor_backfill_idle(
            false,
            &SessionStatus::Idle,
            Some("herdr")
        ));
        // 非 idle:running / pending 新建不受影响(blocked 首观测仍弹)。
        assert!(!is_monitor_backfill_idle(
            true,
            &SessionStatus::Running,
            Some("herdr")
        ));
        assert!(!is_monitor_backfill_idle(
            true,
            &SessionStatus::Pending,
            Some("herdr")
        ));
        // 非监视器源:agent hook 的 is_new+Idle 保留(subagent Stop 场景)。
        for src in ["pi", "claude-code", "codex", "cursor", "opencode"] {
            assert!(!is_monitor_backfill_idle(
                true,
                &SessionStatus::Idle,
                Some(src)
            ));
        }
        assert!(!is_monitor_backfill_idle(true, &SessionStatus::Idle, None));
    }

    #[test]
    fn presentation_attempt_show_when_gates_pass() {
        let gates = PopupGateInput {
            already_focused: false,
            focus_suppression_enabled: true,
            gate_allow: true,
            recently_visited: false,
            stage_acked: false,
            cooldown_available: true,
        };
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::AttemptShow);
        assert!(reason.is_none());
    }

    #[test]
    fn presentation_suppress_with_sound_when_focused() {
        let gates = PopupGateInput {
            already_focused: true,
            focus_suppression_enabled: true,
            gate_allow: true,
            recently_visited: false,
            stage_acked: false,
            cooldown_available: true,
        };
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn presentation_skip_when_not_stage_eligible() {
        let gates = PopupGateInput {
            already_focused: false,
            focus_suppression_enabled: true,
            gate_allow: true,
            recently_visited: false,
            stage_acked: false,
            cooldown_available: true,
        };
        let (p, reason) = evaluate_popup_presentation(false, &gates);
        assert_eq!(p, PopupPresentation::Skip);
        assert_eq!(reason, Some(PopupSuppressReason::NotStageEligible));
    }

    // ==== Focus suppression 组合矩阵 ====
    //
    // 由 `docs/briefs/20260708-vm-e2e-07-08-skip-and-decision-unit-tests.md` 补齐。
    // VM E2E 08 Case B 承载的"已聚焦 + suppression 开关 → 抑制弹窗"决策,
    // 从 VM 唯一验证下沉到本地单测,避免受 macOS SSH AppleScript activate
    // 判定不稳的影响。四种 (already_focused, focus_suppression_enabled) 组合
    // 全部枚举,期望结果对齐 `docs/product-spec.md §模块 3 → 触发条件`。

    fn passing_gates() -> PopupGateInput {
        PopupGateInput {
            already_focused: false,
            focus_suppression_enabled: true,
            gate_allow: true,
            recently_visited: false,
            stage_acked: false,
            cooldown_available: true,
        }
    }

    #[test]
    fn focus_matrix_focused_and_suppression_on_suppresses_with_sound() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.focus_suppression_enabled = true;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn focus_matrix_focused_but_suppression_off_still_attempts_show() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.focus_suppression_enabled = false;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::AttemptShow);
        assert!(reason.is_none());
    }

    #[test]
    fn focus_matrix_not_focused_and_suppression_on_attempts_show() {
        let mut gates = passing_gates();
        gates.already_focused = false;
        gates.focus_suppression_enabled = true;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::AttemptShow);
        assert!(reason.is_none());
    }

    #[test]
    fn focus_matrix_not_focused_and_suppression_off_attempts_show() {
        let mut gates = passing_gates();
        gates.already_focused = false;
        gates.focus_suppression_enabled = false;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::AttemptShow);
        assert!(reason.is_none());
    }

    // ==== 抑制优先级 ====
    //
    // `already_focused` 与其它抑制项同时命中时,应先输 AlreadyFocused,
    // 因为它带 sound(SuppressWithSound),而其它是 SuppressSilent。
    // 用户已在终端时至少给一次音效比默默压掉更合承诺。

    #[test]
    fn suppression_priority_focused_beats_gate_off() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.gate_allow = false;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn suppression_priority_focused_beats_recently_visited() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.recently_visited = true;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn suppression_priority_focused_beats_stage_acked() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.stage_acked = true;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn suppression_priority_focused_beats_cooldown() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        gates.cooldown_available = false;
        let (p, reason) = evaluate_popup_presentation(true, &gates);
        assert_eq!(p, PopupPresentation::SuppressWithSound);
        assert_eq!(reason, Some(PopupSuppressReason::AlreadyFocused));
    }

    #[test]
    fn suppression_priority_stage_ineligible_beats_focused() {
        let mut gates = passing_gates();
        gates.already_focused = true;
        let (p, reason) = evaluate_popup_presentation(false, &gates);
        assert_eq!(p, PopupPresentation::Skip);
        assert_eq!(reason, Some(PopupSuppressReason::NotStageEligible));
    }
}
