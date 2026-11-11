//! host-agent 环境三件套采集(herdr / cmux / wezterm / otty)。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

/// 采集 herdr 环境三件套。任一为空直接返回 None(全有才认为在 herdr pane 里)。
///
/// 三字段来源(herdr docs/agent-skill):
///   HERDR_PANE_ID       e.g. "w3:p1"
///   HERDR_WORKSPACE_ID  e.g. "w3"
///   HERDR_SOCKET_PATH   e.g. "/Users/me/.config/herdr/herdr.sock"
///
/// 上报到 server 后,server 落到 sessions.json 的 herdr_* 字段;跳回时用作段 B
/// (herdr 内部 agent.focus)的调度依据。
pub(crate) fn collect_herdr_env() -> Option<(String, String, String)> {
    let pane = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    let ws = std::env::var("HERDR_WORKSPACE_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    let sock = std::env::var("HERDR_SOCKET_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        // 兜底:HERDR_SOCKET_PATH 未设置但 HERDR_ENV=1 时可能靠默认位置,不做默认
        // 补 —— 明确 3 件套齐全才算 herdr 场景。
        ;
    Some((pane, ws, sock?))
}

/// 采集 cmux 环境两件套。两个都为空直接返回 None(全有才认为在 cmux pane 里)。
///
/// 两字段来源(cmux 官方注入,pane shell 启动就设置):
///   CMUX_SURFACE_ID     e.g. "FABEB780-8D56-4548-8D92-1E4CB60B7E68" (UUID)
///   CMUX_WORKSPACE_ID   e.g. "4C78AC55-7559-49CF-8C75-784F722C1491" (UUID)
///
/// surface_id 是 pane 生命周期恒定的 UUID,可以直接喂给 `cmux focus-panel --panel`;
/// workspace_id 只做诊断记录,不参与 focus 决策。
///
/// 上报到 server 后落到 sessions.json 的 cmux_* 字段;跳回时直接走 short-circuit
/// `cmux focus-panel --panel <surface_id>`。
pub(crate) fn collect_cmux_env() -> Option<(String, String)> {
    let surface = std::env::var("CMUX_SURFACE_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    let ws = std::env::var("CMUX_WORKSPACE_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some((surface, ws))
}

/// 采集 WezTerm 环境。以 `WEZTERM_PANE` + `WEZTERM_UNIX_SOCKET` 为 gate(不依赖
/// `TERM_PROGRAM`,CC daemon resume 时后者常丢)。`WEZTERM_EXECUTABLE` 可选,供 server
/// 侧 CLI 在 PATH 找不到 wezterm 时使用。
///
/// 优先级低于 herdr / cmux:agent 跑在 cmux 或 herdr 内层 pty 时不采集。
pub(crate) fn collect_wezterm_env() -> Option<(String, String, Option<String>)> {
    let pane = std::env::var("WEZTERM_PANE")
        .ok()
        .filter(|s| !s.is_empty())?;
    let sock = std::env::var("WEZTERM_UNIX_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())?;
    let executable = std::env::var("WEZTERM_EXECUTABLE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.ends_with("wezterm-gui") {
                s.replace("wezterm-gui", "wezterm")
            } else {
                s
            }
        });
    Some((pane, sock, executable))
}

pub(crate) fn collect_otty_env() -> Option<String> {
    std::env::var("OTTY_PANE_ID").ok().filter(|s| !s.is_empty())
}
