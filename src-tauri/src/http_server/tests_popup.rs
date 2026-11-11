//! http_server 单测:popup 阶段判定(stage eligible 过渡矩阵 + Running 关 popup)。
//! 从 `http_server.rs` 的 `mod tests` 拆出(brief 20260727-split-http-server-rs
//! 第二阶段),`super::*` 引用路径不变,测试内容零改动。

use crate::popup_decision::{
    is_stage_popup_eligible, should_close_popup_on_running, StageTransitionInput,
};
use crate::sessions::SessionStatus;

#[test]
fn should_close_popup_returns_false_for_new_sessions() {
    assert!(!should_close_popup_on_running(
        true,
        &SessionStatus::Running,
        None
    ));
}

#[test]
fn should_close_popup_closes_idle_to_running_transition() {
    assert!(should_close_popup_on_running(
        false,
        &SessionStatus::Running,
        Some(&SessionStatus::Idle),
    ));
}

#[test]
fn should_close_popup_closes_last_failed_to_running_transition() {
    assert!(should_close_popup_on_running(
        false,
        &SessionStatus::Running,
        Some(&SessionStatus::LastFailed),
    ));
}

#[test]
fn should_close_popup_closes_pending_to_running_transition() {
    assert!(should_close_popup_on_running(
        false,
        &SessionStatus::Running,
        Some(&SessionStatus::Pending),
    ));
}

#[test]
fn should_close_popup_keeps_popup_for_running_to_running_transition() {
    assert!(!should_close_popup_on_running(
        false,
        &SessionStatus::Running,
        Some(&SessionStatus::Running),
    ));
}

fn stage_eligible(
    status: &SessionStatus,
    prev: Option<&SessionStatus>,
    is_new: bool,
    message: &str,
    prev_message: Option<&str>,
) -> bool {
    is_stage_popup_eligible(&StageTransitionInput {
        status,
        prev_status: prev,
        is_new,
        message,
        prev_message,
    })
}

// 大部分 transition-only 测不依赖 message,统一传 ""/None。
// pending→pending 的 message 分支单独覆盖。

#[test]
fn should_show_popup_for_running_to_idle_transition() {
    assert!(stage_eligible(
        &SessionStatus::Idle,
        Some(&SessionStatus::Running),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_running_to_last_failed_transition() {
    assert!(stage_eligible(
        &SessionStatus::LastFailed,
        Some(&SessionStatus::Running),
        false,
        "",
        None,
    ));
}

#[test]
fn should_not_show_popup_for_idle_to_idle_transition() {
    assert!(!stage_eligible(
        &SessionStatus::Idle,
        Some(&SessionStatus::Idle),
        false,
        "",
        None,
    ));
}

#[test]
fn should_not_show_popup_for_last_failed_to_last_failed_transition() {
    assert!(!stage_eligible(
        &SessionStatus::LastFailed,
        Some(&SessionStatus::LastFailed),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_running_to_pending_transition() {
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::Running),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_idle_to_pending_transition() {
    // 用户工作流:agent 完成 → idle → 又进入 pending 等输入,这是新的
    // "等待用户"提醒,必须弹 popup。原规则只看 prev=Running 会吞掉这条。
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::Idle),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_last_failed_to_pending_transition() {
    // 上一轮 API 错误后 agent 重试又进入 pending,也是新提醒。
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::LastFailed),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_pending_to_pending_same_message() {
    // 2026-06-25 修订:Pending 永远 eligible。cc 卡在 permission 等 approve
    // 时,hook 每 4-15 分钟 retry 同一条 notification(message 不变),旧逻辑
    // 吞掉 retry → 用户错过首次 popup 后 agent 无声卡死。频率改由 PopupCooldown
    // 兜底,不再靠"同 message 抑制"。
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::Pending),
        false,
        "Awaiting input",
        Some("Awaiting input"),
    ));
}

/// Pending → Pending 但 message 内容变了 → 新需求,允许弹(原行为保留)。
#[test]
fn should_show_popup_for_pending_to_pending_different_message() {
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::Pending),
        false,
        "Claude Code needs your approval for the plan",
        Some("Awaiting input"),
    ));
}

/// 边界:prev_message 是 None 的 Pending → Pending,仍允许弹(Pending 永远
/// eligible,不再依赖 prev_message 判定)。
#[test]
fn should_show_popup_pending_pending_when_prev_message_unknown() {
    assert!(stage_eligible(
        &SessionStatus::Pending,
        Some(&SessionStatus::Pending),
        false,
        "anything",
        None,
    ));
}

#[test]
fn should_not_show_popup_for_pending_to_running_transition() {
    assert!(!stage_eligible(
        &SessionStatus::Running,
        Some(&SessionStatus::Pending),
        false,
        "",
        None,
    ));
}

#[test]
fn should_show_popup_for_new_pending_session() {
    assert!(stage_eligible(
        &SessionStatus::Pending,
        None,
        true,
        "",
        None
    ));
}

#[test]
fn should_show_popup_for_new_idle_or_last_failed_session() {
    assert!(stage_eligible(&SessionStatus::Idle, None, true, "", None));
    assert!(stage_eligible(
        &SessionStatus::LastFailed,
        None,
        true,
        "",
        None
    ));
}
