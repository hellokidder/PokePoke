//! 后台 watcher 统一监督:命名线程 + catch_unwind 兜底 + 心跳停滞检测。
//! 架构增强 W3,brief 20260813-watcher-supervisor。
//!
//! 背景:15 个后台 watcher,任何一个 panic / 卡死后 = 静默功能丧失(panic hook
//! 只落盘不重启,用户侧表现为"提醒不再出现"且无告警)。本模块提供:
//! - 命名线程(`std::thread::Builder::name`)—— 崩溃报告 / ps 可辨;
//! - `catch_unwind` 兜底 —— body panic 记 ERROR 后线程继续下一轮,不退出;
//! - 心跳停滞检测 —— 单 monitor 线程周期扫描 REGISTRY,停滞 = 距上次心跳超过
//!   `max(3 × nominal_interval, 90s)` → log_warn + metrics 计数 + 事件环。
//!
//! 不做自动重启(编排者裁决:catch_unwind 已把"panic 杀线程"转为"线程存活";
//! 卡死类(死锁 / osascript 挂起)盲目重启会叠加副作用 —— restart 是 followup)。
//!
//! 两种迁移模式:
//! - [`spawn_periodic`]:固定间隔 watcher,helper 负责 sleep → 心跳 → 执行单轮。
//! - [`spawn_loop_guarded`]:自适应退避 watcher(循环体自带 sleep),helper 只包
//!   catch_unwind,body 内部每轮自行 `heartbeat(entry)`。

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// 停滞判定阈值:距上次心跳超过 `max(3 × nominal_interval, 90s)` 视为停滞。
/// 90s 下限避免小间隔 watcher(5s)在正常 sleep + 偶发慢轮时误报。
fn stall_threshold(interval: Duration) -> Duration {
    let triple = interval.saturating_mul(3);
    if triple > Duration::from_secs(90) {
        triple
    } else {
        Duration::from_secs(90)
    }
}

/// 单个 watcher 的监督条目(register 时创建,心跳 / 停滞计数由 supervisor 更新)。
pub struct WatcherEntry {
    name: &'static str,
    nominal_interval: Duration,
    /// `(monotonic, wall_clock)` 双时钟:停滞判定用 monotonic(不受系统改钟影响),
    /// 诊断导出用 wall clock 字符串。
    last_heartbeat: Mutex<(Instant, SystemTime)>,
    stalled_count: AtomicU64,
    /// 同一停滞周期只告警一次;心跳恢复后复位。
    last_stall_reported: AtomicBool,
}

impl WatcherEntry {
    fn new(name: &'static str, nominal_interval: Duration) -> Self {
        let now = Instant::now();
        Self {
            name,
            nominal_interval,
            last_heartbeat: Mutex::new((now, SystemTime::now())),
            stalled_count: AtomicU64::new(0),
            last_stall_reported: AtomicBool::new(false),
        }
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<&'static str, Arc<WatcherEntry>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<&'static str, Arc<WatcherEntry>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 手动模式:注册 watcher 并创建监督条目(自适应退避 watcher 用)。
/// 注册时记录"本轮开始"为首次心跳。
pub fn register(name: &'static str, interval: Duration) -> Arc<WatcherEntry> {
    let entry = Arc::new(WatcherEntry::new(name, interval));
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(name, entry.clone());
    entry
}

/// 手动模式每轮调用:更新心跳(停滞判定基准 + 诊断快照),恢复时复位告警位。
pub fn heartbeat(entry: &WatcherEntry) {
    let now = Instant::now();
    let sys = SystemTime::now();
    if let Ok(mut hb) = entry.last_heartbeat.lock() {
        *hb = (now, sys);
    }
    entry.last_stall_reported.store(false, Ordering::Relaxed);
}

/// 停滞判定纯函数(注入 now 便于单测)。
pub fn is_stalled(entry: &WatcherEntry, now: Instant) -> bool {
    let last = entry.last_heartbeat.lock().map(|hb| hb.0).unwrap_or(now);
    now.duration_since(last) > stall_threshold(entry.nominal_interval)
}

/// 固定间隔周期任务:命名线程,每 `interval` 执行一轮 —— sleep → 心跳 →
/// catch_unwind(body)。body panic 记 ERROR 后下一轮继续,线程永不退出。
pub fn spawn_periodic(name: &'static str, interval: Duration, body: impl FnMut() + Send + 'static) {
    let entry = register(name, interval);
    let thread_name = name.to_string();
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let mut body = body;
            loop {
                std::thread::sleep(interval);
                heartbeat(&entry);
                if let Err(payload) = catch_unwind(AssertUnwindSafe(&mut body)) {
                    crate::log_error!(
                        "supervisor",
                        "watcher body panicked name={} payload={}",
                        name,
                        crate::logger::panic_payload_text(&*payload)
                    );
                }
            }
        })
        .expect("spawn supervisor periodic watcher");
}

/// 自适应退避 watcher:循环体自带 sleep(interval 仅作 nominal 停滞阈值),
/// helper 只负责命名线程 + 每轮 catch_unwind;body 内部每轮自行 `heartbeat(entry)`。
/// body panic 后从初始状态重启循环,线程永不退出。
pub fn spawn_loop_guarded(
    name: &'static str,
    interval: Duration,
    body: impl FnMut(&Arc<WatcherEntry>) + Send + 'static,
) {
    let entry = register(name, interval);
    let thread_name = name.to_string();
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let mut body = body;
            loop {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| body(&entry))) {
                    crate::log_error!(
                        "supervisor",
                        "watcher loop panicked name={} payload={}",
                        name,
                        crate::logger::panic_payload_text(&*payload)
                    );
                }
            }
        })
        .expect("spawn supervisor guarded watcher");
}

/// 单 monitor 线程:每 `tick` 扫描 REGISTRY,停滞 → 告警(同一停滞周期只记一次)
/// + stalled_count + metrics 计数 + 事件环;并把最新快照推给 runtime_metrics
/// (healthy watcher 也刷新,诊断导出随时可见)。
pub fn spawn_monitor(tick: Duration) {
    let thread_name = "supervisor-monitor".to_string();
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || loop {
            std::thread::sleep(tick);
            let now = Instant::now();
            let entries: Vec<Arc<WatcherEntry>> = registry()
                .lock()
                .map(|g| g.values().cloned().collect())
                .unwrap_or_default();
            for entry in entries {
                let name = entry.name;
                let stalled = is_stalled(&entry, now);
                if stalled && !entry.last_stall_reported.load(Ordering::Relaxed) {
                    entry.last_stall_reported.store(true, Ordering::Relaxed);
                    entry.stalled_count.fetch_add(1, Ordering::Relaxed);
                    let age_secs = now
                        .duration_since(entry.last_heartbeat.lock().map(|hb| hb.0).unwrap_or(now))
                        .as_secs();
                    crate::log_warn!(
                        "supervisor",
                        "watcher stalled name={} age_s={} nominal_interval_s={}",
                        name,
                        age_secs,
                        entry.nominal_interval.as_secs(),
                    );
                    if let Some(metrics) = crate::runtime_metrics::global() {
                        metrics.record_watcher_stall(name);
                    }
                }
                if let Some(metrics) = crate::runtime_metrics::global() {
                    metrics.update_watcher_snapshot(name, snapshot_of(&entry, now));
                }
            }
        })
        .expect("spawn supervisor monitor");
}

/// 取当前监督快照(供 monitor 推给 runtime_metrics / 测试断言)。
fn snapshot_of(entry: &WatcherEntry, now: Instant) -> crate::runtime_metrics::WatcherSnapshot {
    let (mono, wall) = entry
        .last_heartbeat
        .lock()
        .map(|hb| *hb)
        .unwrap_or((now, SystemTime::now()));
    crate::runtime_metrics::WatcherSnapshot {
        last_heartbeat_at: chrono::DateTime::<chrono::Local>::from(wall)
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        nominal_interval_secs: entry.nominal_interval.as_secs(),
        age_secs: now.duration_since(mono).as_secs(),
        stalled_count: entry.stalled_count.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn entry_with(interval: Duration, age: Duration) -> WatcherEntry {
        let e = WatcherEntry::new("test", interval);
        if let Ok(mut hb) = e.last_heartbeat.lock() {
            hb.0 = Instant::now() - age;
        }
        e
    }

    #[test]
    fn is_stalled_within_interval_is_false() {
        // 5s watcher:阈值 = max(15s, 90s) = 90s;10s 心跳龄不滞后。
        let e = entry_with(Duration::from_secs(5), Duration::from_secs(10));
        assert!(!is_stalled(&e, Instant::now()));
    }

    #[test]
    fn is_stalled_beyond_three_times_interval_is_true() {
        // 100s watcher:阈值 = 300s;301s 心跳龄 → 停滞。
        let e = entry_with(Duration::from_secs(100), Duration::from_secs(301));
        assert!(is_stalled(&e, Instant::now()));
        // 200s(3 倍内)不滞后。
        let e2 = entry_with(Duration::from_secs(100), Duration::from_secs(200));
        assert!(!is_stalled(&e2, Instant::now()));
    }

    #[test]
    fn heartbeat_resets_stall_and_recovery_flag() {
        let e = entry_with(Duration::from_secs(100), Duration::from_secs(400));
        assert!(is_stalled(&e, Instant::now()));
        e.last_stall_reported.store(true, Ordering::Relaxed);
        heartbeat(&e);
        assert!(!is_stalled(&e, Instant::now()), "心跳后应恢复");
        assert!(
            !e.last_stall_reported.load(Ordering::Relaxed),
            "恢复后告警位应复位"
        );
    }

    #[test]
    fn stall_threshold_has_90s_floor() {
        assert_eq!(
            stall_threshold(Duration::from_secs(5)),
            Duration::from_secs(90)
        );
        assert_eq!(
            stall_threshold(Duration::from_secs(8)),
            Duration::from_secs(90)
        );
        assert_eq!(
            stall_threshold(Duration::from_secs(100)),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn spawn_periodic_survives_panic_and_continues_next_round() {
        let (tx, rx) = mpsc::channel();
        let mut rounds = 0u32;
        spawn_periodic("test-periodic", Duration::from_millis(20), move || {
            rounds += 1;
            if rounds == 1 {
                panic!("intentional test panic");
            }
            let _ = tx.send(rounds);
        });
        let received = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("第二轮的 channel 信号应在超时内到达");
        assert_eq!(received, 2, "第一轮 panic 后线程应存活并执行第二轮");
    }

    #[test]
    fn registry_concurrent_register_and_heartbeat_smoke() {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    let name = Box::leak(format!("smoke-{i}").into_boxed_str());
                    let entry = register(name, Duration::from_secs(5));
                    for _ in 0..20 {
                        heartbeat(&entry);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("watcher 线程应正常结束");
        }
        let reg = registry().lock().unwrap_or_else(|p| p.into_inner());
        for i in 0..8 {
            let name = format!("smoke-{i}");
            let entry = reg
                .get(name.as_str())
                .unwrap_or_else(|| panic!("{name} 应已注册"));
            assert!(
                !is_stalled(entry, Instant::now()),
                "{name} 刚心跳过不应停滞"
            );
        }
    }
}
