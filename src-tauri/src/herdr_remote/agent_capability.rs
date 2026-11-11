//! herdr 侧「该 agent 是否有本地 hook 通路接管」判定。
//!
//! 从 `herdr_remote.rs` 抽出(AGENTS.md §4a:主文件已在阈值上限),行为不变。
//!
//! brief `20260813-herdr-triple-popup` Phase 2:本地 pane 跑的是自带 hook 的 agent 时,
//! 该 pane 的会话已由本地 hook 通路以更高保真度接管(title / message / tty / jump
//! 绑定都在 agent session 上),`poll_once` 再喂一次 pane 级任务就会出现「同一次完成
//! 弹两次」+ 面板重复条目。**Remote 连接不适用该跳过**:远程 agent 的 hook 打不到
//! 本机,pane 级监视是唯一通道(brief 20260804「本地走 hook / 远程走监视」分工)。

/// 自带 hook 通路的 agent 名(herdr `agent.list` 的 `agent` 字段小写归一后匹配)。
///
/// 八端 = pokehook 的八个 managed source:`cc` / `codex` / `cursor` / `opencode` /
/// `pi` / `omp` / `kimi` / `grok`。其中 claude 端有两种上报写法(`claude` /
/// `claude-code`,后者是 pokehook source 的写法),两者都认。
const HOOK_CAPABLE_AGENTS: &[&str] = &[
    "pi",
    "omp",
    "claude",
    "claude-code",
    "codex",
    "cursor",
    "opencode",
    "kimi",
    "grok",
];

/// 大小写不敏感;`None` / 未知 agent 一律 false(保守:不认识的按"没有 hook 通路"处理,
/// 仍走 pane 级监视,不会漏提醒)。
pub(super) fn is_hook_capable_agent(agent: Option<&str>) -> bool {
    agent
        .map(|a| a.to_ascii_lowercase())
        .is_some_and(|a| HOOK_CAPABLE_AGENTS.contains(&a.as_str()))
}

#[cfg(test)]
mod tests {
    use super::is_hook_capable_agent;

    #[test]
    fn matches_eight_sources_and_aliases() {
        // 八端 + claude/claude-code 别名;大小写不敏感。
        for name in [
            "pi",
            "omp",
            "claude",
            "claude-code",
            "codex",
            "cursor",
            "opencode",
            "kimi",
            "grok",
            "Pi",
            "Omp",
        ] {
            assert!(
                is_hook_capable_agent(Some(name)),
                "{name} 应判为自带 hook 的 agent"
            );
        }
    }

    #[test]
    fn false_for_unknown_or_missing() {
        for name in ["foo", "vim", "server", "tail", "ssh", ""] {
            assert!(
                !is_hook_capable_agent(Some(name)),
                "{name} 不应判为自带 hook 的 agent"
            );
        }
        assert!(!is_hook_capable_agent(None));
    }
}
