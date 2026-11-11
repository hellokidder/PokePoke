//! Ghostty terminal_id 周期对账。
//!
//! ## 为什么需要这个(brief 20260629-ghostty-jump-reconcile)
//!
//! SessionStart 时的单次 capture(http_server::spawn_ghostty_capture)依赖
//! "敲命令那一刻焦点恰好在该 tab" + "Ghostty 报的 cwd 跟 workspace 兼容"。
//! 真机实测两个前提经常不成立:
//!   - 冷启动(resume / compact / 后台 reconnect)焦点不在该 tab → capture 抓到无关 tab;
//!   - Ghostty 的 `working directory` 大面积停在家目录(OSC7 未持续上报)→ cwd 校验恒 fail,
//!     terminal_id 永不落库,跳转恒退化到 L2 cwd first-match(既漏又错绑)。
//!
//! 对账换一个**唯一且 CC 主动维护**的匹配键:Ghostty tab 的 `name`(title)里嵌的
//! `external_session_id` 前缀(形如 `poke-poke · <prompt> · 14f71163-274b-4d`)。
//! 周期枚举所有 terminal,只要某一刻 title 显示了 session-id,就把
//! `(session ↔ 稳定 UUID)` 绑定写回 terminal_id。**title 之后被命令名覆盖也不影响**:
//! 存的是 UUID(surface 生命周期恒定),跳转不再依赖 title。
//!
//! cwd 降为兜底键(title 抓不到时才用,沿用 surface::cwds_compatible 防污染)。

use crate::http_server::AppState;
use std::time::Duration;

/// 对账 tick 间隔。比 transcript_watch 的 5s 稍长:对账是"逮到一次就锁",
/// 不需要高频;且每 tick 要全量枚举 Ghostty terminal(osascript 有成本)。
const RECONCILE_INTERVAL: Duration = Duration::from_secs(8);

/// osascript 连续 TIMEOUT 时下一轮 sleep 的封顶。
///
/// **背景**(brief 20260715-ghostty-reconcile-timeout-backoff):Ghostty 一时无响应时,
/// `enumerate_ghostty_terminals` 每 8s 打一次 osascript,每次卡满 5s DEFAULT_TIMEOUT
/// 被 kill,稳态占用约 0.38 核 —— 现场遇 441% CPU 事故的直接起因。用**指数退避**
/// (8 → 16 → 32 → 60 封顶)把稳态压到 <0.02 核。
///
/// **一次成功立即归零**:任意一次 osascript 拿到结果(即 `Some`)就把 sleep 复位到
/// `RECONCILE_INTERVAL`,不留粘性。Ghostty 恢复后用户完全无感。
///
/// 60s 封顶:reconcile 是"最终一致性"补偿路径,不承担实时性;跳转/capture 独立走
/// commands/focus.rs,不受影响。60s 是一个既足够节能又不至于让"用户重启 Ghostty
/// 后半分钟 title-binding 还没修好"变成体感问题的折中。
const RECONCILE_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// `scan_once` 的返回:除了写回条数,还带出"osascript 是否成功"的信号,让 watcher
/// 主循环能据此调整下一轮 sleep。**业务逻辑与写回行为完全不受此字段影响**。
#[derive(Debug, Clone)]
pub struct ScanOutcome {
    /// 本轮向 store 写回的 terminal_id 绑定条数。
    pub written: usize,
    /// 本轮 `enumerate_ghostty_terminals` osascript 是否**在超时内拿到结果**
    /// (即使结果 vec 为空也算 true;只有 timeout / spawn 失败 / 非零退出才 false)。
    /// early-exit(没有 Ghostty 相关 session)也视为 true —— osascript 通道健康,
    /// 只是没工作可做,不应触发退避。
    pub osascript_ok: bool,
    /// 本轮弱证据 skip 事件(ambiguous-cwd / contested)。watcher 用它做状态转变 gate:
    /// 稳态 skip(同一 task_id 同一原因)不重复打日志,避免长活 ambiguous 会话刷屏。
    pub skips: Vec<SkipEvent>,
}

/// backoff 状态机:根据"上一轮 sleep 时长"和"本轮 osascript 是否 ok"决定下一轮 sleep。
///
/// - `osascript_ok = true`:立即归零到 `RECONCILE_INTERVAL`(8s),不管之前退到多远
/// - `osascript_ok = false`:上一轮 × 2,封顶 `RECONCILE_BACKOFF_MAX`(60s)
///
/// 纯函数,便于单测;不依赖任何外部状态。
fn next_sleep(prev: Duration, osascript_ok: bool) -> Duration {
    if osascript_ok {
        return RECONCILE_INTERVAL;
    }
    let doubled = prev.saturating_mul(2);
    if doubled > RECONCILE_BACKOFF_MAX {
        RECONCILE_BACKOFF_MAX
    } else if doubled < RECONCILE_INTERVAL {
        // 极端保护:prev 意外为 0 时至少走一个 RECONCILE_INTERVAL
        RECONCILE_INTERVAL
    } else {
        doubled
    }
}

/// 本轮某 session 的 skip 原因相比上轮是否构成"状态转变" -- 首次出现或原因变化。
/// watcher 据此决定是否打日志:稳态(同 task_id 同原因每 8s)不打,避免刷屏 +
/// 触发日志轮转把排查信号挤掉。纯函数,便于单测。
fn should_log_skip(prev: Option<&SkipReason>, cur: &SkipReason) -> bool {
    match prev {
        None => true,
        Some(p) => p != cur,
    }
}

/// session 的 source 是否属于可能跑在 Ghostty 里的行为体。
/// 同步更新 http_server::ghostty_capture_eligible。
fn session_source_is_ghostty_relevant(s: &crate::sessions::Session) -> bool {
    let src = s.source.as_deref().unwrap_or("");
    src.eq_ignore_ascii_case("claude-code")
        || src.eq_ignore_ascii_case("codex")
        || src.eq_ignore_ascii_case("opencode")
}

/// 全量枚举脚本里 id / cwd / title 三字段的分隔符(ASCII US,31),
/// 行间用 ASCII RS(30)。都跟普通终端文本撞不上。
const FIELD_SEP: char = '\u{001f}';
const RECORD_SEP: char = '\u{001e}';

/// 一次 Ghostty 全量枚举里的单个 terminal 快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhosttySnapshot {
    /// stable terminal surface UUID(跳转锚点,生命周期恒定)。
    pub terminal_id: String,
    /// terminal 当前 working directory(本机常为家目录,仅作兜底匹配键)。
    pub working_directory: String,
    /// terminal 的 name / title(含 session-id 前缀的主键来源,易变)。
    pub title: String,
}

/// 对账输入里需要被匹配的 session 最小投影(从 SessionStore 快照抽出,避免持锁跨 osascript)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileCandidate {
    pub task_id: String,
    pub external_session_id: Option<String>,
    pub workspace_path: Option<String>,
    /// 当前已存的 terminal_id(用于判断是"补空"还是"纠正错绑")。
    pub terminal_id: Option<String>,
}

/// 对账一个 session 的结论:把它绑到哪个 UUID,以及当时 snapshot 的 cwd(写回 captured_cwd)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileAssignment {
    pub task_id: String,
    pub terminal_id: String,
    pub captured_cwd: String,
    /// 命中方式,仅用于日志分桶(title 主键 / cwd 兜底)。
    pub matched_by: MatchKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKey {
    Title,
    Cwd,
}

/// session-id 前缀参与 title 匹配的最小长度。CC 把 title 截成 `14f71163-274b-4d`(16 字符),
/// 但短前缀(如纯 8 位 hex)在多 session 下有撞车风险。要求 title 里命中的连续前缀
/// ≥ 该长度才算 title 命中,否则视为没匹配上(降级 cwd 兜底)。
const MIN_TITLE_ID_PREFIX: usize = 8;

/// 判断 `title` 是否包含 `external_session_id` 的一个足够长的前缀。
///
/// CC 的 title 尾部是**截断**的 session id(`14f71163-274b-4d`),不是完整 UUID,
/// 所以不能用 `title.contains(full_id)`。改为:取 ext_id 的递减前缀,找 title 里
/// 包含的最长那个;命中长度 ≥ MIN_TITLE_ID_PREFIX 即算 title 命中。
///
/// 纯函数,无 IO。返回命中的前缀长度(便于"同一 session 多 snapshot 命中时取最长"),
/// 不命中返回 None。
pub fn title_matches_session_prefix(title: &str, external_session_id: &str) -> Option<usize> {
    let id = external_session_id.trim();
    if id.len() < MIN_TITLE_ID_PREFIX || title.is_empty() {
        return None;
    }
    // 从完整长度往下递减,找 title 包含的最长前缀。命中即返回(贪心,最长优先)。
    let max = id.len();
    for len in (MIN_TITLE_ID_PREFIX..=max).rev() {
        let prefix = &id[..len];
        if title.contains(prefix) {
            return Some(len);
        }
    }
    None
}

/// 把 Ghostty snapshots 对账到候选 sessions,返回需要写回的绑定。
///
/// 匹配优先级(每个 session 至多产出一条 assignment):
///   Pass 1 — title 主键:title 含 ext_id 前缀。一个 snapshot 只能被认领一次,
///            一个 session 只取命中前缀最长的那个 snapshot(防一个 session 撞多 tab)。
///   Pass 2 — cwd 兜底,**仅唯一匹配**:Pass1 没认领的 session,用 surface::cwds_compatible
///            跟剩余 snapshot 匹配,但只有 candidate↔snapshot 双向 degree 都为 1 时才 bind。
///            同 cwd 多候选(任一侧 degree≥2)一律 skip,不落库 —— 与 capture 层的
///            AmbiguousCwd 规律一致,不用弱证据把错绑洞开回来。
///
/// **只处理"需要对账"的 session**:terminal_id 为空(补空),或虽有值但当前没有任一
/// snapshot 的 UUID 等于它(疑似错绑/stale,允许纠正)。已有且 snapshot 仍在的 session
/// 不动(幂等,省写)。这个过滤由 caller 在传入 candidates 时决定,本函数对所有传入
/// candidate 一视同仁地匹配。
/// 本轮弱证据 skip 的原因。watcher 据此判断"状态是否变化"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// cwd 兼容的未认领 snapshot >=2 个,无法唯一绑定(正向 degree>=2)。
    AmbiguousCwd { compatible_snapshots: usize },
    /// 兼容的 snapshot 被多个未认领 session 争用(反向 degree>=2)。
    Contested { contenders: usize },
}

/// 一个 session 本轮因弱证据被 skip 的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipEvent {
    pub task_id: String,
    pub reason: SkipReason,
}

/// `reconcile_assignments` 的返回:成功落库的 assignments + 因弱证据 skip 的事件。
/// 把 skips 从"直接打日志"改成"带出给 caller",让 watcher 能跨 tick 做状态转变 gate,
/// 避免稳态 skip 每 8s 刷一条。
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileResult {
    pub assignments: Vec<ReconcileAssignment>,
    pub skips: Vec<SkipEvent>,
}

pub fn reconcile_assignments(
    snapshots: &[GhosttySnapshot],
    candidates: &[ReconcileCandidate],
) -> ReconcileResult {
    let mut assignments: Vec<ReconcileAssignment> = Vec::new();
    let mut skips: Vec<SkipEvent> = Vec::new();
    let mut claimed_snapshot: Vec<bool> = vec![false; snapshots.len()];
    let mut claimed_candidate: Vec<bool> = vec![false; candidates.len()];

    // Pass 1: title 主键。对每个 candidate 找命中前缀最长的未认领 snapshot。
    for (ci, cand) in candidates.iter().enumerate() {
        let Some(ext_id) = cand.external_session_id.as_deref() else {
            continue;
        };
        let mut best: Option<(usize, usize)> = None; // (snapshot_index, matched_len)
        for (si, snap) in snapshots.iter().enumerate() {
            if claimed_snapshot[si] {
                continue;
            }
            if let Some(len) = title_matches_session_prefix(&snap.title, ext_id) {
                if best.is_none_or(|(_, best_len)| len > best_len) {
                    best = Some((si, len));
                }
            }
        }
        if let Some((si, _)) = best {
            claimed_snapshot[si] = true;
            claimed_candidate[ci] = true;
            assignments.push(ReconcileAssignment {
                task_id: cand.task_id.clone(),
                terminal_id: snapshots[si].terminal_id.clone(),
                captured_cwd: snapshots[si].working_directory.clone(),
                matched_by: MatchKey::Title,
            });
        }
    }

    // Pass 2: cwd 兜底,但**只做 bipartite 唯一匹配**,绝不 first-match。
    //
    // capture 层已把"同 cwd 多候选 + 无 title key"判为弱证据、拒绝落库
    // (focus.rs 的 GhosttyMatch::AmbiguousCwd → 返回 None)。reconcile 不能用同一弱
    // 证据把 terminal_id 重新写回来 —— 否则 capture 堵上的错绑洞在这里又开回去
    // (多 tab / 多窗口同 cwd,枚举顺序决定谁绑谁,恒错)。
    //
    // 规则:某 candidate 恰好兼容 1 个未认领 snapshot,**且**该 snapshot 也恰好只兼容
    // 这 1 个未认领 candidate(1↔1)时才 bind。任一侧 degree≥2 一律 skip,只打诊断
    // 日志,等 title 强证据。单窗口 / OSC7 正常的兜底(唯一匹配)因此保留。
    //
    // 参与 Pass2 的 candidate:Pass1 未认领 且 有非空 workspace(否则无法 cwd 匹配)。
    let pass2_cands: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(ci, cand)| {
            !claimed_candidate[*ci]
                && cand
                    .workspace_path
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
        })
        .map(|(ci, _)| ci)
        .collect();

    // (ci, si) 是否 cwd 兼容。ci 一定有非空 workspace(上面已过滤)。
    let compatible = |ci: usize, si: usize| -> bool {
        let ws = candidates[ci].workspace_path.as_deref().unwrap_or("");
        crate::commands::cwds_compatible(&snapshots[si].working_directory, ws)
    };

    for &ci in &pass2_cands {
        // 该 candidate 兼容的、仍未认领的 snapshot。
        let compat_snaps: Vec<usize> = (0..snapshots.len())
            .filter(|&si| !claimed_snapshot[si] && compatible(ci, si))
            .collect();
        if compat_snaps.len() != 1 {
            if compat_snaps.len() >= 2 {
                skips.push(SkipEvent {
                    task_id: candidates[ci].task_id.clone(),
                    reason: SkipReason::AmbiguousCwd {
                        compatible_snapshots: compat_snaps.len(),
                    },
                });
            }
            continue;
        }
        let si = compat_snaps[0];

        // 反向 degree:该 snapshot 兼容的、仍未认领的 candidate 也必须唯一。
        // 否则两个 session 抢同一个 terminal,谁绑谁靠偶然 → 一律 skip。
        let compat_cands = pass2_cands
            .iter()
            .copied()
            .filter(|&other| !claimed_candidate[other] && compatible(other, si))
            .count();
        if compat_cands != 1 {
            skips.push(SkipEvent {
                task_id: candidates[ci].task_id.clone(),
                reason: SkipReason::Contested {
                    contenders: compat_cands,
                },
            });
            continue;
        }

        claimed_snapshot[si] = true;
        claimed_candidate[ci] = true;
        assignments.push(ReconcileAssignment {
            task_id: candidates[ci].task_id.clone(),
            terminal_id: snapshots[si].terminal_id.clone(),
            captured_cwd: snapshots[si].working_directory.clone(),
            matched_by: MatchKey::Cwd,
        });
    }

    ReconcileResult { assignments, skips }
}

// ===== IO 层:枚举 Ghostty / 周期 scan / spawn =====

/// 解析全量枚举脚本的输出:行(RECORD_SEP 分隔)→ 每行 `id\x1fcwd\x1ftitle`。
/// id 为空的行丢弃(无效 terminal)。纯函数,便于单测。
fn parse_enumerate_output(raw: &str) -> Vec<GhosttySnapshot> {
    raw.split(RECORD_SEP)
        .filter_map(|line| {
            let mut parts = line.splitn(3, FIELD_SEP);
            let id = parts.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let cwd = parts.next().unwrap_or("").trim();
            let title = parts.next().unwrap_or("").trim();
            Some(GhosttySnapshot {
                terminal_id: id.to_string(),
                working_directory: cwd.to_string(),
                title: title.to_string(),
            })
        })
        .collect()
}

/// 全量枚举所有 Ghostty terminal 的 (id, working directory, name)。
/// Ghostty 未运行 / sdef 缺失 / TCC 未授权 / osascript 超时被 kill → 返回 `None`
/// (**对账整轮空转,无害**)。`Some(vec)` 表示 osascript 通道健康,即使 vec 为空
/// (无窗口)也算成功 —— watcher 循环凭此信号决定是否退避。
fn enumerate_ghostty_terminals() -> Option<Vec<GhosttySnapshot>> {
    // 拓扑同 capture:application → window → tab → terminal。这里全量遍历,
    // 每个 terminal 取 id / working directory / name,字段间 US、行间 RS。
    let script = r#"set fieldSep to ASCII character 31
set recordSep to ASCII character 30
tell application "Ghostty"
    if not (it is running) then return ""
    set out to {}
    repeat with w in windows
        repeat with t in tabs of w
            repeat with term in terminals of t
                set tid to ""
                set tcwd to ""
                set tname to ""
                try
                    set tid to (id of term as text)
                end try
                try
                    set tcwd to (working directory of term as text)
                end try
                try
                    set tname to (name of term as text)
                end try
                set end of out to tid & fieldSep & tcwd & fieldSep & tname
            end repeat
        end repeat
    end repeat
    set AppleScript's text item delimiters to recordSep
    set joined to out as string
    set AppleScript's text item delimiters to ""
    return joined
end tell"#;

    // 带超时:枚举全部 Ghostty terminal 的 osascript,Ghostty 卡住时不该无限阻塞
    // 对账线程(见 osascript 模块,2026-07-01 主线程挂死事故)。
    crate::osascript::run(
        script,
        crate::osascript::DEFAULT_TIMEOUT,
        "ghostty",
        "reconcile_enumerate",
    )
    .map(|raw| parse_enumerate_output(raw.trim()))
}

/// 从 store 抽出"需要对账"的候选。
///
/// 判定规则(v1.0.1 review H5 修正):
///   - 有 external_session_id → **无条件纳入**,让 Pass1 title 强证据可覆盖已有绑定
///     (老 sessions.json 残留的 first-match 时代错绑;多 tab 同项目冷启动首轮误绑到
///     一个存活但不匹配的 UUID)。sessions.rs::reconcile_terminal_id 里 `by_title=true`
///     分支已支持覆盖,但此前这里的 filter 用"terminal_id 存活 = skip"卡在门口。
///   - 无 external_session_id + terminal_id 空 → 纳入(靠 workspace_path 走 Pass2 兜底)
///   - 无 external_session_id + terminal_id 已在存活枚举里 → skip(幂等,省 osascript)
///   - 无 external_session_id + terminal_id 不在存活枚举里 → 纳入(疑似 stale,补空)
///
/// 仅 Ghostty 相关 source(claude-code / codex)才纳入。
fn collect_candidates(
    sessions: &[crate::sessions::Session],
    live_ids: &std::collections::HashSet<String>,
) -> Vec<ReconcileCandidate> {
    sessions
        .iter()
        .filter(|s| {
            // WezTerm session 已有 pane/socket binding,不走 Ghostty reconcile 误绑。
            if matches!(s.surface, crate::surface::SurfaceBinding::Wezterm { .. }) {
                return false;
            }
            // 只管可能跑在 Ghostty 里的 source。iTerm2/Terminal/Cursor 有 tty 精准跳,不需要。
            if !session_source_is_ghostty_relevant(s) {
                return false;
            }
            // review H5:有 ext_id 时不看 terminal_id,允许 Pass1 title 主键覆盖已存错绑。
            if s.external_session_id
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            {
                return true;
            }
            match s.terminal_id.as_deref() {
                None => true,                       // 补空(靠 workspace 走 Pass2 兜底)
                Some(id) => !live_ids.contains(id), // stale 补空 / 已绑 skip
            }
        })
        .map(|s| ReconcileCandidate {
            task_id: s.task_id.clone(),
            external_session_id: s.external_session_id.clone(),
            workspace_path: s.workspace_path.clone(),
            terminal_id: s.terminal_id.clone(),
        })
        .collect()
}

/// 从 herdr snapshot 里选出跟一个 session 匹配的 pane。
///
/// 匹配规则(严格):
///   - agent == "claude" / "codex" / "opencode"(与 session_source_is_ghostty_relevant 一致)
///   - foreground_cwd 与 session.workspace_path 兼容(surface::cwds_compatible)
///   - **workspace 层唯一**:同一 workspace 内多个 pane 都兼容 → skip(避免撞车,
///     等下一轮 title 命中在 Ghostty tab 层已经区分)
///
/// 返回选中的 (pane_id, workspace_id);没匹配到返回 None。
pub fn pick_herdr_pane_for_session(
    panes: &[crate::herdr::HerdrPaneSnapshot],
    workspace_path: &str,
    session_source: Option<&str>,
) -> Option<(String, String)> {
    let source_agent = match session_source {
        Some("claude-code") => "claude",
        Some("codex") => "codex",
        Some("opencode") => "opencode",
        _ => return None,
    };
    let candidates: Vec<&crate::herdr::HerdrPaneSnapshot> = panes
        .iter()
        .filter(|p| p.agent.as_deref() == Some(source_agent))
        .filter(|p| {
            let cwd = if !p.foreground_cwd.is_empty() {
                &p.foreground_cwd
            } else {
                &p.cwd
            };
            crate::commands::cwds_compatible(cwd, workspace_path)
        })
        .collect();
    // 严格唯一:多候选 skip,等下一轮 title 层强证据在 ghostty_reconcile 主 pass 里
    // 兜底(段 A 已经解决多 pane 的 title 分辨,段 B 只需要"我到底属于哪个 pane")。
    if candidates.len() != 1 {
        return None;
    }
    let p = candidates[0];
    Some((p.pane_id.clone(), p.workspace_id.clone()))
}

/// 跑一轮对账。返回 [`ScanOutcome`]:写回条数 + osascript 通道健康信号(供 watcher
/// 主循环决定退避)。
/// 关键并发纪律(镜像 transcript_watch::scan_once):**枚举 osascript 期间不持 store 锁**。
pub fn scan_once(state: &AppState) -> ScanOutcome {
    // 1. 先快照 sessions(短暂持锁)。
    let sessions = {
        let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.get_all().to_vec()
    };

    // 2. 没有任何 Ghostty 相关 session 直接返回,省掉 osascript。
    //    osascript 没跑 → 通道无判据 → 视为 ok(不触发退避)。
    let any_relevant = sessions.iter().any(session_source_is_ghostty_relevant);
    if !any_relevant {
        return ScanOutcome {
            written: 0,
            osascript_ok: true,
            skips: Vec::new(),
        };
    }

    // 3. 全量枚举(不持锁)。
    //    `None` = osascript 通道不健康(timeout / spawn 失败),交给 watcher 退避;
    //    `Some(vec)` = 通道健康,vec 空只表示当前没窗口,仍归 ok。
    let snapshots = match enumerate_ghostty_terminals() {
        Some(v) => v,
        None => {
            return ScanOutcome {
                written: 0,
                osascript_ok: false,
                skips: Vec::new(),
            };
        }
    };
    if snapshots.is_empty() {
        return ScanOutcome {
            written: 0,
            osascript_ok: true,
            skips: Vec::new(),
        };
    }
    let live_ids: std::collections::HashSet<String> =
        snapshots.iter().map(|s| s.terminal_id.clone()).collect();

    // 4. 过滤出需对账的候选;都不需要就早退。
    //    osascript 已成功,通道健康,不触发退避。
    let candidates = collect_candidates(&sessions, &live_ids);
    if candidates.is_empty() {
        return ScanOutcome {
            written: 0,
            osascript_ok: true,
            skips: Vec::new(),
        };
    }

    // 5. 纯函数匹配。
    let result = reconcile_assignments(&snapshots, &candidates);
    if result.assignments.is_empty() {
        return ScanOutcome {
            written: 0,
            osascript_ok: true,
            skips: result.skips,
        };
    }
    let assignments = result.assignments;

    // 6. 写回(短暂持锁)。
    let mut written = 0;
    {
        let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        for a in &assignments {
            let by_title = a.matched_by == MatchKey::Title;
            if store.reconcile_terminal_id(
                &a.task_id,
                a.terminal_id.clone(),
                a.captured_cwd.clone(),
                by_title,
            ) {
                written += 1;
                crate::log_info!(
                    "ghostty",
                    "reconcile bound task_id={} terminal_id={} matched_by={:?} captured_cwd={}",
                    a.task_id,
                    a.terminal_id,
                    a.matched_by,
                    a.captured_cwd
                );
            }
        }
    }

    // ==== herdr binding stale 刷新 ====
    //
    // 与 Ghostty terminal_id reconcile 独立:此处只关心 herdr binding,不管
    // Ghostty tab 层匹配。**只刷新已有 binding,不新建** —— 新建只信 pokehook
    // SessionStart 从 CC 进程环境变量采到的 HERDR_PANE_ID 三件套(那才能证明
    // "这个 CC 确实跑在 herdr pane 里")。cwd + agent=claude 唯一匹配是弱证据,
    // 会把"直接跑在 Ghostty 里的 CC"误绑到同 cwd 的 herdr pane,后续段 B 就把
    // 用户传送到错的地方。
    //
    // 走的规则:
    //   - session 无 herdr_binding → 不动(等 pokehook 上报)
    //   - session 有 herdr_binding + pane_id 仍在 snapshot 里 → 不动(幂等)
    //   - session 有 herdr_binding + pane_id 已不在 snapshot 里 → 视为 stale,
    //     用 workspace_path + agent 兜底重挑(用户在 herdr 里 split/kill 换了
    //     pane_id,这里认。source 里 pokehook 已经证过它属于 herdr。)
    //
    // 按每个 herdr server socket 分组 reconcile —— 多 named session 时不能用
    // find_map 取任意一个 socket 去刷所有 binding,否则会绑错 pane。
    let mut sockets: std::collections::HashSet<String> = sessions
        .iter()
        .filter_map(|s| {
            crate::surface::SurfaceBinding::herdr_fields(s)
                .and_then(|(_, _, socket)| socket.map(str::to_string))
        })
        .collect();

    for sock_str in sockets.drain() {
        let sock = std::path::PathBuf::from(&sock_str);
        let panes = crate::herdr::list_agent_panes(&sock);
        if panes.is_empty() {
            continue;
        }
        let live_pane_ids: std::collections::HashSet<String> =
            panes.iter().map(|p| p.pane_id.clone()).collect();
        let mut store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        let candidates: Vec<(String, String, Option<String>)> = store
            .sessions
            .iter()
            .filter(|s| {
                crate::surface::SurfaceBinding::herdr_fields(s).and_then(|(_, _, socket)| socket)
                    == Some(sock_str.as_str())
            })
            .filter(|s| match crate::surface::SurfaceBinding::herdr_fields(s) {
                None => false,
                Some((pane_id, _, _)) => !live_pane_ids.contains(pane_id),
            })
            .filter_map(|s| {
                s.workspace_path
                    .as_deref()
                    .map(|ws| (s.task_id.clone(), ws.to_string(), s.source.clone()))
            })
            .collect();
        for (task_id, ws, source) in candidates {
            let Some((pane_id, workspace_id)) =
                pick_herdr_pane_for_session(&panes, &ws, source.as_deref())
            else {
                continue;
            };
            let changed = store.set_surface_binding(
                &task_id,
                crate::surface::SurfaceBinding::Herdr {
                    pane_id: pane_id.clone(),
                    workspace_id: Some(workspace_id),
                    socket_path: Some(sock_str.clone()),
                },
            );
            if changed {
                crate::log_info!(
                    "herdr",
                    "reconcile bound herdr task_id={} pane_id={} sock={}",
                    task_id,
                    pane_id,
                    sock_str
                );
            }
        }
    }

    // 7. 有写回则通知前端刷新(terminal_id 变化影响跳转,但不影响列表展示;
    //    仍发一次让 sessions.json 落盘后状态一致)。
    if written > 0 {
        state.events.sessions_updated();
    }
    ScanOutcome {
        written,
        osascript_ok: true,
        skips: result.skips,
    }
}

/// 起后台对账线程。镜像 transcript_watch::spawn_transcript_interruption_watcher。
///
/// **退避语义**(brief 20260715-ghostty-reconcile-timeout-backoff):Ghostty AppleScript
/// 通道正常时每 [`RECONCILE_INTERVAL`] 跑一次;连续 osascript TIMEOUT 会让下次 sleep
/// 走 [`next_sleep`] 指数退避,封顶 [`RECONCILE_BACKOFF_MAX`]。任意一次成功立即归零。
/// 这只影响**下轮 sleep 时长**,不影响 `scan_once` 内部对账判定与写回。
///
/// W3(brief 20260813-watcher-supervisor):迁移到 supervisor::spawn_loop_guarded
/// (命名线程 + catch_unwind),保留原 sleep/退避结构;循环体开头 heartbeat,
/// 其余 body 逻辑零变化。
pub fn spawn_ghostty_reconcile_watcher(state: AppState) {
    crate::supervisor::spawn_loop_guarded("ghostty-reconcile", RECONCILE_INTERVAL, move |entry| {
        let mut sleep_dur = RECONCILE_INTERVAL;
        // 上一轮各 session 的 skip 原因,用于状态转变 gate:稳态不重复打日志。
        let mut prev_skips: std::collections::HashMap<String, SkipReason> =
            std::collections::HashMap::new();
        loop {
            crate::supervisor::heartbeat(entry);
            std::thread::sleep(sleep_dur);
            let ghostty_relevant_sessions = {
                let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
                store
                    .get_all()
                    .iter()
                    .filter(|s| session_source_is_ghostty_relevant(s))
                    .count()
            };
            let outcome = scan_once(&state);
            sleep_dur = next_sleep(sleep_dur, outcome.osascript_ok);
            if let Some(metrics) = crate::runtime_metrics::global() {
                metrics.record_ghostty_scan(
                    outcome.osascript_ok,
                    ghostty_relevant_sessions,
                    outcome.written,
                    sleep_dur.as_secs(),
                );
            }
            // 状态转变 gate:只在首次进入或原因变化时打日志。稳态(同 task_id
            // 同原因每 8s 一条)不打,避免长活 ambiguous 会话刷屏 + 触发日志轮转。
            let cur_skips: std::collections::HashMap<String, SkipReason> = outcome
                .skips
                .iter()
                .map(|s| (s.task_id.clone(), s.reason.clone()))
                .collect();
            for ev in &outcome.skips {
                if should_log_skip(prev_skips.get(&ev.task_id), &ev.reason) {
                    match &ev.reason {
                        SkipReason::AmbiguousCwd {
                            compatible_snapshots,
                        } => {
                            crate::log_info!(
                                "ghostty",
                                "reconcile skip ambiguous-cwd task_id={} compatible_snapshots={} (无 title key,弱证据不落库,等强证据)",
                                ev.task_id,
                                compatible_snapshots
                            );
                        }
                        SkipReason::Contested { contenders } => {
                            crate::log_info!(
                                "ghostty",
                                "reconcile skip ambiguous-cwd task_id={} snapshot 被 {} 个 session 争用(无 title key,弱证据不落库)",
                                ev.task_id,
                                contenders
                            );
                        }
                    }
                }
            }
            prev_skips = cur_skips;
        }
    });
}

#[cfg(test)]
#[path = "ghostty_reconcile/tests.rs"]
mod tests;
