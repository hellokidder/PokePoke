//! http_server 单测:状态层(SessionMemory 五槽位:dedupe / popup cooldown /
//! recently_visited / acked)。
//! 从 `http_server.rs` 的 `mod tests` 拆出(brief 20260727-split-http-server-rs
//! 第二阶段);W8(brief 20260813-session-memory)五结构合一后改写为
//! SessionMemory 等价断言,语义与断言不变。

use super::SessionMemory;
use crate::sessions::SessionStatus;
use std::time::Duration;

#[test]
fn dedupe_window_first_call_is_not_duplicate() {
    let mem = SessionMemory::default();
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_millis(100)));
}

#[test]
fn dedupe_window_second_call_within_window_is_duplicate() {
    let mem = SessionMemory::default();
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_millis(100)));
    assert!(mem.dedupe_is_duplicate("task-1", "stop", Duration::from_millis(100)));
}

#[test]
fn dedupe_window_different_task_id_is_not_duplicate() {
    let mem = SessionMemory::default();
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_millis(100)));
    assert!(!mem.dedupe_is_duplicate("task-2", "stop", Duration::from_millis(100)));
}

#[test]
fn dedupe_window_different_signature_is_not_duplicate() {
    let mem = SessionMemory::default();
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_millis(100)));
    assert!(!mem.dedupe_is_duplicate("task-1", "running", Duration::from_millis(100)));
}

#[test]
fn dedupe_window_after_window_expires_not_duplicate() {
    let mem = SessionMemory::default();
    // 用极小窗口模拟"已过窗口"场景
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_nanos(1)));
    std::thread::sleep(Duration::from_millis(2));
    assert!(!mem.dedupe_is_duplicate("task-1", "stop", Duration::from_nanos(1)));
}

#[test]
fn dedupe_window_state_machine_transition_then_back_is_not_duplicate() {
    // running → idle → running:第三次 running 距第一次 running 很近,
    // 但中间有 idle 改了 last_seen 的 signature,所以不算重复。
    let mem = SessionMemory::default();
    assert!(!mem.dedupe_is_duplicate("task-1", "running", Duration::from_millis(100)));
    assert!(!mem.dedupe_is_duplicate("task-1", "idle", Duration::from_millis(100)));
    assert!(!mem.dedupe_is_duplicate("task-1", "running", Duration::from_millis(100)));
}

#[test]
fn popup_cooldown_first_acquire_allowed() {
    let mem = SessionMemory::new(
        Duration::from_secs(30),
        Duration::from_secs(15),
        Duration::from_secs(3),
    );
    assert!(mem.popup_try_acquire("session-a"));
}

#[test]
fn popup_cooldown_second_acquire_within_window_blocked() {
    // 同 session_id 在冷却期内第二次 acquire 应该被挡:这是反复 transition
    // 导致同一 session 短时间反复弹 popup 的真实修复场景。
    let mem = SessionMemory::new(
        Duration::from_secs(30),
        Duration::from_secs(15),
        Duration::from_secs(3),
    );
    assert!(mem.popup_try_acquire("session-a"));
    assert!(!mem.popup_try_acquire("session-a"));
}

#[test]
fn popup_cooldown_after_window_expires_allowed_again() {
    // 用极短 cooldown 模拟"已过冷却期"
    let mem = SessionMemory::new(
        Duration::from_secs(30),
        Duration::from_nanos(1),
        Duration::from_secs(3),
    );
    assert!(mem.popup_try_acquire("session-a"));
    std::thread::sleep(Duration::from_millis(2));
    assert!(mem.popup_try_acquire("session-a"));
}

#[test]
fn popup_cooldown_independent_per_session() {
    // 不同 session_id 互不干扰 — 多个 cc / cursor session 各自的 popup
    // 不该被另一个 session 的 cooldown 挡住。
    let mem = SessionMemory::new(
        Duration::from_secs(30),
        Duration::from_secs(15),
        Duration::from_secs(3),
    );
    assert!(mem.popup_try_acquire("session-a"));
    assert!(mem.popup_try_acquire("session-b"));
    assert!(!mem.popup_try_acquire("session-a"));
    assert!(!mem.popup_try_acquire("session-b"));
}

#[test]
fn recently_visited_unmarked_session_not_recent() {
    let mem = SessionMemory::default();
    assert!(!mem.was_recently("session-a"));
}

#[test]
fn recently_visited_marked_session_within_window_is_recent() {
    let mem = SessionMemory::default();
    mem.mark_visited("session-a");
    assert!(mem.was_recently("session-a"));
}

#[test]
fn recently_visited_after_window_expires_no_longer_recent() {
    // 极短窗口 → 立刻"过期"
    let mem = SessionMemory::new(
        Duration::from_nanos(1),
        Duration::from_secs(3),
        Duration::from_secs(3),
    );
    mem.mark_visited("session-a");
    std::thread::sleep(Duration::from_millis(2));
    assert!(!mem.was_recently("session-a"));
}

#[test]
fn recently_visited_per_session_isolation() {
    let mem = SessionMemory::default();
    mem.mark_visited("session-a");
    assert!(mem.was_recently("session-a"));
    assert!(!mem.was_recently("session-b"));
}

#[test]
fn acked_stages_unmarked_session_not_acked() {
    let mem = SessionMemory::default();
    assert!(!mem.is_acked("session-a", &SessionStatus::Pending));
}

#[test]
fn acked_stages_mark_then_same_status_is_acked() {
    let mem = SessionMemory::default();
    mem.mark_acked("session-a", SessionStatus::Pending);
    assert!(mem.is_acked("session-a", &SessionStatus::Pending));
}

#[test]
fn acked_stages_different_status_not_acked() {
    // 用户认领的是 Pending,session 进入 Idle 时不该被挡。
    let mem = SessionMemory::default();
    mem.mark_acked("session-a", SessionStatus::Pending);
    assert!(!mem.is_acked("session-a", &SessionStatus::Idle));
    assert!(!mem.is_acked("session-a", &SessionStatus::LastFailed));
}

#[test]
fn acked_stages_mark_overwrites_previous_status() {
    // 用户先认领 Pending,后来 status 变 Idle 又点了一次 popup,ack 改成 Idle。
    // 此时再发 Pending 应该不挡(已不在记录里)。
    let mem = SessionMemory::default();
    mem.mark_acked("session-a", SessionStatus::Pending);
    mem.mark_acked("session-a", SessionStatus::Idle);
    assert!(mem.is_acked("session-a", &SessionStatus::Idle));
    assert!(!mem.is_acked("session-a", &SessionStatus::Pending));
}

#[test]
fn acked_stages_clear_removes_entry() {
    let mem = SessionMemory::default();
    mem.mark_acked("session-a", SessionStatus::Pending);
    mem.clear_acked("session-a");
    assert!(!mem.is_acked("session-a", &SessionStatus::Pending));
}

#[test]
fn acked_stages_per_session_isolation() {
    let mem = SessionMemory::default();
    mem.mark_acked("session-a", SessionStatus::Pending);
    assert!(mem.is_acked("session-a", &SessionStatus::Pending));
    assert!(!mem.is_acked("session-b", &SessionStatus::Pending));
}
