//! http_server 的状态层:进程内共享状态结构 + 两个 trait 抽象。
//!
//! 从 `http_server.rs` 抽出来,让 `http_server.rs` 聚焦路由 + notify 处理逻辑,
//! 状态结构(去重窗口 / cooldown / 最近访问 / 已认领 stage / AppState)单独成模块。
//! 纯文本搬运,行为零变化;外部仍通过 `crate::http_server::AppState` 等访问
//! (`http_server.rs` 顶部 `pub use state::*` 重导出)。

use crate::codex_appserver::{CodexAppServerHandle, CodexThreadStatusCache};
use crate::codex_rollout::AutoApprovalCache;
use crate::popup::{FocusOracle, PopupController};
use crate::quiet_mode::QuietModeState;
use crate::sessions::Session;
use crate::sessions::SessionStatus;
use crate::sessions::SessionStore;
use crate::settings::SettingsStore;
use crate::sound::SoundPlayer;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

// NotificationGate trait(全局提示开关新增):把"是否允许发 popup + 播提示音"
// 这个决策抽出来,避免 http_server 直接依赖 SettingsStore,也方便 integration
// 测试注入可控实现,显式验证开关关闭时 popup / sound 一定不会触发。
pub trait NotificationGate: Send + Sync {
    fn allow_notifications(&self) -> bool;
}

pub struct SettingsNotificationGate {
    settings: Arc<Mutex<SettingsStore>>,
}

impl SettingsNotificationGate {
    pub fn new(settings: Arc<Mutex<SettingsStore>>) -> Self {
        Self { settings }
    }
}

impl NotificationGate for SettingsNotificationGate {
    fn allow_notifications(&self) -> bool {
        self.settings
            .lock()
            .map(|s| s.settings.notifications_enabled)
            .unwrap_or(true)
    }
}

// EventEmitter trait(Layer 1 测试自动化新增):把 emit("sessions-updated")
// 包成接口,让 integration 测试不需要构造 AppHandle。
// 生产实现 AppEventEmitter 持有 AppHandle 并直接调 emit。
pub trait EventEmitter: Send + Sync {
    fn sessions_updated(&self);
}

pub struct AppEventEmitter {
    app: AppHandle,
}

impl AppEventEmitter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventEmitter for AppEventEmitter {
    fn sessions_updated(&self) {
        let _ = self.app.emit("sessions-updated", ());
    }
}

/// http_server 路由共享的进程内状态。所有字段都是 Arc,axum State Clone 传递。
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<SessionStore>>,
    pub popup: Arc<dyn PopupController>,
    pub sound: Arc<dyn SoundPlayer>,
    pub focus: Arc<dyn FocusOracle>,
    pub events: Arc<dyn EventEmitter>,
    pub gate: Arc<dyn NotificationGate>,
    /// W8(brief 20260813-session-memory):五窗口单锁多槽(recently_visited /
    /// acked / popup cooldown / sound cooldown / dedupe),语义维度各自独立,
    /// 存储与锁合一。
    pub memory: Arc<SessionMemory>,
    /// Codex 自动审批判据缓存(纯只读查询,见 `codex_rollout`)。命中=该轮是自动
    /// 审批轮,evaluate_and_show_popup 结合 thread status 决定是否抑制。
    pub auto_approval: Arc<AutoApprovalCache>,
    /// Codex app-server `thread/status/changed` 缓存(见 `codex_appserver`)。
    pub codex_thread_status: Arc<CodexThreadStatusCache>,
    /// 向 codex app-server 发 `thread/resume` 的后台句柄。
    pub codex_appserver: CodexAppServerHandle,
    pub settings: Arc<Mutex<SettingsStore>>,
    /// 静谧模式倒计时状态；`None` 仅用于测试桩。
    pub quiet_mode: Option<Arc<QuietModeState>>,
    /// popup 呈现(staging)派发器:gate 判定 + show/sound + trace 落盘是否
    /// 脱离 `/notify` 响应路径。生产用 `StagingDispatcher::spawn_worker`,
    /// 测试用 `inline()` 保持同步语义(见 `http_server/staging.rs` 模块注释)。
    pub staging: crate::http_server::StagingDispatcher,
}

/// 五个 per-session 窗口结构合并为单一 `SessionMemory`(W8,brief
/// 20260813-session-memory):单锁多槽位。语义维度彼此独立(审查已确认不能
/// 合并),但存储与锁可以合一 —— 每次 /notify 从拿 5 把锁降到 1 把,未来
/// 加第 6 个窗口不再复制模板(模板复制是漏提醒类 bug 的温床)。
///
/// 槽位语义(逐条搬迁自原五结构,doc 注释保留原窗口理由):
/// - `recently_visited`:用户跳转去看过后的 30s 静默窗(挡 agent 被动 emit)
/// - `acked`:用户已认领的 stage,直到 Running 清除(挡同 stage 重弹)
/// - `popup_last_shown`:弹窗 3s 冷却(挡 transition thrashing)
/// - `sound_last_played`:播音 3s 冷却,按 task_id(挡双发双音)
/// - `dedupe_last_seen`:(task_id, signature) 100ms 双触发去重(§3.2)
pub struct SessionMemory {
    inner: Mutex<SessionMemoryInner>,
    // 窗口时长(构造注入,测试用自定义值;生产走 Default 常量,固定后无并发变更)。
    recently_window: Duration,
    popup_cooldown: Duration,
    sound_cooldown: Duration,
}

struct SessionMemoryInner {
    recently_visited: HashMap<String, Instant>,
    acked: HashMap<String, SessionStatus>,
    popup_last_shown: HashMap<String, Instant>,
    sound_last_played: HashMap<String, Instant>,
    dedupe_last_seen: HashMap<String, (String, Instant)>,
}

/// 用户通过 popup 跳转去看过某个 session 后,该 session 进入"最近访问"
/// 状态 — 默认 30s 内同 session 的新 popup 静默(只更新内部 store,不弹)。
/// 处理"刚跳过去看了一眼,几十秒后 agent 自己 emit Notification 又被 popup 拉
/// 回去"的恼人体验:cc 在某些 tab 切换 / 上下文重载场景下会重新 emit 等待用户
/// 输入的 hook 事件,但用户本人没有任何新输入,这种"被动事件"不该再打扰用户。
///
/// 跟 popup cooldown 维度不同:
/// - popup cooldown 按"上次弹 popup 的时间",防 transition thrashing(3s)
/// - recently_visited 按"用户上次主动跳转去看的时间",防 agent 被动 emit(30s)
const RECENTLY_VISITED_WINDOW: Duration = Duration::from_secs(30);
const RECENTLY_VISITED_PRUNE_THRESHOLD: Duration = Duration::from_secs(120);

// 用户点过 popup 后,把"该 session 此刻所处的 stage"记进来 — 同 session
// 同 status 再触发 popup 直接抑制。直到 session 至少经过一次 Running(=用
// 户开始干新一轮活)才清除记录,下一次到达 Idle / LastFailed / Pending 才
// 会再弹。
//
// 解决 recently_visited 30s 窗口外的"agent 自己在 Pending ↔ Idle 之间反复
// 摆动而用户实际没新输入"场景:cc 在 SubAgent 返回、Compact、内部超时等
// 情况下会重新 emit hook,在 PokePoke 状态机看是合法新一轮 stage 转移,但
// 用户已经认领过这条 session 的当前 stage,没必要再叨。
//
// 跟现有两道关卡的维度区分:
// - `recently_visited` (30s):按"刚跳过去看的时间"挡几十秒内被动 emit
// - `popup_last_shown` (3s):按"上次弹时间"挡 transition thrashing
// - `acked` (until next Running):按"用户已认领的 stage"挡同 stage 重弹
//
// 颗粒度 = `(session_id, status)`。每个 session 只记一份"最新认领的
// status",新认领覆盖旧认领;进入 Running 时整条清掉。

/// 同 session_id 的 popup 在 `cooldown` 内只允许弹一次。这是 should_show_popup
/// (transition rules) + already_focused 之外的最后一道关卡,专门处理 cc / cursor
/// agent 内部反复 transition (running ↔ idle ↔ running) 的状态机 thrashing
/// 导致同一 session 短时间反复弹 popup 的恼人体验。
///
/// 跟 dedupe 维度不同:
/// - dedupe 按 (task_id, signature) 防 hook 双触发(100ms)
/// - popup cooldown 按 session_id 防 transition thrashing(3s)
// 3 秒:挡 hook 重传 / agent 内部 1-2s 间隔的状态机 thrashing(running ↔ idle
// 反复切),不影响用户在终端里的快速问答(那种 popup 也会被 already_focused
// 提前挡掉,走不到这道关)。设太短(<2s)挡不住 thrashing,设太长(15s+)
// 会把用户在副屏失焦时的连续真实问答 popup 也吞掉。
const POPUP_COOLDOWN: Duration = Duration::from_secs(3);
const POPUP_COOLDOWN_PRUNE_THRESHOLD: Duration = Duration::from_secs(30);

/// 聚焦播音限流:同 task_id 的 stage-ending 事件(如 cursor Ctrl+C 连发的
/// stop|idle + stop|last_failed,~40ms 间隔)只播一次音。
///
/// 为什么独立于 popup cooldown:
/// - popup cooldown 按 session_id 挡"弹窗创建",只在 AttemptShow 路径消耗;
///   用户聚焦终端走 SuppressWithSound(只播音不弹窗)时它不查询也不消耗 →
///   双发事件各播一次音(20260805 双音效 bug 的直接机制)。
/// - 若复用 popup 槽位(聚焦播音消耗 3s 弹窗 cooldown),用户 3s 内切走 +
///   下一轮结束 → 真 popup 被吞 = 漏提醒,违反主线。
/// - 按 task_id(非 session_id):双发共享 task_id;且 session_end 重建 session
///   (session_id 变化)的场景也能命中;不同 task 互不干扰,不产生漏提醒。
///
/// 窗口 3s:与 popup cooldown 一致。双发实测 40ms 内、tty 连发窗口 ~250ms,
/// 余量充足;同 task 3s 内完成两轮真任务(会吞第二次音)对 cursor/cc 实际
/// 不可能——单轮耗时秒级~分钟级,且用户聚焦时本就盯着终端。
const SOUND_COOLDOWN: Duration = Duration::from_secs(3);
const SOUND_COOLDOWN_PRUNE_THRESHOLD: Duration = Duration::from_secs(30);

/// 进程内事件去重窗口。判定"同 task_id 上一次 signature 是否相同 + 是否在
/// `window` 内"——只有"信号没变 + 时间相近"才算重复(Cursor 开 Third-party
/// hooks 双触发的真实场景)。状态机正常迁移(如 running → idle → running)
/// 因 signature 变化不会被误吞。
///
/// 结构上比 hook 端的文件锁简单一个量级:server 是单进程、Mutex 足矣,无需
/// sweep / TTL / fail-open。由 `docs/archive/scope-audit-20260422.md §8.2` 硬边界
/// 明文要求:**仅允许在 server 单进程内做,不得引入文件系统锁**。
const DEDUPE_PRUNE_THRESHOLD: Duration = Duration::from_secs(1);

impl SessionMemory {
    /// 构造;窗口时长可注入(测试用),默认走各槽位常量。
    pub fn new(
        recently_visited_window: Duration,
        popup_cooldown: Duration,
        sound_cooldown: Duration,
    ) -> Self {
        Self {
            inner: Mutex::new(SessionMemoryInner {
                recently_visited: HashMap::new(),
                acked: HashMap::new(),
                popup_last_shown: HashMap::new(),
                sound_last_played: HashMap::new(),
                dedupe_last_seen: HashMap::new(),
            }),
            // 窗口参数经闭包字段存下来 —— 为保持单锁结构,窗口时长直接存在
            // Inner 之外由方法读取(无并发变更,读安全)。
            recently_window: recently_visited_window,
            popup_cooldown,
            sound_cooldown,
        }
    }

    /// 用户跳转去看过 session → 记录(30s 静默窗起点)。prune 老于 120s 条目。
    pub fn mark_visited(&self, session_id: &str) {
        let now = Instant::now();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.recently_visited
            .retain(|_, ts| now.duration_since(*ts) < RECENTLY_VISITED_PRUNE_THRESHOLD);
        g.recently_visited.insert(session_id.to_string(), now);
    }

    /// 该 session 是否在最近访问窗口内。
    pub fn was_recently(&self, session_id: &str) -> bool {
        let now = Instant::now();
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.recently_visited
            .get(session_id)
            .map(|ts| now.duration_since(*ts) < self.recently_window)
            .unwrap_or(false)
    }

    /// 用户在 `status` 下点过这个 session 的 popup → 记录认领。
    /// 同 session 多次点(可能 status 已变)按最新一次覆盖。
    pub fn mark_acked(&self, session_id: &str, status: SessionStatus) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.acked.insert(session_id.to_string(), status);
    }

    /// 该 session 此刻 status 是否已被用户认领过。
    pub fn is_acked(&self, session_id: &str, status: &SessionStatus) -> bool {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.acked.get(session_id) == Some(status)
    }

    /// 进入 Running / SessionEnd 时调,把该 session 的认领清掉。
    pub fn clear_acked(&self, session_id: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.acked.remove(session_id);
    }

    /// 同 session 弹窗 3s 冷却:返回 true 允许弹 popup,false 在冷却期内被抑制。
    /// 允许时记录当前时间戳;顺手 prune 老于 30s 的条目避免 map 无限涨。
    pub fn popup_try_acquire(&self, session_id: &str) -> bool {
        let now = Instant::now();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.popup_last_shown
            .retain(|_, ts| now.duration_since(*ts) < POPUP_COOLDOWN_PRUNE_THRESHOLD);
        let allow = g
            .popup_last_shown
            .get(session_id)
            .map(|last| now.duration_since(*last) >= self.popup_cooldown)
            .unwrap_or(true);
        if allow {
            g.popup_last_shown.insert(session_id.to_string(), now);
        }
        allow
    }

    /// 同 task 播音 3s 冷却:返回 true 允许播音,false 在冷却期内被抑制。
    /// 允许时记录当前时间戳;顺手 prune 老于 30s 的条目避免 map 无限涨。
    pub fn sound_try_acquire(&self, task_id: &str) -> bool {
        let now = Instant::now();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.sound_last_played
            .retain(|_, ts| now.duration_since(*ts) < SOUND_COOLDOWN_PRUNE_THRESHOLD);
        let allow = g
            .sound_last_played
            .get(task_id)
            .map(|last| now.duration_since(*last) >= self.sound_cooldown)
            .unwrap_or(true);
        if allow {
            g.sound_last_played.insert(task_id.to_string(), now);
        }
        allow
    }

    /// 若同 task_id 的"上一次 signature"与本次相同,且时间差 < `window`
    /// → 返回 true(重复)。每次调用顺手 prune 老于 1s 的条目,避免 HashMap
    /// 在长时间运行后无限增长。
    ///
    /// 注意:signature 只有 `event_type + status`,同 status 的不同事件(如
    /// 两条 pending、message 不同)在窗口内也会被去重 —— 刻意行为,§3.2
    /// 双触发去重优先于"同状态不同事件都要放行"。
    pub fn dedupe_is_duplicate(&self, task_id: &str, signature: &str, window: Duration) -> bool {
        let now = Instant::now();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.dedupe_last_seen
            .retain(|_, (_, ts)| now.duration_since(*ts) < DEDUPE_PRUNE_THRESHOLD);
        if let Some((prev_sig, prev_ts)) = g.dedupe_last_seen.get(task_id) {
            if prev_sig == signature && now.duration_since(*prev_ts) < window {
                return true;
            }
        }
        g.dedupe_last_seen
            .insert(task_id.to_string(), (signature.to_string(), now));
        false
    }
}

impl Default for SessionMemory {
    fn default() -> Self {
        Self::new(RECENTLY_VISITED_WINDOW, POPUP_COOLDOWN, SOUND_COOLDOWN)
    }
}

/// 统一播音出口(brief 20260805-pending-idle-completed-sound-swallowed,kimi 评审线路):
/// notify 路径所有播音都从这里走,统一 try_acquire + play。职责边界:
/// - quiet_mode / aborted(T1) 在此短路(静默)。
/// - stage-ending(Idle / LastFailed)消耗 sound cooldown 槽位(3s,按 task);
///   Pending(waiting)音**不消耗**槽位 → waiting 音永不饿死 3s 内的 completed 音
///   (修复聚焦路径 Pending→Idle 完成音被吞)。
/// 返回 true = 实际派发播放。
pub fn play_notify_sound(state: &AppState, session: &Session) -> bool {
    if let Some(quiet) = state.quiet_mode.as_ref() {
        if quiet.is_active() {
            crate::log_info!(
                "notify",
                "sound SUPPRESSED task_id={} reason=quiet_mode tty={:?}",
                session.task_id,
                session.terminal_tty,
            );
            return false;
        }
    }
    // T1(brief 20260805-t1-cursor-aborted):aborted 取消不播"完成"音。
    if session.is_recently_aborted() {
        crate::log_info!(
            "notify",
            "sound SUPPRESSED task_id={} reason=aborted tty={:?}",
            session.task_id,
            session.terminal_tty,
        );
        return false;
    }
    let is_stage_ending = matches!(
        session.status,
        SessionStatus::Idle | SessionStatus::LastFailed
    );
    if is_stage_ending && !state.memory.sound_try_acquire(&session.task_id) {
        crate::log_info!(
            "notify",
            "sound SUPPRESSED task_id={} reason=sound_cooldown tty={:?}",
            session.task_id,
            session.terminal_tty,
        );
        return false;
    }
    state.sound.play_alert(session.status.clone());
    crate::log_info!(
        "notify",
        "sound play requested task_id={} status={:?} tty={:?}",
        session.task_id,
        session.status,
        session.terminal_tty,
    );
    true
}

pub(crate) const DEDUPE_WINDOW: Duration = Duration::from_millis(100);

/// codex `~/.codex/session_index.jsonl` 的 title 查询缓存（进程级 static）。
///
/// `resolve_title_for_notify` 对 codex desktop 每次 /notify 都查这个文件的
/// thread_name；文件行数随历史 thread 线性增长，无缓存时每次事件都是全量读 +
/// 逐行 parse。这里以「路径 + mtime」为 key 缓存解析结果：文件没变 → O(1)
/// 命中；变了 → 重读一次。mtime 同秒内长度不变的二次写入可能漏最新一行，
/// 下一事件自愈，可接受。
///
/// 不塞进 `AppState`（避免所有构造点都要补字段）；单条目 keyed by path，
/// 多路径并行最坏退化为"每次重读"，与现状等价，无正确性风险。
pub(crate) static CODEX_TITLE_CACHE: CodexTitleCache = CodexTitleCache::new();

pub(crate) struct CodexTitleCache {
    cached: Mutex<Option<CachedIndex>>,
}

/// 一次读到的 `session_index.jsonl` 快照:文件路径 + 读取时 mtime + id→thread_name。
struct CachedIndex {
    path: PathBuf,
    mtime: SystemTime,
    names: HashMap<String, Option<String>>,
}

impl CodexTitleCache {
    pub(crate) const fn new() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }

    /// 返回 thread_id 的 thread_name。文件缺失 / id 未命中 / 名称为空 → None。
    /// mtime 未变时直接查表；变了才重读文件。
    pub(crate) fn thread_name(&self, path: &Path, thread_id: &str) -> Option<String> {
        let mtime = fs::metadata(path).and_then(|m| m.modified()).ok()?;
        let mut guard = self.cached.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cached) = guard.as_ref() {
            if cached.path == path && cached.mtime == mtime {
                return cached.names.get(thread_id).cloned().flatten();
            }
        }
        let names = read_session_index(path)?;
        let result = names.get(thread_id).cloned().flatten();
        *guard = Some(CachedIndex {
            path: path.to_path_buf(),
            mtime,
            names,
        });
        result
    }
}

/// 全量读 + 逐行 parse `session_index.jsonl`，产出 `{id: thread_name(可为空)}`。
/// 读失败返回 None（不缓存，下次再试）；单行 parse 失败跳过（并发写文件时
/// 可能读到半行，跳过即可，下一事件自愈）。
fn read_session_index(path: &Path) -> Option<HashMap<String, Option<String>>> {
    let content = fs::read_to_string(path).ok()?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(id) = v.get("id").and_then(|i| i.as_str()) else {
            continue;
        };
        let name = v
            .get("thread_name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(|n| n.to_string());
        map.insert(id.to_string(), name);
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sound_cooldown_first_acquire_allowed() {
        let mem = SessionMemory::default();
        assert!(mem.sound_try_acquire("cursor-abc"));
    }

    #[test]
    fn sound_cooldown_second_acquire_within_window_blocked() {
        // 同 task_id 冷却期内第二次播音被挡:cursor Ctrl+C 双发
        // stop|idle + stop|last_failed(~40ms 间隔)只播一次音。
        let mem = SessionMemory::default();
        assert!(mem.sound_try_acquire("cursor-abc"));
        assert!(!mem.sound_try_acquire("cursor-abc"));
    }

    #[test]
    fn sound_cooldown_after_window_expires_allowed_again() {
        let mem = SessionMemory::new(
            Duration::from_secs(30),
            Duration::from_secs(3),
            Duration::from_nanos(1),
        );
        assert!(mem.sound_try_acquire("cursor-abc"));
        std::thread::sleep(Duration::from_millis(2));
        assert!(mem.sound_try_acquire("cursor-abc"));
    }

    #[test]
    fn sound_cooldown_independent_per_task() {
        // 不同 task_id 互不干扰 — 不会把别的任务的真完成音吞掉。
        let mem = SessionMemory::default();
        assert!(mem.sound_try_acquire("cursor-abc"));
        assert!(mem.sound_try_acquire("cursor-def"));
        assert!(!mem.sound_try_acquire("cursor-abc"));
        assert!(!mem.sound_try_acquire("cursor-def")); // 自身仍在窗口内
        assert!(mem.sound_try_acquire("cursor-ghi"));
    }

    #[test]
    fn codex_title_cache_hits_and_refreshes_on_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session_index.jsonl");
        std::fs::write(
            &path,
            "{\"id\":\"t1\",\"thread_name\":\"one\"}\n{\"id\":\"t2\",\"thread_name\":\"\"}\n",
        )
        .unwrap();

        let cache = CodexTitleCache::new();
        assert_eq!(cache.thread_name(&path, "t1").as_deref(), Some("one"));
        assert_eq!(
            cache.thread_name(&path, "t2"),
            None,
            "空 thread_name → None"
        );
        assert_eq!(cache.thread_name(&path, "nope"), None, "未命中 → None");

        // 内容变化(mtime 变)→ 重读,不再命中旧缓存
        std::fs::write(&path, "{\"id\":\"t1\",\"thread_name\":\"renamed\"}\n").unwrap();
        assert_eq!(cache.thread_name(&path, "t1").as_deref(), Some("renamed"));
    }

    #[test]
    fn codex_title_cache_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-such-file.jsonl");
        let cache = CodexTitleCache::new();
        assert_eq!(cache.thread_name(&path, "t1"), None);
    }
}
