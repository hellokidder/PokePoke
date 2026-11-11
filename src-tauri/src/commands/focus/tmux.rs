//! tmux pane 级 best-effort 跳转(brief 20260804-tmux-binding)。
//! tmux 没有 GUI pane 级定位:short-circuit 只做 `tmux select-pane`(切 tmux
//! 内部活动 pane),命令失败静默;随后仍走外层终端 ladder(Ghostty/iTerm2 tab
//! 级兜底,见 grok review §2.6)。

use std::process::Command;

/// 组装 `tmux -S <socket> select-pane -t <pane_id>`;socket 缺失/为空时省略 `-S`
/// (防御边界,正常 binding 两字段全有)。
fn build_tmux_command(pane_id: &str, socket_path: Option<&str>) -> Command {
    let mut cmd = Command::new("tmux");
    if let Some(sock) = socket_path.filter(|s| !s.is_empty()) {
        cmd.arg("-S").arg(sock);
    }
    cmd.args(["select-pane", "-t", pane_id]);
    cmd
}

/// 尝试 `tmux -S <socket> select-pane -t <pane_id>`。best-effort:CLI 缺失 /
/// server 未跑 / 无 attached client 等任何失败都静默返回 false,不 panic。
pub(crate) fn focus_tmux_pane(pane_id: &str, socket_path: Option<&str>) -> bool {
    build_tmux_command(pane_id, socket_path)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(cmd: &Command) -> Vec<&str> {
        cmd.get_args().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn tmux_command_uses_socket_then_select_pane() {
        // 澄清后的契约:Command::new("tmux").args(["-S", socket, "select-pane", "-t", pane_id])
        let cmd = build_tmux_command("%12", Some("/tmp/tmux-501/default"));
        assert_eq!(
            args_of(&cmd),
            ["-S", "/tmp/tmux-501/default", "select-pane", "-t", "%12"]
        );
    }

    #[test]
    fn tmux_command_omits_socket_when_absent() {
        let cmd = build_tmux_command("%12", None);
        assert_eq!(args_of(&cmd), ["select-pane", "-t", "%12"]);
    }

    #[test]
    fn focus_tmux_pane_fails_silently_on_bogus_socket() {
        // 不存在的 socket:tmux 连接失败 → false,不 panic(命令失败静默)。
        assert!(!focus_tmux_pane(
            "%12",
            Some("/tmp/pokepoke-no-such-tmux-socket"),
        ));
    }
}
