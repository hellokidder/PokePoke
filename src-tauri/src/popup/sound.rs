//! popup 主线程 dispatch 计时。
//!
//! 从 `popup.rs` 抽出:`run_on_main_timed`(把闭包派到主线程并测入队延迟,
//! 主线程卡顿告警)。提示音播放已迁到 `http_server::play_notify_sound`
//! (brief 20260805-pending-idle-completed-sound-swallowed,播音与 popup 创建解耦)。

use std::time::{Duration, Instant};

use tauri::AppHandle;

/// 主线程 dispatch 队列延迟超过这个阈值就 WARN。正常情况下派进主线程的闭包在
/// 下一个 AppKit tick(~ms 级)就执行;若延迟很大,说明主线程正被别的活儿堵住
/// (2026-07-01 挂死事故的直接指标)—— 这条 WARN 让“主线程卡了多久”在日志里
/// 一眼可见,不用再靠人肉配对 build/ok。
#[cfg(target_os = "macos")]
pub(super) const MAIN_THREAD_LAG_WARN: Duration = Duration::from_millis(1000);

/// 把闭包派到主线程执行,并测量**入队到开始执行**的延迟。这段延迟 = 主线程被
/// 前面的活儿占住的时长,是“主线程健康度”的直接读数。`op` 用于日志标识是哪个
/// AppKit 操作(build / destroy)。行为跟裸 `run_on_main_thread` 一致:已在主线程
/// 时同步执行、否则排到下一 tick。
#[cfg(target_os = "macos")]
pub(super) fn run_on_main_timed<F>(app: &AppHandle, op: &'static str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    let enqueued = Instant::now();
    let _ = app.run_on_main_thread(move || {
        let lag = enqueued.elapsed();
        if lag >= MAIN_THREAD_LAG_WARN {
            crate::log_warn!(
                "popup",
                "main-thread dispatch LAGGED op={} queue_ms={} (main thread was busy)",
                op,
                lag.as_millis(),
            );
        } else {
            crate::log_debug!(
                "popup",
                "main-thread dispatch op={} queue_ms={}",
                op,
                lag.as_millis(),
            );
        }
        f();
    });
}
