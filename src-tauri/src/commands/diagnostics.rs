//! 诊断信息导出(roadmap P2)。
//!
//! 一键收集**脱敏后**的诊断报告写到 `~/.pokepoke/diagnostics-<ts>.json`,方便用户
//! 贴给维护者排障。核心硬约束:**绝不导出敏感数据**。
//!
//! 报告的设计目标是"能定位真问题",所以除了静态体检项,还采集**运行时事实**:
//! - server UDS sock(`~/.pokepoke/sock`)是否存在 / 是否有实例在监听
//!   —— "通知静默"最常见原因(server 没起 / sock stale)。
//! - 三端外部配置文件(claude/codex/cursor)的存在性 / 大小 / 修改时间
//!   —— drift 检测背后的事实依据,而非只有一个布尔。
//! - 部署的 hook 二进制与 bundle 内 hook 是否**逐字节一致**
//!   —— B 类失效"协议升级后用户没重启,旧 hook 还在跑"的直接证据。
//! - 数据文件(sessions/settings/log)的大小 + mtime —— 看 store 是否在更新。
//!
//! 脱敏边界(对照 `settings.rs` / `sessions.rs` 的敏感字段):
//! - session 的 `message` / `title` / `workspace_path` / `terminal_tty` /
//!   `transcript_path` / 各类 id —— 不导单条内容,只导状态 / 来源**计数**。
//! - 所有路径经 `tilde_collapse` 把 `$HOME` 折叠成 `~`,不暴露真实用户名。
//! - 外部 / 数据文件只采集**元信息**(存在性 / 字节数 / mtime),**绝不读内容**。
//! - 集成检测结果走**字段白名单**(只取布尔位),不整段透传 hook JSON。
//! - **不打包** `pokepoke.log` 原始日志(可能含 workspace 路径 / message 原文)。

use crate::commands::integration;
use crate::popup::PopupList;
use crate::runtime_metrics::{self, RuntimeMetrics, RuntimeMetricsSnapshot};
use crate::sessions::{Session, SessionStatus, SessionStore};
use crate::settings::{Settings, SettingsStore};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{AppHandle, Manager, State};

const SCHEMA_VERSION: u32 = 6;

// schema v6(2026-08-13,架构增强 W3):`health.metrics` 增加 `watchers` ——
// 后台 watcher 健康表(name → last_heartbeat_at / nominal_interval_secs /
// age_secs / stalled_count,由 supervisor monitor 周期写入),定位"watcher
// 静默死亡 / 停滞"型漏提醒。
// schema v5(2026-08-13,架构增强 W2):`health.metrics` 增加
// `notify_decisions` —— 最近 20 条 /notify 全链路决策 trace
// (`~/.pokepoke/notify-decisions.jsonl`,1MB cap),排障「为什么这条没弹」。

/// 日志尾部最多纳入多少行 WARN/ERROR。
const LOG_TAIL_MAX: usize = 80;
/// 运维相关日志(含轮转备份)最多纳入多少行。
const OPERATIONAL_LOG_MAX: usize = 60;
/// 跨重启事件环最多导出多少行。
const EVENT_RING_MAX: usize = 40;
/// 进程 uptime 低于此值时在 hints 里提示「可能刚重启,现场证据不足」。
const SHORT_UPTIME_HINT_SECS: u64 = 300;

#[derive(Serialize)]
pub struct DiagnosticsReport {
    schema_version: u32,
    generated_at: String,
    app: AppInfo,
    runtime: RuntimeInfo,
    health: HealthInfo,
    integration: IntegrationInfo,
    settings: SettingsDigest,
    sessions: SessionsDigest,
    /// 给阅读者的固定说明(说明哪些东西被刻意排除)。
    notes: Vec<String>,
}

#[derive(Serialize)]
struct AppInfo {
    version: String,
    os: String,
    arch: String,
    debug_logging: bool,
    /// 是否 debug 构建(`cargo build` 未加 --release)。dev 模式下 bundled hook
    /// 指向 `target/debug/pokehook`,与 deployed 常不一致,`matches_bundled`
    /// 的 false 多为 dev 误报;生产 release 构建下才是真信号。
    dev_build: bool,
}

/// 文件元信息 —— 只采集存在性 / 字节数 / mtime,**绝不读内容**。
#[derive(Serialize)]
struct FileFacts {
    path: String,
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified: Option<String>,
}

#[derive(Serialize)]
struct RuntimeInfo {
    pokepoke_dir: String,
    /// hook ↔ server 的 UDS sock(`~/.pokepoke/sock`)文件元信息。
    server_socket: FileFacts,
    /// 能否连上 sock —— true 说明有 PokePoke 实例在监听(server 活着)。
    server_responding: bool,
    /// 写前备份(snapshots/)文件数,反映三端配置被改写过几次。
    snapshots_count: usize,
    /// sessions.json / settings.json / pokepoke.log 的元信息(不含内容)。
    data_files: Vec<FileFacts>,
    /// 最近若干行 **WARN/ERROR** 级日志(HOME 已折叠成 ~)。
    recent_log_warnings: Vec<String>,
    /// 轮转备份 `pokepoke.log.1` 的元信息(若存在)。
    #[serde(skip_serializing_if = "Option::is_none")]
    rotated_log: Option<FileFacts>,
    /// 从当前日志 + 轮转备份里筛出的运维相关行(osascript / ghostty / popup 等)。
    recent_operational_log: Vec<String>,
}

#[derive(Serialize)]
struct HealthInfo {
    /// 自本次进程启动以来的秒数。
    uptime_secs: u64,
    /// 当前屏上 popup 窗口数(不含 label / session 内容)。
    active_popup_count: usize,
    /// 自启动以来 osascript / ghostty reconcile 累计指标。
    metrics: RuntimeMetricsSnapshot,
    /// 跨重启保留的轻量事件环(`~/.pokepoke/diagnostics-events.jsonl`)。
    recent_events: Vec<String>,
    /// 维护者读的自动推断提示(非敏感)。
    analysis_hints: Vec<String>,
}

#[derive(Serialize)]
struct HookBinaryInfo {
    /// 部署到 `~/.local/bin/poke-hook` 的 hook。
    deployed: FileFacts,
    deployed_executable: bool,
    /// bundle / target 内的 hook(`<app>/pokehook`),启动期会覆盖到 deployed。
    bundled: FileFacts,
    /// deployed 与 bundled 是否逐字节一致。任一缺失为 None。
    /// false = 部署的 hook 旧了(可能 B 类:协议升级后没重启)。
    #[serde(skip_serializing_if = "Option::is_none")]
    matches_bundled: Option<bool>,
}

#[derive(Serialize)]
struct IntegrationInfo {
    hook_binary: HookBinaryInfo,
    cc: IntegrationCheck,
    codex: IntegrationCheck,
    cursor: IntegrationCheck,
    opencode: IntegrationCheck,
    pi: IntegrationCheck,
    kimi: IntegrationCheck,
    grok: IntegrationCheck,
    /// 各端外部配置文件的元信息,drift 的事实依据。
    config_files: BTreeMap<String, FileFacts>,
    drift_outdated_sources: Vec<String>,
}

/// 集成检测白名单 —— 只保留布尔位 + cc 的 issue 枚举字符串(非敏感)。
#[derive(Serialize)]
struct IntegrationCheck {
    installed: bool,
    hooks_configured: bool,
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hooks_outdated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feature_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue: Option<String>,
}

#[derive(Serialize)]
struct SettingsDigest {
    sound_pending: String,
    sound_idle: String,
    sound_failed: String,
    locale: String,
    notifications_enabled: bool,
    popup_position: crate::settings::PopupPosition,
    popup_screen_strategy: crate::settings::PopupScreenStrategy,
    tray_icon_style: crate::settings::TrayIconStyle,
    popup_auto_close_secs: Option<u32>,
    update_auto_check: bool,
    update_skip_version: Option<String>,
    focus_suppression_enabled: bool,
}

#[derive(Serialize)]
struct SessionsDigest {
    total: usize,
    by_status: StatusCounts,
    by_source: BTreeMap<String, usize>,
}

#[derive(Serialize, Default)]
struct StatusCounts {
    running: usize,
    pending: usize,
    idle: usize,
    last_failed: usize,
}

/// 把以 `$HOME` 开头的路径折叠成 `~`,不暴露真实用户名。
fn tilde_collapse(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => path.replacen(&home, "~", 1),
        _ => path.to_string(),
    }
}

/// 把字符串里**所有**出现的 HOME 路径替换成 `~`(行内脱敏,用于日志行)。
/// 与 `tilde_collapse`(只折叠前缀)区别:日志里路径常出现在行中间。
fn redact_home_all(s: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => s.replace(&home, "~"),
        _ => s.to_string(),
    }
}

fn home_path(rel: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(rel)
}

/// 去掉日志行行首的时间戳(第一个空格之前),得到"消息签名",用于重复折叠。
fn log_line_signature(line: &str) -> &str {
    line.split_once(' ').map(|(_, rest)| rest).unwrap_or(line)
}

/// 同签名只保留最后一次出现,带 `[×N, first ...]`;折叠后取最近 `max` 条。
fn collapse_repeated_log_lines(lines: Vec<String>, max: usize) -> Vec<String> {
    if lines.is_empty() {
        return lines;
    }
    let mut total: HashMap<&str, usize> = HashMap::new();
    let mut first_ts: HashMap<&str, &str> = HashMap::new();
    for l in &lines {
        let sig = log_line_signature(l);
        *total.entry(sig).or_insert(0) += 1;
        first_ts
            .entry(sig)
            .or_insert_with(|| l.split_once(' ').map(|(ts, _)| ts).unwrap_or(""));
    }
    let mut remaining = total.clone();
    let mut out: Vec<String> = Vec::new();
    for l in &lines {
        let sig = log_line_signature(l);
        let r = remaining.get_mut(sig).expect("sig counted in pass 1");
        *r -= 1;
        if *r == 0 {
            let count = total[sig];
            if count > 1 {
                let ft = first_ts.get(sig).copied().unwrap_or("");
                out.push(format!("{l}  [×{count}, first {ft}]"));
            } else {
                out.push(l.clone());
            }
        }
    }
    if out.len() > max {
        out = out.split_off(out.len() - max);
    }
    out
}

/// 读 `pokepoke.log`,筛 WARN/ERROR 后取最近 `max` 条(非 seek-tail I/O)。
///
/// 实现先 `read_to_string` 整文件再过滤 —— 日志有轮转、体量可控,非热路径。
/// 规则:
/// 1. 逐行 HOME→`~` 脱敏。
/// 2. **重复消息折叠**:同签名(去时间戳后相同)只保留最后一次出现,带 `[×N, first ...]`。
/// 3. 折叠后取最近 `max` 条(按最后出现顺序)。
/// 文件缺失 / 读失败 → 空 vec(不让导出失败)。
fn recent_log_warnings(log_path: &Path, max: usize) -> Vec<String> {
    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let lines: Vec<String> = content
        .lines()
        .filter(|l| l.contains("[WARN]") || l.contains("[ERROR]"))
        .map(redact_home_all)
        .collect();

    collapse_repeated_log_lines(lines, max)
}

/// 从日志文件筛运维相关行(osascript、ghostty、popup 门控/关闭/抑制等)。
/// 整文件读取后过滤(见 [`recent_log_warnings`] 说明);不含 session message 原文。
fn operational_lines_from_path(path: &Path) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter(|l| is_operational_log_line(l))
        .map(redact_home_all)
        .collect()
}

/// 运维相关日志行:osascript、ghostty reconcile、popup 门控/关闭/抑制等。
fn is_operational_log_line(line: &str) -> bool {
    if line.contains("[WARN]") || line.contains("[ERROR]") {
        return true;
    }
    line.contains("osascript TIMEOUT")
        || line.contains("osascript SLOW")
        || line.contains("osascript FAILED")
        || line.contains("osascript spawn failed")
        || line.contains("[ghostty]")
        || line.contains("focus-watcher")
        || line.contains("main-thread dispatch LAGGED")
        || line.contains("popup focus_gate")
        || line.contains("close_popup_by_label")
        || line.contains("show popup SUPPRESSED")
        || line.contains("sound SUPPRESSED")
}

/// 运维日志片段:优先当前 `pokepoke.log`(最近价值最高),不足 `max` 再从
/// `pokepoke.log.1` **尾部**按时间序补齐。避免把主日志 + 巨大轮转文件简单
/// 拼接后取尾 —— 那样尾部会被旧轮转内容淹没(2026-07-29 排障教训)。
fn recent_operational_log(current: &Path, rotated: Option<&Path>, max: usize) -> Vec<String> {
    let mut lines = operational_lines_from_path(current);
    if lines.len() < max {
        if let Some(rotated_path) = rotated.filter(|p| p.exists()) {
            let mut older = operational_lines_from_path(rotated_path);
            let need = max.saturating_sub(lines.len());
            if older.len() > need {
                older = older.split_off(older.len() - need);
            }
            older.append(&mut lines);
            lines = older;
        }
    }
    collapse_repeated_log_lines(lines, max)
}

fn active_popup_count(app: &AppHandle) -> usize {
    app.try_state::<PopupList>()
        .map(|pl| pl.lock().map(|g| g.len()).unwrap_or(0))
        .unwrap_or(0)
}

fn build_analysis_hints(
    uptime_secs: u64,
    metrics: &RuntimeMetricsSnapshot,
    recent_log_warnings: &[String],
    recent_operational_log: &[String],
    recent_events: &[String],
) -> Vec<String> {
    let mut hints = Vec::new();

    if uptime_secs < SHORT_UPTIME_HINT_SECS
        && metrics.osascript.timeouts == 0
        && recent_log_warnings.is_empty()
        && recent_operational_log.is_empty()
        && recent_events.is_empty()
    {
        hints.push(
            "process_uptime_under_5min_and_no_osascript_events: export likely taken \
             shortly after restart — reproduce under load before exporting"
                .to_string(),
        );
    }

    if metrics.osascript.timeouts >= 5 {
        hints.push(format!(
            "osascript_timeout_storm: {} timeouts since startup (by_op: {:?}) — \
             common when Ghostty/iTerm2 AppleScript channel hangs; check ghostty_reconcile metrics",
            metrics.osascript.timeouts, metrics.osascript.timeouts_by_op
        ));
    }

    if metrics.ghostty_reconcile.consecutive_failures >= 2 {
        hints.push(format!(
            "ghostty_reconcile_backoff: {} consecutive osascript failures, \
             current_sleep_secs={} — reconcile is in exponential backoff",
            metrics.ghostty_reconcile.consecutive_failures,
            metrics.ghostty_reconcile.current_sleep_secs
        ));
    }

    if metrics.osascript.slow >= 10 {
        hints.push(format!(
            "osascript_slow_frequent: {} calls exceeded slow threshold since startup",
            metrics.osascript.slow
        ));
    }

    if metrics.active_popup_count > 3 {
        hints.push(format!(
            "many_active_popups: {} popup windows on screen — may contribute to WindowServer CPU",
            metrics.active_popup_count
        ));
    }

    if metrics
        .popup
        .suppress_by_reason
        .get("already_focused")
        .is_some_and(|n| *n >= 5)
    {
        let count = metrics.popup.suppress_by_reason["already_focused"];
        hints.push(format!(
            "popup_suppress_already_focused: {count} suppressions since startup — \
             user may perceive missing popups while terminal is focused; \
             check focus_suppression_enabled in settings"
        ));
    }

    if metrics
        .popup
        .close_by_reason
        .get("focus_watcher")
        .is_some_and(|n| *n >= 3)
    {
        let count = metrics.popup.close_by_reason["focus_watcher"];
        hints.push(format!(
            "popup_close_focus_watcher: {count} popups closed by focus watcher — \
             check if popups flash briefly before dismiss"
        ));
    }

    if metrics
        .popup
        .suppress_by_reason
        .get("free_quota_exhausted")
        .is_some_and(|n| *n >= 1)
    {
        hints.push(
            "popup_suppress_free_quota: free-tier daily quota exhausted — \
             popups and sounds are blocked until quota resets"
                .to_string(),
        );
    }

    hints
}

fn collect_health_info(app: &AppHandle, metrics: &RuntimeMetrics) -> HealthInfo {
    let popup_count = active_popup_count(app);
    let snapshot = metrics.snapshot(popup_count);
    let recent_events = runtime_metrics::read_recent_events(EVENT_RING_MAX);
    HealthInfo {
        uptime_secs: snapshot.uptime_secs,
        active_popup_count: popup_count,
        metrics: snapshot,
        recent_events,
        // collect_report 会用完整日志片段覆盖;此处不重复构建 hints。
        analysis_hints: Vec::new(),
    }
}

fn debug_logging_enabled() -> bool {
    std::env::var("POKEPOKE_DEBUG")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(o) = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
        {
            if o.status.success() {
                let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !v.is_empty() {
                    return format!("macOS {}", v);
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

fn fmt_systime(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

/// 采集单个文件的元信息(不读内容)。路径一律 tilde 折叠。
fn file_facts(path: &Path) -> FileFacts {
    let meta = std::fs::metadata(path).ok();
    FileFacts {
        path: tilde_collapse(&path.display().to_string()),
        exists: meta.is_some(),
        size_bytes: meta.as_ref().map(|m| m.len()),
        modified: meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(fmt_systime),
    }
}

/// 逐字节比较两个文件是否一致。任一打不开 → None。
fn files_equal(a: &Path, b: &Path) -> Option<bool> {
    let (Ok(da), Ok(db)) = (std::fs::read(a), std::fs::read(b)) else {
        return None;
    };
    Some(da == db)
}

fn bundled_hook_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("pokehook")))
}

/// 探活:能连上 sock 说明有实例在监听。
fn server_responding(sock: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(sock).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = sock;
        false
    }
}

fn count_dir_entries(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

/// 从 hook `--check-*` 的原始 JSON 里只挑布尔白名单字段,丢弃其余(防夹带)。
fn whitelist_check(v: &Value, issue: Option<String>) -> IntegrationCheck {
    let b = |k: &str| v.get(k).and_then(|x| x.as_bool());
    IntegrationCheck {
        installed: b("installed").unwrap_or(false),
        hooks_configured: b("hooks_configured").unwrap_or(false),
        connected: b("connected").unwrap_or(false),
        hooks_outdated: b("hooks_outdated"),
        feature_enabled: b("feature_enabled"),
        issue,
    }
}

/// 纯函数:由 Settings 生成脱敏摘要。单测据此断言无敏感子串。
fn settings_digest(s: &Settings) -> SettingsDigest {
    SettingsDigest {
        sound_pending: s.sound_pending.clone(),
        sound_idle: s.sound_idle.clone(),
        sound_failed: s.sound_failed.clone(),
        locale: s.locale.clone(),
        notifications_enabled: s.notifications_enabled,
        popup_position: s.popup_position,
        popup_screen_strategy: s.popup_screen_strategy,
        tray_icon_style: s.tray_icon_style,
        popup_auto_close_secs: s.popup_auto_close_secs,
        update_auto_check: s.update_auto_check,
        update_skip_version: s.update_skip_version.clone(),
        focus_suppression_enabled: s.focus_suppression_enabled,
    }
}

/// 纯函数:由 session 列表生成状态 / 来源计数。只计数,不含任何单条内容。
fn sessions_digest(sessions: &[Session]) -> SessionsDigest {
    let mut by_status = StatusCounts::default();
    let mut by_source: BTreeMap<String, usize> = BTreeMap::new();
    for s in sessions {
        match s.status {
            SessionStatus::Running => by_status.running += 1,
            SessionStatus::Pending => by_status.pending += 1,
            SessionStatus::Idle => by_status.idle += 1,
            SessionStatus::LastFailed => by_status.last_failed += 1,
        }
        let source = s.source.clone().unwrap_or_else(|| "unknown".to_string());
        *by_source.entry(source).or_insert(0) += 1;
    }
    SessionsDigest {
        total: sessions.len(),
        by_status,
        by_source,
    }
}

fn collect_runtime_info() -> RuntimeInfo {
    let dir = home_path(".pokepoke");
    let sock = dir.join("sock");
    let log_path = dir.join("pokepoke.log");
    let rotated_log_path = dir.join("pokepoke.log.1");
    let data_files = vec![
        file_facts(&dir.join("sessions.json")),
        file_facts(&dir.join("settings.json")),
        file_facts(&log_path),
    ];
    let rotated_log = {
        let facts = file_facts(&rotated_log_path);
        if facts.exists {
            Some(facts)
        } else {
            None
        }
    };
    let recent_log_warnings = recent_log_warnings(&log_path, LOG_TAIL_MAX);
    let recent_operational_log =
        recent_operational_log(&log_path, Some(&rotated_log_path), OPERATIONAL_LOG_MAX);
    RuntimeInfo {
        pokepoke_dir: tilde_collapse(&dir.display().to_string()),
        server_responding: server_responding(&sock),
        server_socket: file_facts(&sock),
        snapshots_count: count_dir_entries(&dir.join("snapshots")),
        data_files,
        recent_log_warnings,
        rotated_log,
        recent_operational_log,
    }
}

fn collect_hook_binary_info() -> HookBinaryInfo {
    let deployed_path = integration::hook_bin_path();
    let (_, deployed_executable) = integration::hook_binary_state();
    let bundled = bundled_hook_path();
    let matches_bundled = bundled
        .as_ref()
        .and_then(|b| files_equal(&deployed_path, b));
    HookBinaryInfo {
        deployed: file_facts(&deployed_path),
        deployed_executable,
        bundled: bundled.map(|p| file_facts(&p)).unwrap_or(FileFacts {
            path: "<unknown>".into(),
            exists: false,
            size_bytes: None,
            modified: None,
        }),
        matches_bundled,
    }
}

fn collect_integration_info(app: &AppHandle) -> IntegrationInfo {
    let cc_status = serde_json::to_value(integration::cc_integration_status()).unwrap_or_default();
    let cc_issue = cc_status
        .get("issue")
        .and_then(|v| v.as_str())
        .map(String::from);

    let drift = app
        .try_state::<Arc<Mutex<integration::HookDriftState>>>()
        .map(|s| {
            s.lock()
                .unwrap_or_else(|p| p.into_inner())
                .outdated_sources()
        })
        .unwrap_or_default();

    let mut config_files: BTreeMap<String, FileFacts> = BTreeMap::new();
    config_files.insert(
        "claude_settings".into(),
        file_facts(&home_path(".claude/settings.json")),
    );
    config_files.insert(
        "codex_config".into(),
        file_facts(&home_path(".codex/config.toml")),
    );
    config_files.insert(
        "codex_hooks".into(),
        file_facts(&home_path(".codex/hooks.json")),
    );
    config_files.insert(
        "cursor_hooks".into(),
        file_facts(&home_path(".cursor/hooks.json")),
    );
    config_files.insert(
        "opencode_plugin".into(),
        file_facts(&home_path(".config/opencode/plugins/pokepoke.js")),
    );
    config_files.insert(
        "pi_extension".into(),
        file_facts(&home_path(".pi/agent/extensions/pokepoke/index.ts")),
    );
    config_files.insert(
        "kimi_config".into(),
        file_facts(&home_path(".kimi-code/config.toml")),
    );
    config_files.insert(
        "grok_hooks".into(),
        file_facts(&home_path(".grok/hooks/pokepoke.json")),
    );
    IntegrationInfo {
        hook_binary: collect_hook_binary_info(),
        cc: whitelist_check(&cc_status, cc_issue),
        codex: whitelist_check(&integration::codex_check_json(), None),
        cursor: whitelist_check(&integration::cursor_check_json(), None),
        opencode: whitelist_check(&integration::opencode_check_json(), None),
        pi: whitelist_check(&integration::pi_check_json(), None),
        kimi: whitelist_check(&integration::kimi_check_json(), None),
        grok: whitelist_check(&integration::grok_check_json(), None),
        config_files,
        drift_outdated_sources: drift,
    }
}

fn collect_report(
    app: &AppHandle,
    store: &Arc<Mutex<SessionStore>>,
    settings: &Arc<Mutex<SettingsStore>>,
    metrics: &Arc<RuntimeMetrics>,
) -> DiagnosticsReport {
    let sessions = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_all()
        .to_vec();
    let settings_snapshot = settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .settings
        .clone();

    let runtime = collect_runtime_info();
    let mut health = collect_health_info(app, metrics);
    health.analysis_hints = build_analysis_hints(
        health.uptime_secs,
        &health.metrics,
        &runtime.recent_log_warnings,
        &runtime.recent_operational_log,
        &health.recent_events,
    );

    DiagnosticsReport {
        schema_version: SCHEMA_VERSION,
        generated_at: chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        app: AppInfo {
            version: app.package_info().version.to_string(),
            os: os_version(),
            arch: std::env::consts::ARCH.to_string(),
            debug_logging: debug_logging_enabled(),
            dev_build: cfg!(debug_assertions),
        },
        runtime,
        health,
        integration: collect_integration_info(app),
        settings: settings_digest(&settings_snapshot),
        sessions: sessions_digest(&sessions),
        notes: vec![
            "Sensitive and non-public data are intentionally excluded: session message/title/workspace \
             path, terminal tty, transcript path."
                .to_string(),
            "File entries carry metadata only (size/mtime); contents are never read. \
             recent_log_warnings is WARN/ERROR only (whole-file read, then last N lines); \
             recent_operational_log prefers current pokepoke.log (backfills from .log.1 only when needed)."
                .to_string(),
            "health.metrics.popup counts suppress/close/focus_gate since this process start; \
             health.recent_events may include popup_suppress/popup_close JSON lines."
                .to_string(),
            "health.metrics.notify_decisions holds the most recent /notify full-chain decision \
             traces (~/.pokepoke/notify-decisions.jsonl, 1MB ring cap, last 20 exported) — \
             the first stop when a notification didn't pop."
                .to_string(),
            "health.metrics.watchers lists background watcher health (name / last heartbeat / \
             nominal interval / age / stalled_count, written by supervisor monitor) — a \
             stalled watcher is a silent-loss root cause for missed reminders."
                .to_string(),
            "health.metrics counts since this process start; health.recent_events persists \
             across restarts in ~/.pokepoke/diagnostics-events.jsonl (no session content)."
                .to_string(),
            "If CPU or popup issues occur, export diagnostics while the problem is happening — \
             do not restart first."
                .to_string(),
        ],
    }
}

fn diagnostics_dir() -> PathBuf {
    home_path(".pokepoke")
}

fn diagnostics_file_path() -> PathBuf {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    diagnostics_dir().join(format!("diagnostics-{}.json", ts))
}

/// macOS:在 Finder 中定位刚导出的文件。其他平台 no-op。
fn reveal_in_finder(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// 导出脱敏诊断报告到 `~/.pokepoke/diagnostics-<ts>.json` 并在 Finder 定位。
/// 返回给前端的是 `~` 折叠后的路径字符串(同样不暴露真实用户名)。
#[tauri::command]
pub fn export_diagnostics(
    app: AppHandle,
    store: State<'_, Arc<Mutex<SessionStore>>>,
    settings: State<'_, Arc<Mutex<SettingsStore>>>,
    metrics: State<'_, Arc<RuntimeMetrics>>,
) -> Result<String, String> {
    let report = collect_report(
        &app,
        &store.inner().clone(),
        &settings.inner().clone(),
        &metrics.inner().clone(),
    );
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;

    let path = diagnostics_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, json).map_err(|e| e.to_string())?;

    let display_path = tilde_collapse(&path.display().to_string());
    crate::log_info!("diagnostics", "exported diagnostics to {}", display_path);
    reveal_in_finder(&path);
    Ok(display_path)
}

#[cfg(test)]
#[path = "diagnostics/tests.rs"]
mod tests;
