//! staging 派发(响应路径与 popup gate 解耦)的行为验证 —— brief
//! `docs/briefs/20261013-focus-off-response-path.md`。
//!
//! 生产走 `StagingDispatcher::spawn_worker`(专用线程 FIFO),测试默认 `inline`
//! (其余 900+ 用例因此保持"process_notify 返回即可断言副作用"的同步语义)。
//! 本文件专测 worker 模式的三条契约:
//!
//! 1. **响应不等 gate**:focus 探测(生产里是 osascript,实测 0.2–1.9s)没跑完,
//!    `/notify` 也必须已经返回;
//! 2. **gate 一定会跑完并落 trace**:请求早已结束、甚至客户端已断开,staging 仍
//!    必须把 gate 判定 + popup/sound 副作用 + 决策 trace 做完(这正是不能改成
//!    hook fire-and-forget 的原因,见 brief 里的 0/6 实测);
//! 3. **FIFO 串行**:多个请求的 gate 按派发顺序执行,不因并发而翻转(pending→idle
//!    这类紧凑序列依赖先后次序)。

use super::tests::test_state_with;
use super::tests::{redirect_trace_file, trace_lines_for, TRACE_TEST_LOCK};
use super::{process_notify, AppState, NotifyRequest, StagingDispatcher};
use crate::popup::{FocusOracle, PopupController};
use crate::sessions::Session;
use crate::surface::SurfaceEvidence;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

// ==================== stubs ====================

type Gate = Arc<(Mutex<bool>, Condvar)>;

fn new_gate() -> Gate {
    Arc::new((Mutex::new(false), Condvar::new()))
}

fn release(gate: &Gate) {
    let (lock, cv) = &**gate;
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
    cv.notify_all();
}

fn wait_on_gate(gate: &Gate) {
    let (lock, cv) = &**gate;
    let mut released = lock.lock().unwrap_or_else(|e| e.into_inner());
    while !*released {
        released = cv.wait(released).unwrap_or_else(|e| e.into_inner());
    }
}

/// 挡住 focus 判定:模拟生产里那段 osascript 探测(200ms–1.9s)。
struct GatedFocusOracle {
    gate: Gate,
}

impl FocusOracle for GatedFocusOracle {
    fn is_focused_resolved(
        &self,
        _evidence: &SurfaceEvidence,
        _resolved: &crate::surface::ResolvedSurface,
    ) -> bool {
        wait_on_gate(&self.gate);
        false
    }
}

/// 只挡第一次 focus 判定:用来区分"FIFO 串行"与"每请求并发"。
/// 若实现是并发的,第二个请求会绕过 gate 先完成 → 记录顺序翻转 → 用例失败。
struct FirstCallGatedFocusOracle {
    calls: AtomicUsize,
    gate: Gate,
    /// 第一次判定已进入(供测试确认 worker 已取走第一个 job)。
    entered: Arc<AtomicBool>,
}

impl FocusOracle for FirstCallGatedFocusOracle {
    fn is_focused_resolved(
        &self,
        _evidence: &SurfaceEvidence,
        _resolved: &crate::surface::ResolvedSurface,
    ) -> bool {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.store(true, Ordering::SeqCst);
            wait_on_gate(&self.gate);
        }
        false
    }
}

/// 记录 `show` 顺序的 popup stub。
struct RecordingPopup {
    shown: Arc<Mutex<Vec<String>>>,
}

impl RecordingPopup {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let shown = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                shown: shown.clone(),
            },
            shown,
        )
    }
}

impl PopupController for RecordingPopup {
    fn show(&self, session: &Session) -> bool {
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(session.task_id.clone());
        true
    }
    fn close(&self, _session_id: &str, _reason: crate::popup::PopupCloseReason) {}
    fn has_session_popup(&self, _session_id: &str) -> bool {
        false
    }
}

// ==================== helpers ====================

/// 与 tests.rs::req_with 同形(那边是文件私有,这里复制一份最小版本)。
fn req_with(task_id: &str, event_type: &str, status: &str) -> NotifyRequest {
    NotifyRequest {
        protocol_version: None,
        title: "t".into(),
        message: "m".into(),
        task_id: task_id.into(),
        external_session_id: None,
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some(event_type.into()),
        status: Some(status.into()),
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: None,
        herdr_workspace_id: None,
        herdr_socket_path: None,
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    }
}

/// worker 模式的 AppState:gate 用注入的 focus oracle 顶住。
fn worker_state(popup: Arc<dyn PopupController>, focus: Arc<dyn FocusOracle>) -> AppState {
    let mut state = test_state_with(popup);
    state.focus = focus;
    state.staging = StagingDispatcher::spawn_worker(state.clone());
    state
}

fn poll_until<F: Fn() -> bool>(deadline: Duration, cond: F) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}

fn shown_len(shown: &Arc<Mutex<Vec<String>>>) -> usize {
    shown.lock().unwrap_or_else(|e| e.into_inner()).len()
}

// ==================== tests ====================

/// 契约 1 + 2:响应不等 gate;gate 放行后仍会跑完并落 trace。
#[test]
fn worker_mode_returns_before_gate_and_finishes_it_afterwards() {
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let trace_path = redirect_trace_file();

    let gate = new_gate();
    let (popup, shown) = RecordingPopup::new();
    let state = worker_state(
        Arc::new(popup),
        Arc::new(GatedFocusOracle { gate: gate.clone() }),
    );

    let started = Instant::now();
    let (code, _body) = process_notify(&state, req_with("cc-stg-1", "Stop", "idle"));
    let elapsed = started.elapsed();

    // 1. gate 还卡着,/notify 必须已经返回。
    assert_eq!(code, axum::http::StatusCode::CREATED);
    assert!(
        elapsed < Duration::from_millis(200),
        "响应不得等 focus 探测,实际 {elapsed:?}"
    );
    assert_eq!(shown_len(&shown), 0, "gate 未放行前不应有 popup");
    assert!(
        trace_lines_for(&trace_path, "cc-stg-1").is_empty(),
        "gate 未跑完前不应落 trace(gate verdict 还未产生)"
    );

    // 2. 放行后 staging 仍会完成 —— 请求早已返回,与请求生命周期无关。
    release(&gate);
    assert!(
        poll_until(Duration::from_secs(3), || shown_len(&shown) == 1),
        "放行后 worker 必须完成 staging"
    );
    assert!(
        poll_until(Duration::from_secs(3), || !trace_lines_for(
            &trace_path,
            "cc-stg-1"
        )
        .is_empty()),
        "worker 必须落一条决策 trace"
    );
    let lines = trace_lines_for(&trace_path, "cc-stg-1");
    assert_eq!(lines.len(), 1, "每个 /notify 恰好一条 trace:{lines:#?}");
    assert!(
        lines[0].contains("\"stage_eligible\":true"),
        "Stop→Idle 新 session 应 stage eligible:{:#?}",
        lines[0]
    );
}

/// 契约 3:多个请求的 gate 按派发顺序 FIFO 执行。
#[test]
fn worker_mode_runs_gates_in_fifo_order() {
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = redirect_trace_file();

    let gate = new_gate();
    let entered = Arc::new(AtomicBool::new(false));
    let (popup, shown) = RecordingPopup::new();
    let state = worker_state(
        Arc::new(popup),
        Arc::new(FirstCallGatedFocusOracle {
            calls: AtomicUsize::new(0),
            gate: gate.clone(),
            entered: entered.clone(),
        }),
    );

    // 第一个请求卡在 focus gate 上;等 worker 真的进入第一个 job 后再派发第二个
    // —— 若实现是每请求并发,第二个会先完成并把顺序翻转。
    process_notify(&state, req_with("cc-stg-f1", "Stop", "idle"));
    assert!(
        poll_until(Duration::from_secs(2), || entered.load(Ordering::SeqCst)),
        "worker 应已开始第一个 job 的 gate 判定"
    );
    process_notify(&state, req_with("cc-stg-f2", "Stop", "idle"));
    release(&gate);

    assert!(
        poll_until(Duration::from_secs(3), || shown_len(&shown) == 2),
        "两个请求都应完成 staging"
    );
    let order = shown.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(
        order,
        vec!["cc-stg-f1".to_string(), "cc-stg-f2".to_string()],
        "gate 必须按派发顺序串行执行"
    );
}

/// 对照:inline 模式保持旧语义(响应等 gate 跑完),仅作行为文档与回归护栏。
#[test]
fn inline_mode_still_waits_for_gate() {
    let _g = TRACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = redirect_trace_file();

    let gate = new_gate();
    let (popup, shown) = RecordingPopup::new();
    let mut state = test_state_with(Arc::new(popup));
    state.focus = Arc::new(GatedFocusOracle { gate: gate.clone() });
    state.staging = StagingDispatcher::inline();

    let releaser = {
        let gate = gate.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            release(&gate);
        })
    };

    let started = Instant::now();
    let _ = process_notify(&state, req_with("cc-stg-inline", "Stop", "idle"));
    let elapsed = started.elapsed();
    releaser.join().expect("releaser thread");

    assert!(
        elapsed >= Duration::from_millis(200),
        "inline 模式应在响应前等 gate 跑完,实际 {elapsed:?}"
    );
    assert_eq!(shown_len(&shown), 1, "inline 模式返回时 popup 已产生");
}
