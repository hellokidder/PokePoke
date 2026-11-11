//! Ghostty terminal 枚举 / capture / cwd 兼容判定(从 `ghostty.rs` 拆出,
//! 2026-08-10 段 C Ghostty 宿主分支重构,brief 20260810-herdr-remote-ghostty-jumpback)。
//! 不持有任何 focus 副作用:枚举快照 + 稳定 id 选择 + cwd 校验,供
//! `ghostty.rs`(L1/L2/L3 跳转)与 `ghostty_herdr.rs`(herdr tab anchor / 段 C)消费。

use super::applescript::{is_app_running, run_applescript_str};

/// 0.0.5+:capture 返回 (id, cwd) 二元组,让 caller 跟 session.workspace_path
/// 校验后再决定要不要落库。原来只返回 id 没法做 cross-host 防误,7225a45 放宽
/// gate 之后引入"乱写无关 tab id"风险,此 struct 让验证可行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedGhostty {
    pub id: String,
    pub cwd: String,
}

/// SessionStart 事件到达 server 时,如果 source 是 Ghostty(或 0.0.5+ 放宽 gate
/// 后:任何 terminal_app 缺失的 session),异步抓 stable terminal id + 当前 cwd。
/// 时机假设:**热启动**(用户刚敲完 `claude/codex/cursor` 命令)焦点 100% 在该
/// tab —— id 一定对。**冷启动**(cc resume / compact / background reconnect)
/// 用户可能切去别 tab,这里抓到的可能是无关 tab,**必须靠 caller 做 cwd 校验**
/// 才能落库。
///
/// 失败(Ghostty 未运行 / < 1.3 没 sdef / 用户瞬间切走 tab / TCC 未授权)
/// 都返回 None,server 端走 cwd first-match 兜底。
/// 详见 brief 20260520-ghostty-terminal-focus.md。
///
/// 2026-07-02 重构(brief 20260702-ghostty-capture-enumerate):不再赌 `front window`。
/// 旧实现抓"capture 那一刻的前台窗口",而 capture 是 SessionStart 后异步跑的
/// (~200ms 后),用户敲完 `cursor-agent` 顺手切到别的项目窗口就会抓错 → cwd 不符
/// REJECT → terminal_id 永 None → 点击退 L2 cwd 模糊匹配,同 cwd 多窗口跳错。
/// 新实现**枚举所有 terminal**,按 session 的 (workspace_path, task_id) 精确挑出属于
/// 本会话的那个 —— 发起窗口即使已切走仍在枚举列表里。参数由 caller
/// (`spawn_ghostty_capture`)从 session 透传。
///
/// `workspace_path` 为空时(理论上 ghostty gate 已要求非空,防御性保留)退回旧的
/// front-window 抓取。
pub fn capture_ghostty_focused_terminal_id(
    workspace_path: Option<String>,
    task_id: Option<String>,
) -> Option<CapturedGhostty> {
    if !is_app_running("Ghostty") {
        return None;
    }

    let ws = workspace_path.as_deref().unwrap_or("");
    if ws.is_empty() {
        // 防御性兜底:无 workspace 无法做枚举匹配,退回 front-window(旧行为)。
        return capture_ghostty_front_window_with_retry();
    }
    let key = task_id.as_deref().and_then(task_id_match_key);

    // 枚举重试:agent 刚启动时目标 terminal 的 working directory 可能还没落位
    // (OSC7 未上报),同旧实现 3 拍节奏,拿到 cwd 兼容的候选即定。
    for delay_ms in [0_u64, 75, 150] {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        let candidates = enumerate_ghostty_terminals();
        match pick_ghostty_terminal(&candidates, ws, key) {
            GhosttyMatch::TitleKey(c) => {
                crate::log_info!(
                    "ghostty",
                    "capture pick via title-key key={} id={} cwd={}",
                    key.unwrap_or(""),
                    c.id,
                    c.cwd,
                );
                return Some(c);
            }
            GhosttyMatch::UniqueCwd(c) => {
                crate::log_info!(
                    "ghostty",
                    "capture pick via unique-cwd id={} cwd={}",
                    c.id,
                    c.cwd,
                );
                return Some(c);
            }
            GhosttyMatch::AmbiguousCwd(c) => {
                // 同 cwd 多窗口且 title 无 session-id 可消歧:workspace_path 相同只能证明
                // "可能是这个目录",不能唯一定位是哪个 Ghostty terminal。此时若写库(挑第一个
                // cwd 兼容的),同 cwd 多 session 会绑到同一个 terminal_id → popup/⌘N 跳错会话。
                // 只做诊断,不落库:保持 terminal_id=None,交给周期 reconcile 用强证据
                // (title 命中 external_session_id / cwd 兜底逐一认领 snapshot)去唯一绑定。
                crate::log_info!(
                    "ghostty",
                    "capture SKIP ambiguous-cwd (multiple same-cwd windows, no title key) sample_id={} cwd={} (不落库,等 reconcile 强证据绑定)",
                    c.id,
                    c.cwd,
                );
                return None;
            }
            GhosttyMatch::None => continue,
        }
    }
    crate::log_info!(
        "ghostty",
        "capture no cwd-compatible terminal for workspace={} (发起窗口可能已关 / Ghostty 未运行该项目)",
        ws,
    );
    None
}

/// task_id(`{source}-{8hex}`)里用于匹配窗口 title 的 key。取 `-` 后的 short_key
/// (8 位),cc/codex/cursor 的 Ghostty 窗口 title 都含这段(cc title 尾部
/// `…· <8hex>-…`,cursor title `…· cursor-<8hex>-`)。`unknown` 之类退化 key 照样
/// 走(title 不会含,Pass 1 自然 no-op,退 cwd 匹配)。
fn task_id_match_key(task_id: &str) -> Option<&str> {
    task_id
        .split_once('-')
        .map(|(_, key)| key)
        .filter(|k| !k.is_empty())
}

/// 枚举到的单个 terminal 候选(供 ghostty_herdr.rs 段 C 复用)。字段全 pub(super)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GhosttyCandidate {
    pub(super) id: String,
    pub(super) cwd: String,
    pub(super) title: String,
}

/// `pick_ghostty_terminal` 的结果,变体即"命中原因",便于 caller 分桶打 log + 单测断言。
#[derive(Debug, Clone, PartialEq, Eq)]
enum GhosttyMatch {
    /// title 含 task_id key 且 cwd 兼容 —— 最精确,同 cwd 多窗口靠这个消歧。
    TitleKey(CapturedGhostty),
    /// 恰好一个 cwd 兼容的候选 —— 常见场景。
    UniqueCwd(CapturedGhostty),
    /// 多个 cwd 兼容但无 title key 命中 —— 弱证据,caller 只用于诊断**不落库**
    /// (同 cwd 多 session 会撞同一个 id),交给 reconcile 强证据绑定。
    AmbiguousCwd(CapturedGhostty),
    /// 无 cwd 兼容候选。
    None,
}

/// 纯函数:从枚举到的所有 Ghostty terminal 里挑出属于本 session 的那个。
/// 匹配优先级(稳定性递减,见各变体注释):TitleKey → UniqueCwd → AmbiguousCwd → None。
///
/// title 匹配只在 **cwd 也兼容** 的候选里做,避免 title 里恰好含同段 hex 的无关窗口
/// (跨项目)误命中。cwd 兼容判定复用 `cwds_compatible`(与 L1 verify 同口径)。
fn pick_ghostty_terminal(
    candidates: &[GhosttyCandidate],
    workspace_path: &str,
    task_id_key: Option<&str>,
) -> GhosttyMatch {
    let cwd_compatible: Vec<&GhosttyCandidate> = candidates
        .iter()
        .filter(|c| !c.id.is_empty() && cwds_compatible(&c.cwd, workspace_path))
        .collect();

    if cwd_compatible.is_empty() {
        return GhosttyMatch::None;
    }

    // Pass 1: title 含 task_id key(在 cwd 兼容子集里)。
    if let Some(key) = task_id_key.filter(|k| !k.is_empty()) {
        let key_lower = key.to_lowercase();
        if let Some(c) = cwd_compatible
            .iter()
            .find(|c| c.title.to_lowercase().contains(&key_lower))
        {
            return GhosttyMatch::TitleKey(CapturedGhostty {
                id: c.id.clone(),
                cwd: c.cwd.clone(),
            });
        }
    }

    // Pass 2/3: cwd 兼容唯一 → UniqueCwd;多个 → AmbiguousCwd(取第一个)。
    let first = cwd_compatible[0];
    let picked = CapturedGhostty {
        id: first.id.clone(),
        cwd: first.cwd.clone(),
    };
    if cwd_compatible.len() == 1 {
        GhosttyMatch::UniqueCwd(picked)
    } else {
        GhosttyMatch::AmbiguousCwd(picked)
    }
}

/// 枚举当前所有 Ghostty terminal 的 (id, cwd, title)。`terminals` 直接给出跨窗口
/// 的所有 surface(实测无需嵌套 window→tab)。字段用 \x1f、记录用 \x1e 分隔
/// (跟普通文本撞不上)。任一属性读失败该字段留空,不影响其它记录。
/// `pub(super)`:ghostty_herdr.rs(段 C)复用。
pub(super) fn enumerate_ghostty_terminals() -> Vec<GhosttyCandidate> {
    let script = r#"tell application "Ghostty"
    set fieldSep to (ASCII character 31)
    set recSep to (ASCII character 30)
    set outLines to {}
    repeat with t in terminals
        set tid to ""
        set tcwd to ""
        set tname to ""
        try
            set tid to (id of t as text)
        end try
        try
            set tcwd to (working directory of t as text)
        end try
        try
            set tname to (name of t as text)
        end try
        set end of outLines to tid & fieldSep & tcwd & fieldSep & tname
    end repeat
    set AppleScript's text item delimiters to recSep
    set joined to outLines as string
    set AppleScript's text item delimiters to ""
    return joined
end tell"#;
    run_applescript_str(script)
        .map(|raw| parse_ghostty_enumeration(&raw))
        .unwrap_or_default()
}

/// 解析枚举脚本输出:记录用 \x1e 分隔,每条 `id \x1f cwd \x1f title`。id 空的记录丢弃。
fn parse_ghostty_enumeration(raw: &str) -> Vec<GhosttyCandidate> {
    raw.split('\u{001e}')
        .filter_map(|rec| {
            let mut fields = rec.splitn(3, '\u{001f}');
            let id = fields.next().unwrap_or("").trim().to_string();
            let cwd = fields.next().unwrap_or("").trim().to_string();
            let title = fields.next().unwrap_or("").trim().to_string();
            if id.is_empty() {
                return None;
            }
            Some(GhosttyCandidate { id, cwd, title })
        })
        .collect()
}

/// front-window 抓取 + 3 拍重试(旧行为)。仅在无 workspace 的防御性路径用。
fn capture_ghostty_front_window_with_retry() -> Option<CapturedGhostty> {
    let mut attempts = Vec::new();
    for delay_ms in [0_u64, 75, 150] {
        if delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        let captured = capture_ghostty_focused_terminal_once();
        let has_cwd = captured.as_ref().is_some_and(|c| !c.cwd.is_empty());
        attempts.push(captured);
        if has_cwd {
            break;
        }
    }

    choose_stable_ghostty_capture(attempts)
}

fn capture_ghostty_focused_terminal_once() -> Option<CapturedGhostty> {
    // Ghostty 1.3+ AppleScript dictionary 拓扑:application → window → tab → terminal。
    // `focused terminal` 属性挂在 **tab** 上(不是 window),所以必须穿 `selected tab`:
    //
    //   id of focused terminal of selected tab of front window
    //
    // 早期写成 `focused terminal of front window` 会报
    // "不能获得 focused terminal of front window",capture 永远 None。
    // sdef 明确 `id` 是 "Stable ID for this terminal surface",surface lifetime 内不变。
    //
    // 0.0.5+:同步抓 `working directory`(同 terminal object 的属性)用于 caller
    // 做 cwd 校验。两个值用 `\x1e`(RS, ASCII record separator,跟普通字符
    // 撞不上)分隔。AppleScript 任一查询失败整体 return "" → None,语义跟原
    // single-value 一致(对旧 caller 友好,失败一并失败)。
    let script = r#"tell application "Ghostty"
    try
        set t to focused terminal of selected tab of front window
        return (id of t as text) & (ASCII character 30) & (working directory of t as text)
    on error
        return ""
    end try
end tell"#;
    run_applescript_str(script).and_then(|raw| parse_ghostty_capture_raw(&raw))
}

fn parse_ghostty_capture_raw(raw: &str) -> Option<CapturedGhostty> {
    let mut parts = raw.splitn(2, '\u{001e}');
    let id = parts.next()?.trim().to_string();
    let cwd = parts.next()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    Some(CapturedGhostty { id, cwd })
}

fn choose_stable_ghostty_capture(
    attempts: Vec<Option<CapturedGhostty>>,
) -> Option<CapturedGhostty> {
    let captures: Vec<CapturedGhostty> = attempts.into_iter().flatten().collect();
    let first = captures.first()?;

    if captures.iter().any(|c| c.id != first.id) {
        return None;
    }

    captures
        .iter()
        .find(|c| !c.cwd.is_empty())
        .cloned()
        .or_else(|| captures.last().cloned())
}

/// 校验 capture 抓到的 cwd 跟 session 的 workspace_path 是否"够接近"。
///
/// 通过条件(任一即过):
/// - **完全相等**(`cap == sess`)
/// - **captured 是 session 子目录**(`cap` starts with `sess/`)—— shell 启动后
///   用户在 cc 里 `cd src/tauri` 之类
///
/// **不允许反方向**(`sess` starts with `cap/`)—— 历史上这条曾允许,但实测会触发
/// "Ghostty 前台是 home shell (/Users/edy) → 任意 workspace 都被误套家目录 tab
/// id" 的 false positive:`/Users/edy` 是几乎所有 workspace 的祖先,放开反向
/// 等于让 home 的 tab id 污染所有 session。session 比 capture 深的合法场景
/// (`cc --workspace=sub` 从 `/proj` 启动)罕见,代价是不写 id 走 L2 cwd 兜底,
/// 净影响小;污染家目录 id 的代价是 click-jump 稳定跳错家目录,严重得多。
///
/// 都为空或都 None → false,不算匹配。抽 pure fn 主要为单测。
pub fn cwds_compatible(captured: &str, session_workspace: &str) -> bool {
    let cap = captured.trim_end_matches('/');
    let sess = session_workspace.trim_end_matches('/');
    if cap.is_empty() || sess.is_empty() {
        return false;
    }
    // 正向：capture 是 session 的子目录（用户 cd 到子目录）
    if cap == sess || cap.starts_with(&format!("{sess}/")) {
        return true;
    }
    // 反向：session 是 capture 的子目录（terminal 在 monorepo 根，session 在子项目）。
    // 两层防护防家目录污染：
    //   1. captured 至少 3 个路径段（/a/b/c）——2 段的 /Users/edy 太浅
    //   2. captured 末段不能是已知泛目录名（Documents、Downloads 等），否则
    //      /Users/edy/Documents 会套到所有子项目
    if sess.starts_with(&format!("{cap}/")) {
        let segments: Vec<&str> = cap.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 3 {
            return false;
        }
        if let Some(last) = segments.last() {
            if matches!(
                last.to_lowercase().as_str(),
                "documents" | "downloads" | "desktop" | "home" | "users" | "tmp" | "var"
            ) {
                return false;
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod cwds_compatible_tests {
    use super::{
        choose_stable_ghostty_capture, cwds_compatible, parse_ghostty_capture_raw, CapturedGhostty,
    };

    #[test]
    fn parse_ghostty_capture_raw_extracts_id_and_cwd() {
        assert_eq!(
            parse_ghostty_capture_raw("term-1\u{001e}/Users/edy/proj"),
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "/Users/edy/proj".into(),
            })
        );
    }

    #[test]
    fn parse_ghostty_capture_raw_rejects_empty_id() {
        assert_eq!(parse_ghostty_capture_raw("\u{001e}/Users/edy/proj"), None);
        assert_eq!(parse_ghostty_capture_raw(""), None);
    }

    #[test]
    fn parse_ghostty_capture_raw_allows_empty_cwd_for_caller_rejection() {
        assert_eq!(
            parse_ghostty_capture_raw("term-1\u{001e}"),
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            })
        );
    }

    #[test]
    fn choose_stable_capture_prefers_later_non_empty_cwd_for_same_id() {
        let attempts = vec![
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            }),
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "/Users/edy/proj".into(),
            }),
        ];

        assert_eq!(
            choose_stable_ghostty_capture(attempts),
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "/Users/edy/proj".into(),
            })
        );
    }

    #[test]
    fn choose_stable_capture_rejects_id_change_during_retry() {
        let attempts = vec![
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            }),
            Some(CapturedGhostty {
                id: "term-2".into(),
                cwd: "/Users/edy/other".into(),
            }),
        ];

        assert_eq!(choose_stable_ghostty_capture(attempts), None);
    }

    #[test]
    fn choose_stable_capture_returns_empty_cwd_when_id_stable_but_cwd_never_appears() {
        let attempts = vec![
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            }),
            None,
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            }),
        ];

        assert_eq!(
            choose_stable_ghostty_capture(attempts),
            Some(CapturedGhostty {
                id: "term-1".into(),
                cwd: "".into(),
            })
        );
    }

    #[test]
    fn exact_match() {
        assert!(cwds_compatible("/Users/edy/proj", "/Users/edy/proj"));
    }

    #[test]
    fn trailing_slash_normalized() {
        assert!(cwds_compatible("/Users/edy/proj/", "/Users/edy/proj"));
        assert!(cwds_compatible("/Users/edy/proj", "/Users/edy/proj/"));
    }

    #[test]
    fn capture_is_subdir_of_session_workspace() {
        // 用户在 cc 启动后 `cd src/` 之类,Ghostty 报子目录,session.workspace_path 是根
        assert!(cwds_compatible(
            "/Users/edy/proj/src/tauri",
            "/Users/edy/proj"
        ));
    }

    #[test]
    fn session_workspace_deeper_than_capture_with_depth_guard() {
        // 反方向允许 monorepo 场景：terminal 在根目录,session 在子项目。
        // 深度门槛（>=4 层）防家目录污染。
        // `/Users/edy/proj` (4 层) → 允许
        assert!(cwds_compatible(
            "/Users/edy/proj",
            "/Users/edy/proj/src/tauri"
        ));
        // 真实场景：terminal 在 poke-poke 根,session workspace 是 src-tauri
        assert!(cwds_compatible(
            "/Users/edy/Documents/Kidder/poke-poke",
            "/Users/edy/Documents/Kidder/poke-poke/src-tauri"
        ));
        // 浅层目录仍被拒绝——家目录污染防护:
        assert!(!cwds_compatible(
            "/Users/edy",
            "/Users/edy/Documents/Hungry/star-maker"
        ));
        // `/Users/edy/Documents` (3 层) 也不够深
        assert!(!cwds_compatible(
            "/Users/edy/Documents",
            "/Users/edy/Documents/Hungry/star-maker"
        ));
    }

    #[test]
    fn unrelated_paths_reject() {
        // 主要保护场景:用户在 iTerm2 跑 cc-workspace-A,Ghostty 有个 workspace-B 的 tab 在前
        assert!(!cwds_compatible(
            "/Users/edy/Hungry/solutionhub",
            "/Users/edy/Kidder/poke-poke"
        ));
    }

    #[test]
    fn sibling_prefix_does_not_falsely_match() {
        // /home/proj vs /home/project — 前缀字符串匹配但路径不同,必须靠 "/" 分隔判定
        assert!(!cwds_compatible("/home/project", "/home/proj"));
        assert!(!cwds_compatible("/home/proj", "/home/project"));
    }

    #[test]
    fn empty_inputs_reject() {
        assert!(!cwds_compatible("", "/Users/edy/proj"));
        assert!(!cwds_compatible("/Users/edy/proj", ""));
        assert!(!cwds_compatible("", ""));
    }

    #[test]
    fn root_only_reject_unless_both_root() {
        // 守住"任何路径都跟 / 兼容"的退化:trim 后 "/" -> "",empty_inputs_reject 拦下
        assert!(!cwds_compatible("/", "/Users/edy/proj"));
        assert!(!cwds_compatible("/Users/edy/proj", "/"));
    }
}
#[cfg(test)]
mod ghostty_enumeration_tests {
    use super::{
        parse_ghostty_enumeration, pick_ghostty_terminal, task_id_match_key, CapturedGhostty,
        GhosttyCandidate, GhosttyMatch,
    };

    fn cand(id: &str, cwd: &str, title: &str) -> GhosttyCandidate {
        GhosttyCandidate {
            id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
        }
    }

    #[test]
    fn task_id_match_key_strips_source_prefix() {
        assert_eq!(task_id_match_key("cursor-acd40fce"), Some("acd40fce"));
        assert_eq!(task_id_match_key("cc-fdbc00ed"), Some("fdbc00ed"));
        assert_eq!(task_id_match_key("codex-019f209d"), Some("019f209d"));
        // 无 `-` 或 key 空 → None
        assert_eq!(task_id_match_key("weird"), None);
        assert_eq!(task_id_match_key("cursor-"), None);
    }

    #[test]
    fn parse_enumeration_multi_record() {
        let raw =
            "id1\u{001f}/a\u{001f}proj · hi · cursor-acd40fce-\u{001e}id2\u{001f}/b\u{001f}codex";
        let got = parse_ghostty_enumeration(raw);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "id1");
        assert_eq!(got[0].cwd, "/a");
        assert!(got[0].title.contains("cursor-acd40fce"));
        assert_eq!(got[1].id, "id2");
        assert_eq!(got[1].title, "codex");
    }

    #[test]
    fn parse_enumeration_drops_empty_id_records() {
        let raw = "\u{001f}/a\u{001f}title\u{001e}id2\u{001f}/b\u{001f}name";
        let got = parse_ghostty_enumeration(raw);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "id2");
    }

    #[test]
    fn pick_unique_cwd_is_the_common_case() {
        // 只有一个 poke-poke 窗口:直接命中(这正是"起完切走"后发起窗口仍在列表里的修复)
        let cands = vec![
            cand(
                "A",
                "/Users/edy/Documents/Hungry/solutionhub2",
                "solutionhub2 · x",
            ),
            cand(
                "B",
                "/Users/edy/Documents/Kidder/poke-poke",
                "poke-poke · hi",
            ),
        ];
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("acd40fce")
            ),
            GhosttyMatch::UniqueCwd(CapturedGhostty {
                id: "B".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
    }

    #[test]
    fn pick_title_key_disambiguates_two_same_cwd_windows() {
        // 你上轮卡住的场景:同 cwd 开两个 poke-poke 窗口,靠 title 里的 session-id 消歧。
        let cands = vec![
            cand(
                "51080B08",
                "/Users/edy/Documents/Kidder/poke-poke",
                "poke-poke · hi · cursor-acd40fce-",
            ),
            cand(
                "F6E68A11",
                "/Users/edy/Documents/Kidder/poke-poke",
                "poke-poke · hi · cursor-2167f9e9-",
            ),
        ];
        // 会话 2167f9e9 应精确挑到 F6E68A11,而不是第一个 51080B08
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("2167f9e9")
            ),
            GhosttyMatch::TitleKey(CapturedGhostty {
                id: "F6E68A11".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
        // 会话 acd40fce 应挑到 51080B08
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("acd40fce")
            ),
            GhosttyMatch::TitleKey(CapturedGhostty {
                id: "51080B08".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
    }

    #[test]
    fn pick_ambiguous_when_same_cwd_no_title_key() {
        // 同 cwd 两窗口,title 都被命令覆盖(session-id 消失)→ AmbiguousCwd(弱证据,
        // 分类仍返回第一个供诊断;caller 见此变体不落库,交给 reconcile)。
        let cands = vec![
            cand(
                "A",
                "/Users/edy/Documents/Kidder/poke-poke",
                "pnpm tauri dev",
            ),
            cand("B", "/Users/edy/Documents/Kidder/poke-poke", "codex"),
        ];
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("acd40fce")
            ),
            GhosttyMatch::AmbiguousCwd(CapturedGhostty {
                id: "A".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
    }

    #[test]
    fn pick_none_when_originating_window_absent() {
        // 发起窗口已关 / cwd 无兼容项 → None(caller 不落库,同旧 REJECT 语义)
        let cands = vec![cand("A", "/Users/edy/Documents/Hungry/solutionhub2", "x")];
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("acd40fce")
            ),
            GhosttyMatch::None
        );
    }

    #[test]
    fn pick_title_key_only_within_cwd_compatible_subset() {
        // title 含 key 但 cwd 是别的项目 → 不能命中(防跨项目 title 巧合);
        // 真正 cwd 兼容的那个(无 key)走 UniqueCwd。
        let cands = vec![
            cand(
                "WRONG",
                "/Users/edy/Documents/Hungry/solutionhub2",
                "x · acd40fce-",
            ),
            cand(
                "RIGHT",
                "/Users/edy/Documents/Kidder/poke-poke",
                "poke-poke · hi",
            ),
        ];
        assert_eq!(
            pick_ghostty_terminal(
                &cands,
                "/Users/edy/Documents/Kidder/poke-poke",
                Some("acd40fce")
            ),
            GhosttyMatch::UniqueCwd(CapturedGhostty {
                id: "RIGHT".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
    }

    #[test]
    fn pick_no_title_key_falls_to_cwd() {
        // task_id key=None(退化 unknown)→ 跳过 Pass 1,唯一 cwd 命中
        let cands = vec![cand(
            "B",
            "/Users/edy/Documents/Kidder/poke-poke",
            "poke-poke",
        )];
        assert_eq!(
            pick_ghostty_terminal(&cands, "/Users/edy/Documents/Kidder/poke-poke", None),
            GhosttyMatch::UniqueCwd(CapturedGhostty {
                id: "B".into(),
                cwd: "/Users/edy/Documents/Kidder/poke-poke".into(),
            })
        );
    }
}
