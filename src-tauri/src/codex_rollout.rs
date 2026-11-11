//! Codex rollout 解析器 + 自动审批判据缓存。
//!
//! 解决"替我审批"场景下过时 popup:ChatGPT 桌面端开着自动审批时,Codex 每轮自驱
//! 跑完审批继续,但仍发 Stop->Idle hook,PokePoke 据此弹 popup,用户看到时 session
//! 早已回 Running。Codex hook 事件不带自动审批标志位(人提交与自驱续轮的
//! UserPromptSubmit 逐字段相同),无法从 hook 区分。
//!
//! 但 Codex 把权威审批信号写进 rollout 文件:
//! `~/.codex/sessions/<YYYY/MM/DD>/rollout-<ts>-<threadUUID>.jsonl`。每轮的
//! `turn_context` 事件带 `approval_policy` / `approvals_reviewer` / `model`,
//! 详见 `docs/briefs/20260715-codex-auto-approval-suppression.md`。
//!
//! 本模块只读这些字段,把"该 session 当前轮是否自动审批"缓存起来,供
//! `evaluate_and_show_popup` 查询决定是否抑制 popup。**纯只读,不注入合成事件,
//! 不卡审批**:降级(文件不存在 / 读不到当前轮 / 解析失败)一律判 Unknown ->
//! 调用侧放行(回退现有 hook 行为),绝不误杀真警报。
//!
//! 结构照 `transcript_watch.rs`:5s 轮询线程 snapshot codex session -> tail 读
//! rollout -> 解析最新 turn_context -> 写进程内 cache。但方向相反
//! (transcript_watch 是补该有的提醒,本模块是压不该有的提醒),所以走查询而非注入。

use crate::http_server::AppState;
use crate::sessions::Session;
use crate::{log_debug, log_info};
use chrono::Datelike;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// tail 读取的最大字节数。rollout 文件会随会话变长(实测单文件 4.5MB+),
/// turn_context 每轮一条且在轮开头(task_started 之后),其后跟着该轮全部
/// reasoning/token_count/agent_message 输出--单轮 trailing 输出实测 ~117KB、
/// 理论上限几百 KB(reasoning 模型单轮输出 token 有限)。64KB tail 够不着最后一个
/// turn_context(实测 4.5MB 文件最后一条 turn_context 在距 EOF 117KB 处),
/// 导致恒判 Unknown -> 抑制永不生效。2MB 既可靠覆盖单轮 trailing 输出(>15x 余量),
/// 又把单次 I/O 钉在 2MB 内(每 5s 每 codex session 一次,可接受)。
/// 超出 2MB 的极端长 reasoning 轮 -> tail 仍无 turn_context -> Unknown(放行,安全方向)。
const TAIL_BYTES: u64 = 2 * 1024 * 1024;
const WATCH_INTERVAL: Duration = Duration::from_secs(5);
/// cache TTL:超过则判过期回 Unknown(放行)。比轮询间隔长一档(10s vs 5s),
/// 保证查询时命中上一轮 poll 的结果而非频繁回退;同时在 codex 进程退出后及时
/// 失效,避免用陈旧判据抑制新一轮(可能已关掉自动审批)的 popup。
const CACHE_TTL: Duration = Duration::from_secs(10);

/// 该 session 当前轮的审批判据结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoApprovalVerdict {
    /// 命中自动审批信号(approval_policy=never / approvals_reviewer=auto_review /
    /// model=codex-auto-review 之一)-> 调用侧应抑制 popup。
    AutoApproved,
    /// 明显是 user 审批(approval_policy 非 never 且 reviewer 非 auto_review) ->
    /// 调用侧应弹。
    UserApproval,
    /// 读不到当前轮 turn_context / 文件不存在 / 解析失败 -> 调用侧应放行。
    Unknown,
}

/// 进程内"session 当前是否自动审批"缓存。轮询线程写,evaluate_and_show_popup 读。
/// 跟 `RecentlyVisited` / `AckedStages` 同级:都是按 session_id 的短窗判据。
///
/// 维度区分:
/// - `RecentlyVisited`(30s):用户刚跳去看过,挡被动 emit
/// - `AckedStages`(until Running):用户已认领的 stage,挡同 stage 重弹
/// - `AutoApprovalCache`(10s):该轮是不是自动审批模式,挡自驱空档 popup
///
/// 颗粒度 = `session_id -> (verdict, polled_at)`。TTL 过期 -> 查询返回 Unknown
/// -> 放行。`default()` 给空 cache(所有查询返回 Unknown=放行),测试默认安全。
pub struct AutoApprovalCache {
    entries: Mutex<HashMap<String, (AutoApprovalVerdict, Instant)>>,
}

impl AutoApprovalCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 轮询线程写:记录某 session 最新一次 verdict。返回 true = 本次判据相对上次
    /// 发生了变化(含首次写入),调用方可据此只在转变时记日志,避免稳定状态下每轮
    /// 刷一条噪声(实测每 5s 对已确认的 session 重复记,1 分钟刷 60 条淹没真实事件)。
    pub fn set(&self, session_id: &str, verdict: AutoApprovalVerdict) -> bool {
        let mut map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let changed = map.get(session_id).is_none_or(|(prev, _)| *prev != verdict);
        map.insert(session_id.to_string(), (verdict, Instant::now()));
        changed
    }

    /// 查询:命中且未过期返回 verdict;未命中 / 过期 / 锁失败 -> None
    /// (调用侧把 None 当 Unknown -> 放行)。
    pub fn get(&self, session_id: &str) -> Option<AutoApprovalVerdict> {
        let now = Instant::now();
        let map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.get(session_id)
            .and_then(|(v, ts)| (now.duration_since(*ts) < CACHE_TTL).then_some(*v))
    }

    /// evaluate_and_show_popup 用的便捷查询:已确认是自动审批(true=应抑制)。
    /// 读不到 / 过期 / 非 AutoApproved 一律 false(放行)。
    pub fn is_auto_approved(&self, session_id: &str) -> bool {
        matches!(
            self.get(session_id),
            Some(AutoApprovalVerdict::AutoApproved)
        )
    }
}

impl Default for AutoApprovalCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 从一个 turn_context payload 判定审批 verdict。纯函数,可单测。
///
/// 三信号取"或":任一命中即 AutoApproved。
///   - approval_policy == "never"(永不找人审批)
///   - approvals_reviewer == "auto_review"
///   - model == "codex-auto-review"(ChatGPT 桌面端自动审批专用模型名,最强信号)
///
/// 三者都没命中且字段确实存在 -> UserApproval;字段缺失/类型不符 -> Unknown
/// (保守放行,宁可漏抑制也别误杀真警报)。
fn classify_turn_context(turn: &Value) -> AutoApprovalVerdict {
    let policy = turn.get("approval_policy").and_then(|v| v.as_str());
    let reviewer = turn.get("approvals_reviewer").and_then(|v| v.as_str());
    let model = turn.get("model").and_then(|v| v.as_str());

    let policy_is_never = policy.is_some_and(|p| p == "never");
    let reviewer_is_auto = reviewer.is_some_and(|r| r == "auto_review");
    let model_is_auto = model.is_some_and(|m| m == "codex-auto-review");

    if policy_is_never || reviewer_is_auto || model_is_auto {
        return AutoApprovalVerdict::AutoApproved;
    }

    // 三字段都读到了且都不命中自动审批 -> 明确是 user 审批。
    // 任一字段缺失(rollout schema 变了 / 部分写入)-> 保守 Unknown。
    if policy.is_some() || reviewer.is_some() || model.is_some() {
        AutoApprovalVerdict::UserApproval
    } else {
        AutoApprovalVerdict::Unknown
    }
}

/// 在 rollout 文件尾部内容里找**最后一条** turn_context,判定 verdict。
///
/// rollout 真实 schema(实测):turn_context 是**顶层事件类型**,不是包在 event_msg 里:
///   `{"timestamp":...,"type":"turn_context","payload":{"approval_policy":...,
///    "approvals_reviewer":...,"model":...,...}}`
/// 即 `type` 在根层,approval 字段在 `payload` 内。每轮事件序:
/// task_started -> turn_context -> user_message -> agent_message -> task_complete,
/// turn_context 每轮一条,最新轮的在文件尾部。倒序扫,命中第一个 `type=="turn_context"`
/// 即当前轮,取其 payload 判定。
///
/// 空 / 无 turn_context -> Unknown(放行)。解析单行失败跳过该行,不整体失败。
fn detect_auto_approval_in_lines(content: &str) -> AutoApprovalVerdict {
    for line in content.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let is_turn_context = val
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t == "turn_context");
        if !is_turn_context {
            continue;
        }
        // 命中最新一条 turn_context,取其 payload 判定。
        if let Some(payload) = val.get("payload") {
            return classify_turn_context(payload);
        }
        return AutoApprovalVerdict::Unknown;
    }
    AutoApprovalVerdict::Unknown
}

/// tail 读 rollout 文件末尾 `max_tail_bytes` 字节。照 transcript_watch 的
/// `read_tail_to_string`:从末尾偏移读,起点不在行首时丢掉第一行残行。
fn read_tail_to_string(path: &Path, max_tail_bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_tail_bytes);
    file.seek(SeekFrom::Start(start))?;

    let mut content = String::new();
    file.read_to_string(&mut content)?;

    if start > 0 {
        if let Some((_, rest)) = content.split_once('\n') {
            return Ok(rest.to_string());
        }
        return Ok(content);
    }
    Ok(content)
}

/// 给定 thread UUID,定位其 rollout 文件路径。
///
/// rollout 按日期分目录 `~/.codex/sessions/<YYYY/MM/DD>/`,文件名
/// `rollout-<ts>-<threadUUID>.jsonl`。同 thread 可能跨多天有多文件(会话跨日 /
/// 重开)。glob 今天 + 昨天两个日期目录(防刚过午夜最新文件还在昨天目录),
/// 取 mtime 最新的那个。
///
/// 返回 None:HOME 缺失 / 目录不存在 / 无匹配文件。
fn locate_rollout_path(thread_uuid: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let sessions_root = Path::new(&home).join(".codex").join("sessions");

    // 候选日期目录:今天 + 昨天(防跨午夜)。
    let today = chrono::Local::now().date_naive();
    let date_dirs: Vec<PathBuf> = [0i64, -1]
        .iter()
        .map(|offset| {
            let d = today + chrono::Duration::days(*offset);
            sessions_root.join(format!("{}/{:02}/{:02}", d.year(), d.month(), d.day()))
        })
        .collect();

    // glob `*-<threadUUID>.jsonl`,取 mtime 最新。
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for dir in &date_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(&format!("-{}.jsonl", thread_uuid)) {
                continue;
            }
            let path = entry.path();
            let mtime = entry.metadata().ok()?.modified().ok()?;
            match &best {
                Some((_, best_mtime)) if &mtime <= best_mtime => {}
                _ => best = Some((path, mtime)),
            }
        }
    }
    best.map(|(p, _)| p)
}

/// 对一个 codex session,读其 rollout 当前轮 turn_context,判定 verdict。
/// 纯函数(除文件 I/O),可对临时文件单测。
pub fn detect_auto_approval(thread_uuid: &str) -> AutoApprovalVerdict {
    let Some(path) = locate_rollout_path(thread_uuid) else {
        return AutoApprovalVerdict::Unknown;
    };
    let content = match read_tail_to_string(&path, TAIL_BYTES) {
        Ok(c) => c,
        Err(e) => {
            log_debug!(
                "codex_rollout",
                "read rollout failed thread={} path={} err={}",
                thread_uuid,
                path.display(),
                e
            );
            return AutoApprovalVerdict::Unknown;
        }
    };
    detect_auto_approval_in_lines(&content)
}

/// 哪些 session 需要轮询:codex 源 + 有 external_session_id(= thread UUID)。
/// 非 codex(cc/cursor/opencode)的审批语义不同,且 rollout 是 codex 专属,跳过。
fn eligible_session(session: &Session) -> bool {
    session
        .source
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("codex"))
        && session
            .external_session_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
}

/// 单轮扫描:snapshot 所有 codex session,更新 cache。返回本次刷新的 session 数。
pub fn scan_once(state: &AppState) -> usize {
    // snapshot sessions 时短暂持锁,不跨文件 I/O。
    let sessions: Vec<Session> = {
        let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.get_all().to_vec()
    };

    let mut refreshed = 0;
    for session in sessions.iter().filter(|s| eligible_session(s)) {
        let Some(thread_uuid) = session.external_session_id.as_deref() else {
            continue;
        };
        let verdict = detect_auto_approval(thread_uuid);
        // 只在判据变化(含首次)时记日志;稳定状态不刷,避免每 5s 重复记淹没真实事件。
        let changed = state.auto_approval.set(&session.id, verdict);
        if changed {
            log_info!(
                "codex_rollout",
                "auto-approval verdict changed task_id={} session_id={} thread={} verdict={:?}",
                session.task_id,
                session.id,
                thread_uuid,
                verdict
            );
        }
        refreshed += 1;
    }
    refreshed
}

/// 5s 轮询线程。照 transcript_watch::spawn_transcript_interruption_watcher。
/// W3(brief 20260813-watcher-supervisor):迁移到 supervisor::spawn_periodic
/// (命名线程 + catch_unwind + 心跳),body 单轮逻辑零变化。
pub fn spawn_codex_rollout_watcher(state: AppState) {
    crate::supervisor::spawn_periodic("codex-rollout", WATCH_INTERVAL, move || {
        let _ = scan_once(&state);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_context_json(policy: &str, reviewer: &str, model: &str) -> String {
        // 真实 schema:type 在根层,approval 字段在 payload 内(不在 event_msg 嵌套)。
        format!(
            r#"{{"timestamp":"2026-07-15T06:00:00.000Z","type":"turn_context","payload":{{"turn_id":"t1","approval_policy":"{}","approvals_reviewer":"{}","model":"{}"}}}}"#,
            policy, reviewer, model
        )
    }

    fn task_complete_json() -> &'static str {
        r#"{"timestamp":"2026-07-15T06:00:05.000Z","type":"event_msg","payload":{"type":"task_complete"}}"#
    }

    #[test]
    fn classify_policy_never_is_auto() {
        let v = serde_json::json!({"approval_policy":"never","approvals_reviewer":"auto_review","model":"codex-auto-review"});
        assert_eq!(classify_turn_context(&v), AutoApprovalVerdict::AutoApproved);
    }

    #[test]
    fn classify_reviewer_auto_only_is_auto() {
        // policy=on-request 但 reviewer=auto_review(中途切自动审批,如实测 019f644f)
        let v = serde_json::json!({"approval_policy":"on-request","approvals_reviewer":"auto_review","model":"ark-code-latest"});
        assert_eq!(classify_turn_context(&v), AutoApprovalVerdict::AutoApproved);
    }

    #[test]
    fn classify_model_auto_review_only_is_auto() {
        let v = serde_json::json!({"approval_policy":"on-request","approvals_reviewer":"user","model":"codex-auto-review"});
        assert_eq!(classify_turn_context(&v), AutoApprovalVerdict::AutoApproved);
    }

    #[test]
    fn classify_user_approval() {
        let v = serde_json::json!({"approval_policy":"on-request","approvals_reviewer":"user","model":"ark-code-latest"});
        assert_eq!(classify_turn_context(&v), AutoApprovalVerdict::UserApproval);
    }

    #[test]
    fn classify_missing_fields_is_unknown() {
        // 没有 approval 字段 -> 不能确定是自动审批 -> 放行
        let v = serde_json::json!({"type":"turn_context","turn_id":"t1"});
        assert_eq!(classify_turn_context(&v), AutoApprovalVerdict::Unknown);
    }

    #[test]
    fn detect_picks_latest_turn_context() {
        // 文件里有两条 turn_context(旧 user + 新 auto),应取最后一条(新)
        let content = format!(
            "{}\n{}\n{}\n",
            turn_context_json("on-request", "user", "ark-code-latest"),
            task_complete_json(),
            turn_context_json("never", "auto_review", "codex-auto-review"),
        );
        assert_eq!(
            detect_auto_approval_in_lines(&content),
            AutoApprovalVerdict::AutoApproved
        );
    }

    #[test]
    fn detect_user_session_not_auto() {
        let content = format!(
            "{}\n{}\n",
            turn_context_json("on-request", "user", "ark-code-latest"),
            task_complete_json(),
        );
        assert_eq!(
            detect_auto_approval_in_lines(&content),
            AutoApprovalVerdict::UserApproval
        );
    }

    #[test]
    fn detect_no_turn_context_is_unknown() {
        let content = format!("{}\n", task_complete_json());
        assert_eq!(
            detect_auto_approval_in_lines(&content),
            AutoApprovalVerdict::Unknown
        );
    }

    #[test]
    fn detect_empty_is_unknown() {
        assert_eq!(
            detect_auto_approval_in_lines(""),
            AutoApprovalVerdict::Unknown
        );
    }

    #[test]
    fn detect_skips_malformed_lines() {
        // 残行 / 非 JSON 行跳过,仍能从后面的合法行解析
        let content = format!(
            "garbage not json\n{}\n",
            turn_context_json("never", "auto_review", "codex-auto-review")
        );
        assert_eq!(
            detect_auto_approval_in_lines(&content),
            AutoApprovalVerdict::AutoApproved
        );
    }

    #[test]
    fn cache_returns_verdict_within_ttl() {
        let cache = AutoApprovalCache::new();
        cache.set("s1", AutoApprovalVerdict::AutoApproved);
        assert_eq!(cache.get("s1"), Some(AutoApprovalVerdict::AutoApproved));
        assert!(cache.is_auto_approved("s1"));
    }

    #[test]
    fn cache_set_reports_change_only_on_transition() {
        // 首次写入 -> changed;同值再写 -> unchanged(避免稳定状态每轮刷日志)。
        let cache = AutoApprovalCache::new();
        assert!(cache.set("s", AutoApprovalVerdict::AutoApproved)); // 首次
        assert!(!cache.set("s", AutoApprovalVerdict::AutoApproved)); // 同值
        assert!(cache.set("s", AutoApprovalVerdict::UserApproval)); // 转变
        assert!(cache.set("s", AutoApprovalVerdict::AutoApproved)); // 再转变
    }

    #[test]
    fn cache_unknown_and_missing_not_auto_approved() {
        let cache = AutoApprovalCache::new();
        cache.set("s2", AutoApprovalVerdict::Unknown);
        assert!(!cache.is_auto_approved("s2")); // Unknown -> 放行
        assert!(!cache.is_auto_approved("nonexistent")); // 未命中 -> 放行
    }

    #[test]
    fn cache_user_approval_not_auto_approved() {
        let cache = AutoApprovalCache::new();
        cache.set("s3", AutoApprovalVerdict::UserApproval);
        assert!(!cache.is_auto_approved("s3"));
    }

    #[test]
    fn cache_default_is_empty_and_safe() {
        // 生产 / 测试默认构造:所有查询返回 false(放行),不误杀
        let cache = AutoApprovalCache::default();
        assert!(!cache.is_auto_approved("anything"));
    }
}
