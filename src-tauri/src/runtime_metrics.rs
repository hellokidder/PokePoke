//! 进程内运行时指标 + 跨重启的轻量事件环,供诊断导出读取。
//!
//! 设计目标(对应 2026-07-16 用户 441% CPU 事故复盘):
//! - 用户重启 / 刚导出时 `pokepoke.log` 可能几乎为空,单靠 WARN 尾部定不了因。
//! - 在内存里累计 osascript / ghostty reconcile 计数,导出时一眼能看到 TIMEOUT 风暴。
//! - 关键事件追加到 `~/.pokepoke/diagnostics-events.jsonl`(无 session 正文),重启后仍能看见。

use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

const EVENTS_FILE: &str = "diagnostics-events.jsonl";
/// 事件环文件上限;超出时从头部裁掉,只留尾部。
const EVENTS_MAX_BYTES: u64 = 64 * 1024;

/// `/notify` 决策环文件(每次 notify 一条 JSON line,排障第一入口)。
/// 由 `http_server/decision_trace.rs` 写入、诊断导出读取。
/// 与 `EVENTS_FILE` 分开:决策记录比通用事件环大,且这是「为什么这条没弹」
/// 的第一支持入口,cap 放宽到 1MB。
pub const NOTIFY_DECISIONS_FILE: &str = "notify-decisions.jsonl";
/// 决策环上限(超出时裁头留尾)。
pub const NOTIFY_DECISIONS_MAX_BYTES: u64 = 1024 * 1024;
/// 诊断导出时最多纳入的决策 trace 条数。
pub const NOTIFY_DECISIONS_EXPORT_MAX: usize = 20;

static GLOBAL: OnceLock<Arc<RuntimeMetrics>> = OnceLock::new();

/// 在 `lib.rs` setup 最早阶段调用一次,供后台线程 / osascript 通过 [`global`] 写入。
pub fn install(metrics: Arc<RuntimeMetrics>) {
    let _ = GLOBAL.set(metrics);
}

pub fn global() -> Option<Arc<RuntimeMetrics>> {
    GLOBAL.get().cloned()
}

#[derive(Debug, Serialize, Clone)]
pub struct GhosttyReconcileSnapshot {
    pub last_scan_at: Option<String>,
    pub last_osascript_ok: Option<bool>,
    pub consecutive_failures: u32,
    pub current_sleep_secs: u64,
    pub ghostty_relevant_sessions: usize,
    pub last_written: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct OsascriptSnapshot {
    pub timeouts: u64,
    pub slow: u64,
    pub failed: u64,
    pub spawn_failed: u64,
    pub timeouts_by_op: BTreeMap<String, u32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PopupSnapshot {
    pub suppress_by_reason: BTreeMap<String, u32>,
    pub close_by_reason: BTreeMap<String, u32>,
    pub focus_gate_by_decision: BTreeMap<String, u32>,
}

/// 单个后台 watcher 的健康快照(由 supervisor monitor 周期写入)。
#[derive(Debug, Serialize, Clone)]
pub struct WatcherSnapshot {
    /// 距上次心跳的 wall-clock 时间。
    pub last_heartbeat_at: String,
    /// 注册时的 nominal 间隔(秒),停滞判定阈值 = max(3×, 90s)。
    pub nominal_interval_secs: u64,
    /// 快照时点距上次心跳的秒数(monitor tick 时点)。
    pub age_secs: u64,
    /// 历史停滞次数(同一停滞周期只计一次)。
    pub stalled_count: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct RuntimeMetricsSnapshot {
    pub uptime_secs: u64,
    pub active_popup_count: usize,
    pub osascript: OsascriptSnapshot,
    pub ghostty_reconcile: GhosttyReconcileSnapshot,
    pub popup: PopupSnapshot,
    /// 最近 N 条 /notify 全链路决策 trace(`~/.pokepoke/notify-decisions.jsonl`)。
    pub notify_decisions: Vec<String>,
    /// 后台 watcher 健康表(name → 心跳/停滞快照,supervisor 写入)。
    pub watchers: BTreeMap<String, WatcherSnapshot>,
}

pub struct RuntimeMetrics {
    started_at: Instant,
    osascript_timeouts: AtomicU64,
    osascript_slow: AtomicU64,
    osascript_failed: AtomicU64,
    osascript_spawn_failed: AtomicU64,
    timeouts_by_op: Mutex<BTreeMap<String, u32>>,
    ghostty: Mutex<GhosttyReconcileInner>,
    popup_suppress: Mutex<BTreeMap<String, u32>>,
    popup_close: Mutex<BTreeMap<String, u32>>,
    focus_gate: Mutex<BTreeMap<String, u32>>,
    watcher_heartbeats: Mutex<BTreeMap<String, WatcherSnapshot>>,
}

#[derive(Default)]
struct GhosttyReconcileInner {
    last_scan_at: Option<SystemTime>,
    last_osascript_ok: Option<bool>,
    consecutive_failures: u32,
    current_sleep_secs: u64,
    ghostty_relevant_sessions: usize,
    last_written: usize,
}

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            osascript_timeouts: AtomicU64::new(0),
            osascript_slow: AtomicU64::new(0),
            osascript_failed: AtomicU64::new(0),
            osascript_spawn_failed: AtomicU64::new(0),
            timeouts_by_op: Mutex::new(BTreeMap::new()),
            ghostty: Mutex::new(GhosttyReconcileInner::default()),
            popup_suppress: Mutex::new(BTreeMap::new()),
            popup_close: Mutex::new(BTreeMap::new()),
            focus_gate: Mutex::new(BTreeMap::new()),
            watcher_heartbeats: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn record_osascript_timeout(&self, op: &str) {
        self.osascript_timeouts.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut map) = self.timeouts_by_op.lock() {
            *map.entry(op.to_string()).or_insert(0) += 1;
        }
        append_event_line(&serde_json::json!({
            "kind": "osascript_timeout",
            "op": op,
        }));
    }

    pub fn record_osascript_slow(&self) {
        self.osascript_slow.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_osascript_failed(&self) {
        self.osascript_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_osascript_spawn_failed(&self) {
        self.osascript_spawn_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_popup_suppress(&self, reason: &str) {
        if let Ok(mut map) = self.popup_suppress.lock() {
            *map.entry(reason.to_string()).or_insert(0) += 1;
        }
        append_event_line(&serde_json::json!({
            "kind": "popup_suppress",
            "reason": reason,
        }));
    }

    pub fn record_popup_close(&self, reason: &str) {
        if let Ok(mut map) = self.popup_close.lock() {
            *map.entry(reason.to_string()).or_insert(0) += 1;
        }
        append_event_line(&serde_json::json!({
            "kind": "popup_close",
            "reason": reason,
        }));
    }

    pub fn record_focus_gate(&self, decision: &str) {
        if let Ok(mut map) = self.focus_gate.lock() {
            *map.entry(decision.to_string()).or_insert(0) += 1;
        }
    }

    /// supervisor monitor 上报停滞:计数 +1 并写事件环。
    pub fn record_watcher_stall(&self, name: &str) {
        if let Ok(mut map) = self.watcher_heartbeats.lock() {
            let e = map
                .entry(name.to_string())
                .or_insert_with(|| WatcherSnapshot {
                    last_heartbeat_at: String::new(),
                    nominal_interval_secs: 0,
                    age_secs: 0,
                    stalled_count: 0,
                });
            e.stalled_count += 1;
        }
        append_event_line(&serde_json::json!({
            "kind": "watcher_stall",
            "name": name,
        }));
    }

    /// supervisor monitor 每 tick 推送的最新健康快照(healthy 也刷新)。
    pub fn update_watcher_snapshot(&self, name: &str, snapshot: WatcherSnapshot) {
        if let Ok(mut map) = self.watcher_heartbeats.lock() {
            map.insert(name.to_string(), snapshot);
        }
    }

    pub fn record_ghostty_scan(
        &self,
        osascript_ok: bool,
        ghostty_relevant_sessions: usize,
        written: usize,
        current_sleep_secs: u64,
    ) {
        let mut g = self.ghostty.lock().unwrap_or_else(|p| p.into_inner());
        g.last_scan_at = Some(SystemTime::now());
        g.last_osascript_ok = Some(osascript_ok);
        g.ghostty_relevant_sessions = ghostty_relevant_sessions;
        g.last_written = written;
        g.current_sleep_secs = current_sleep_secs;
        if osascript_ok {
            g.consecutive_failures = 0;
        } else {
            g.consecutive_failures = g.consecutive_failures.saturating_add(1);
            append_event_line(&serde_json::json!({
                "kind": "ghostty_reconcile_osascript_fail",
                "consecutive": g.consecutive_failures,
                "sleep_secs": current_sleep_secs,
            }));
        }
    }

    pub fn snapshot(&self, active_popup_count: usize) -> RuntimeMetricsSnapshot {
        let timeouts_by_op = self
            .timeouts_by_op
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let g = self.ghostty.lock().unwrap_or_else(|p| p.into_inner());
        let popup_suppress = self
            .popup_suppress
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let popup_close = self
            .popup_close
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let focus_gate = self
            .focus_gate
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let watchers = self
            .watcher_heartbeats
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        RuntimeMetricsSnapshot {
            uptime_secs: self.uptime_secs(),
            active_popup_count,
            osascript: OsascriptSnapshot {
                timeouts: self.osascript_timeouts.load(Ordering::Relaxed),
                slow: self.osascript_slow.load(Ordering::Relaxed),
                failed: self.osascript_failed.load(Ordering::Relaxed),
                spawn_failed: self.osascript_spawn_failed.load(Ordering::Relaxed),
                timeouts_by_op,
            },
            ghostty_reconcile: GhosttyReconcileSnapshot {
                last_scan_at: g.last_scan_at.map(fmt_systime),
                last_osascript_ok: g.last_osascript_ok,
                consecutive_failures: g.consecutive_failures,
                current_sleep_secs: g.current_sleep_secs,
                ghostty_relevant_sessions: g.ghostty_relevant_sessions,
                last_written: g.last_written,
            },
            popup: PopupSnapshot {
                suppress_by_reason: popup_suppress,
                close_by_reason: popup_close,
                focus_gate_by_decision: focus_gate,
            },
            notify_decisions: read_recent_jsonl(
                &notify_decisions_path(),
                NOTIFY_DECISIONS_EXPORT_MAX,
            ),
            watchers,
        }
    }
}

fn fmt_systime(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

/// `~/.pokepoke/<file_name>` 路径。
fn pokepoke_path(file_name: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".pokepoke").join(file_name)
}

/// `/notify` 决策环文件路径。test 构建下可用 `POKEPOKE_TEST_NOTIFY_DECISIONS`
/// 重定向(cfg(test) 专属,非产品开关),避免污染真实决策环。
pub(crate) fn notify_decisions_path() -> PathBuf {
    #[cfg(test)]
    if let Ok(p) = std::env::var("POKEPOKE_TEST_NOTIFY_DECISIONS") {
        return PathBuf::from(p);
    }
    pokepoke_path(NOTIFY_DECISIONS_FILE)
}

fn events_path() -> PathBuf {
    pokepoke_path(EVENTS_FILE)
}

/// 追加一行 JSONL 到 `path`(超出 `max_bytes` 时裁头留尾)。事件环 / 决策环
/// 共用;失败静默 —— 观测路径绝不能影响主链路。调用方负责拼好整行(含 ts)。
pub(crate) fn append_jsonl(path: &Path, line: &str, max_bytes: u64) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
    trim_jsonl_file(path, max_bytes);
}

/// 裁剪 JSONL 文件到 `max_bytes` 内(保留尾部)。失败静默。
pub(crate) fn trim_jsonl_file(path: &Path, max_bytes: u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() <= max_bytes {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let keep_from = content.len().saturating_sub(max_bytes as usize / 2);
    let trimmed = content[keep_from..]
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(&content[keep_from..]);
    let _ = std::fs::write(path, trimmed);
}

/// 读 JSONL 文件尾部,供诊断导出。返回原始 JSON 行(HOME 不脱敏 ——
/// 事件 / 决策环里不含路径)。文件不存在 / 读失败返回空。
pub(crate) fn read_recent_jsonl(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    if lines.len() <= max_lines {
        lines
    } else {
        lines[lines.len() - max_lines..].to_vec()
    }
}

fn append_event_line(body: &serde_json::Value) {
    let path = events_path();
    let mut obj = body.as_object().cloned().unwrap_or_default();
    let ts = chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        .to_string();
    obj.insert("ts".to_string(), serde_json::Value::String(ts));
    let line = format!("{}\n", serde_json::Value::Object(obj));
    append_jsonl(&path, &line, EVENTS_MAX_BYTES);
}

#[cfg(test)]
fn trim_events_file(path: &Path) {
    trim_jsonl_file(path, EVENTS_MAX_BYTES);
}

/// 读事件环尾部,供诊断导出。返回原始 JSON 行(HOME 不脱敏 —— 事件里不含路径)。
pub fn read_recent_events(max_lines: usize) -> Vec<String> {
    read_recent_jsonl(&events_path(), max_lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn osascript_timeout_increments_by_op() {
        let m = RuntimeMetrics::new();
        m.record_osascript_timeout("enumerate");
        m.record_osascript_timeout("enumerate");
        m.record_osascript_timeout("focus_check");
        let snap = m.snapshot(0);
        assert_eq!(snap.osascript.timeouts, 3);
        assert_eq!(snap.osascript.timeouts_by_op.get("enumerate"), Some(&2));
    }

    #[test]
    fn ghostty_consecutive_failures_reset_on_ok() {
        let m = RuntimeMetrics::new();
        m.record_ghostty_scan(false, 2, 0, 16);
        m.record_ghostty_scan(false, 2, 0, 32);
        m.record_ghostty_scan(true, 2, 1, 8);
        let snap = m.snapshot(0);
        assert_eq!(snap.ghostty_reconcile.consecutive_failures, 0);
        assert_eq!(snap.ghostty_reconcile.last_written, 1);
    }

    #[test]
    fn popup_metrics_count_suppress_close_and_focus_gate() {
        let m = RuntimeMetrics::new();
        m.record_focus_gate("show");
        m.record_focus_gate("suppress_silent");
        m.record_popup_suppress("already_focused");
        m.record_popup_suppress("already_focused");
        m.record_popup_close("focus_watcher");
        let snap = m.snapshot(2);
        assert_eq!(snap.popup.focus_gate_by_decision.get("show"), Some(&1));
        assert_eq!(
            snap.popup.focus_gate_by_decision.get("suppress_silent"),
            Some(&1)
        );
        assert_eq!(
            snap.popup.suppress_by_reason.get("already_focused"),
            Some(&2)
        );
        assert_eq!(snap.popup.close_by_reason.get("focus_watcher"), Some(&1));
        assert_eq!(snap.active_popup_count, 2);
    }

    #[test]
    fn trim_events_file_keeps_tail() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("pokepoke-metrics-trim");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create_dir_all 失败");
        let path = dir.join("events.jsonl");
        let big = "x".repeat((EVENTS_MAX_BYTES + 4096) as usize);
        std::fs::write(&path, &big).expect("fs::write 失败");
        trim_events_file(&path);
        let len = std::fs::metadata(&path).expect("fs::metadata 失败").len();
        assert!(len <= EVENTS_MAX_BYTES);
    }

    #[test]
    fn append_jsonl_trims_to_cap_keeping_tail() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("pokepoke-metrics-append-trim");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create_dir_all 失败");
        let path = dir.join("ring.jsonl");
        // 小 cap 快速验证:行 101 字节,cap 1024 → 写 30 行后应裁头留尾。
        let line = format!("{}\n", "x".repeat(100));
        for _ in 0..30 {
            append_jsonl(&path, &line, 1024);
        }
        let len = std::fs::metadata(&path).expect("fs::metadata 失败").len();
        assert!(len <= 1024, "ring file {len} bytes exceeds cap");
        let content = std::fs::read_to_string(&path).expect("read_to_string 应可读");
        // 尾部保留:最后一行必须是完整行。
        assert!(content.ends_with(&line));
    }
}
