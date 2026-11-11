//! tmux 环境采集(brief 20260804-tmux-binding,从 pokehook.rs 抽出,见 AGENTS.md
//! §4a:新终端适配用 mod 目录)。原生 tmux(非 herdr)里跑 agent 时,session tty
//! 是 tmux pane pty,与外层终端(Ghostty tab 等)tty 不一致 → 回跳降级。这里采集
//! `TMUX` + `TMUX_PANE` 两件套,上报 server 做 binding 记录 + focus best-effort。

/// tmux 绑定两件套。`TMUX` 值形如 `/tmp/tmux-501/default,20745,3`(socket 路径 +
/// server pid + session id),只取逗号前的 socket 路径;`TMUX_PANE` 形如 `%12`。
pub(crate) struct TmuxEnv {
    pub pane_id: String,
    pub socket_path: String,
}

/// 采集入口:`TMUX` / `TMUX_PANE` 任一缺失或为空 → None(两字段全有才算 tmux 场景)。
pub(crate) fn collect_tmux_env() -> Option<TmuxEnv> {
    collect_tmux_env_from(|key| std::env::var(key).ok())
}

fn collect_tmux_env_from(mut getenv: impl FnMut(&str) -> Option<String>) -> Option<TmuxEnv> {
    let tmux = getenv("TMUX").filter(|s| !s.is_empty())?;
    let pane_id = getenv("TMUX_PANE").filter(|s| !s.is_empty())?;
    // TMUX 值 = `<socket_path>,<server_pid>,<session_id>`;socket 是逗号前那段。
    let socket_path = tmux.split(',').next().filter(|s| !s.is_empty())?;
    Some(TmuxEnv {
        pane_id,
        socket_path: socket_path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(
        tmux: Option<&'static str>,
        pane: Option<&'static str>,
    ) -> impl FnMut(&str) -> Option<String> {
        move |key: &str| match key {
            "TMUX" => tmux.map(str::to_string),
            "TMUX_PANE" => pane.map(str::to_string),
            _ => None,
        }
    }

    #[test]
    fn collect_tmux_env_parses_pane_and_socket() {
        // 全有:TMUX 带 ,pid,session 后缀,socket 只取逗号前那段。
        let got =
            collect_tmux_env_from(env_with(Some("/tmp/tmux-501/default,20745,3"), Some("%12")))
                .expect("TMUX + TMUX_PANE 全有应采集成功");
        assert_eq!(got.pane_id, "%12");
        assert_eq!(got.socket_path, "/tmp/tmux-501/default");
    }

    #[test]
    fn collect_tmux_env_requires_tmux_pane() {
        // TMUX 有、TMUX_PANE 缺 → None(两字段缺一即不认 tmux 场景)。
        assert!(collect_tmux_env_from(env_with(Some("/tmp/tmux-501/default,1,0"), None)).is_none());
    }

    #[test]
    fn collect_tmux_env_requires_tmux() {
        // TMUX 缺、TMUX_PANE 有 → None。
        assert!(collect_tmux_env_from(env_with(None, Some("%12"))).is_none());
    }

    #[test]
    fn collect_tmux_env_rejects_empty_values() {
        assert!(collect_tmux_env_from(env_with(Some(""), Some("%12"))).is_none());
        assert!(collect_tmux_env_from(env_with(Some("/s,1,0"), Some(""))).is_none());
    }
}
