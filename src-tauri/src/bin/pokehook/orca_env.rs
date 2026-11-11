//! Orca 环境采集(从 pokehook.rs 抽出,见 AGENTS.md §4a:新终端适配用 mod 目录)。

/// Orca PTY 环境。以 `ORCA_PANE_KEY` 为 gate(`tabId:leafUuid` 稳定 pane 身份)。
///
/// 主宿主链(herdr/cmux/wezterm/otty)互斥;Orca 在 `add_host_agent_env` 里**叠加**上报,
/// 供 herdr-in-Orca 段 A 定位外层 pane,不与 herdr binding 冲突。
pub(crate) struct OrcaEnv {
    pub pane_key: String,
    pub worktree_id: String,
    pub tab_id: Option<String>,
}

fn collect_orca_env_from(mut getenv: impl FnMut(&str) -> Option<String>) -> Option<OrcaEnv> {
    let pane_key = getenv("ORCA_PANE_KEY").filter(|s| s.contains(':') && !s.is_empty())?;
    let worktree_id = getenv("ORCA_WORKTREE_ID")
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let tab_id = getenv("ORCA_TAB_ID").filter(|s| !s.is_empty());
    Some(OrcaEnv {
        pane_key,
        worktree_id,
        tab_id,
    })
}

pub(crate) fn collect_orca_env() -> Option<OrcaEnv> {
    collect_orca_env_from(|key| std::env::var(key).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_orca_env_requires_colon_in_pane_key() {
        let env = |key: &str| match key {
            "ORCA_PANE_KEY" => Some("tab-only".into()),
            "ORCA_WORKTREE_ID" => Some("wt-1".into()),
            _ => None,
        };
        assert!(collect_orca_env_from(env).is_none());
    }

    #[test]
    fn collect_orca_env_parses_pane_key_and_worktree() {
        let env = |key: &str| match key {
            "ORCA_PANE_KEY" => Some("tab-a:leaf-b".into()),
            "ORCA_WORKTREE_ID" => Some("repo::/path".into()),
            "ORCA_TAB_ID" => Some("tab-a".into()),
            _ => None,
        };
        let got = collect_orca_env_from(env).expect("should parse");
        assert_eq!(got.pane_key, "tab-a:leaf-b");
        assert_eq!(got.worktree_id, "repo::/path");
        assert_eq!(got.tab_id.as_deref(), Some("tab-a"));
    }
}
