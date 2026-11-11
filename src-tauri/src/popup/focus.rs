//! popup 焦点探测:iTerm2 / Terminal / Ghostty / cmux / herdr / WezTerm / Cursor / Warp。
//!
//! 从 `popup.rs` 抽出:`is_surface_focused` 统一分派 + 各端判定 +
//! `build_focus_check_script` osascript 生成。纯只读探测,行为零变化;
//! `popup.rs` 重导出 `is_surface_focused` / `is_terminal_session_focused` /
//! `herdr_outer_tty_for_focus` 保持外部 + mod.rs 内部引用不断。

use std::time::Duration;

use crate::surface::SurfaceEvidence;

/// Narrow check: is the user actively viewing this exact terminal session?
/// 需要**同时**满足：
///   1. macOS 当前最前台 app 是 iTerm2 / Terminal
///   2. 该 app 的 current/front window 自身是 frontmost（防止 iTerm2 内
///      "current window" 的 most-recent 语义误判）
///   3. 该 window 当前激活的 tab 的 tty 等于目标 tty
///
/// 见 `docs/briefs/20260421-popup-focus-misjudge-iterm2.md`。
pub fn is_terminal_session_focused(tty: Option<&str>, terminal_id: Option<&str>) -> bool {
    let script = match build_focus_check_script(tty, terminal_id) {
        Some(s) => s,
        None => return false,
    };
    // 带超时:focus-watcher 每 500ms 调一次,终端 App 卡住时绝不能让轮询线程被
    // 无限占住(见 osascript 模块,2026-07-01 主线程挂死事故)。
    crate::osascript::run_bool(&script, "popup", "focus_check")
}

/// 统一聚焦判定:cmux / herdr / Cursor IDE + 既有 iTerm2/Terminal/Ghostty。
///
/// W7(brief 20260813-focus-dispatch-table):绑定/宿主检查链收敛为有序静态表
/// [`FOCUS_CHECKS`] 驱动 —— 每行 (名字, 检查),检查返回 `Some(hit)` = 该适配器
/// 负责且已判定;`None` = 不负责 → 继续下一行。新宿主接入 = 新 check_* +
/// 表里一行,不改本函数本体。
pub fn is_surface_focused(evidence: &SurfaceEvidence) -> bool {
    let front_app = crate::osascript::frontmost_app_name();

    for (_name, check) in FOCUS_CHECKS {
        if let Some(hit) = check(evidence, front_app.as_deref()) {
            return hit;
        }
    }
    // 不可达:terminal-tty 兜底行永远返回 Some;防御性兜底。
    false
}

/// 聚焦检查适配器签名:返回 `Some(hit)` = 该适配器负责且已判定;
/// `None` = 不负责 → 继续下一行。
type FocusCheck = fn(&SurfaceEvidence, Option<&str>) -> Option<bool>;

/// 有序聚焦检查表。顺序 = W6 前链的逐条顺序(herdr → wezterm → orca → otty →
/// warp → cmux → cursor → terminal-tty 兜底),逐字符保持;新宿主接入 =
/// 新适配器文件 + 表里一行。
const FOCUS_CHECKS: &[(&str, FocusCheck)] = &[
    ("herdr", check_herdr),
    ("wezterm", check_wezterm),
    ("orca", check_orca),
    ("otty", check_otty),
    ("warp", check_warp),
    ("cmux", check_cmux),
    ("cursor", check_cursor),
    // DSH:GUI/Web 宿主,无 tty;socket 现查渲染进程 showing/focused/visible。
    ("dsh", check_dsh),
    ("terminal-tty", check_terminal_tty),
];

/// herdr:绑定在场即负责(内部判 herdr 聚焦,含 herdr-remote 永不判聚焦)。
fn check_herdr(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if has_herdr_binding(evidence) {
        return Some(is_herdr_bound_session_focused(evidence, front_app));
    }
    None
}

/// wezterm:绑定在场即负责(前台是 WezTerm 才判 pane 聚焦,否则不聚焦)。
fn check_wezterm(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if has_wezterm_binding(evidence) {
        if is_wezterm_frontmost(front_app) {
            return Some(is_wezterm_bound_session_focused(evidence));
        }
        return Some(false);
    }
    None
}

/// orca:绑定在场即负责(前台是 Orca 才判 pane 聚焦,否则不聚焦)。
fn check_orca(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if has_orca_binding(evidence) {
        if is_orca_frontmost(front_app) {
            return Some(is_orca_bound_session_focused(evidence));
        }
        return Some(false);
    }
    None
}

/// otty:绑定在场即负责(前台是 Otty 才判 pane 聚焦,否则不聚焦)。
fn check_otty(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if has_otty_binding(evidence) {
        if is_otty_frontmost(front_app) {
            return Some(is_otty_bound_session_focused(evidence));
        }
        return Some(false);
    }
    None
}

/// warp:宿主 app 判定(brief 20260810-warp-terminal-support)——Warp 无
/// AppleScript 可枚举,与回跳 ladder 对齐走 **app 级**判定:前台是 Warp 家族
/// 即判已聚焦。局限:用户在 Warp tab A 而 agent 在 tab B 时会误抑制;tab 级
/// 判定需 local_control rollout 后按 brief Notes 加回(title-key 增强层已下掉)。
fn check_warp(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if evidence.terminal_app.as_deref() == Some("warpterminal") {
        let hit = is_warp_session_focused(front_app);
        crate::log_info!(
            "popup",
            "warp_focus_check app_level front_app={:?} focused={}",
            front_app,
            hit,
        );
        return Some(hit);
    }
    None
}

/// cmux:前台是 cmux 即负责(pane 级聚焦判定,cmux 无 AppleScript 枚举)。
fn check_cmux(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if front_app == Some("cmux") {
        return Some(crate::cmux::is_cmux_terminal_focused(
            evidence.terminal_tty.as_deref(),
            evidence.cmux_surface_id.as_deref(),
        ));
    }
    None
}

/// cursor:source=cursor + 前台 Cursor 家族 + 无 cmux binding + workspace 命中。
/// 不满足条件时不负责(继续下一行,落 terminal 兜底)。
fn check_cursor(evidence: &SurfaceEvidence, front_app: Option<&str>) -> Option<bool> {
    if evidence.source.as_deref() == Some("cursor")
        && is_cursor_frontmost(front_app)
        && !is_present_opt(&evidence.cmux_surface_id)
        && is_cursor_workspace_focused(evidence.workspace_path.as_deref(), front_app)
    {
        return Some(true);
    }
    None
}

/// terminal-tty 兜底:iTerm2 / Terminal.app / Ghostty 的 tty/terminal_id 判定。
/// 永远负责(原链末行无条件执行)。
/// DSH:只对 source=dsh 负责。没有 socket(插件未安装/DSH 未启动)或没有
/// session id 时返回 None,交回 terminal 兜底(→ false,不抑制,安全降级)。
fn check_dsh(evidence: &SurfaceEvidence, _front_app: Option<&str>) -> Option<bool> {
    if evidence.source.as_deref() != Some("dsh") {
        return None;
    }
    let socket = evidence
        .dsh_nav_socket
        .as_deref()
        .filter(|s| !s.is_empty())?;
    let session_id = evidence.external_session_id.as_deref()?;
    Some(crate::commands::focus::dsh_session_is_focused(
        socket, session_id,
    ))
}

fn check_terminal_tty(evidence: &SurfaceEvidence, _front_app: Option<&str>) -> Option<bool> {
    Some(is_terminal_session_focused(
        evidence.terminal_tty.as_deref(),
        evidence.terminal_id.as_deref(),
    ))
}

fn is_present_opt(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|v| !v.is_empty())
}

/// Warp 会话聚焦判定(纯函数,供单测):terminal_app 归一化名为 `warpterminal`
/// 且前台是 Warp 家族(与 `focus_warp_session` 的 frontmost 自查同一名单)。
fn is_warp_session_focused(front_app: Option<&str>) -> bool {
    crate::commands::focus::frontmost_is_warp(front_app)
}

/// 前台是否属于 Cursor 家族(正式版 "Cursor" / "Cursor Insiders" / "Cursor Nightly")。
/// 用前缀匹配而非精确相等,避免 Insiders/Nightly 用户走不进抑制分支(Bug 4)。
fn is_cursor_frontmost(front_app: Option<&str>) -> bool {
    front_app.is_some_and(|name| name == "Cursor" || name.starts_with("Cursor "))
}

fn has_herdr_binding(evidence: &SurfaceEvidence) -> bool {
    is_present_opt(&evidence.herdr_pane_id) && is_present_opt(&evidence.herdr_socket_path)
}

fn is_wezterm_frontmost(front_app: Option<&str>) -> bool {
    front_app.is_some_and(|name| name == "WezTerm" || name.eq_ignore_ascii_case("wezterm-gui"))
}

fn has_wezterm_binding(evidence: &SurfaceEvidence) -> bool {
    is_present_opt(&evidence.wezterm_pane_id) && is_present_opt(&evidence.wezterm_unix_socket)
}

fn is_wezterm_bound_session_focused(evidence: &SurfaceEvidence) -> bool {
    let (Some(pane), Some(sock)) = (
        evidence.wezterm_pane_id.as_deref(),
        evidence.wezterm_unix_socket.as_deref(),
    ) else {
        return false;
    };
    crate::wezterm::is_pane_focused(
        pane,
        sock,
        evidence.wezterm_executable.as_deref(),
        evidence.workspace_path.as_deref(),
    )
}

/// herdr 外层宿主聚焦用哪条 tty:优先 UI client(`lsof herdr-client.log`),
/// 不用 hook 报的 agent 内层 pty(常见 ttys005 vs client ttys000 错位)。
fn herdr_outer_tty_for_focus(hook_tty: Option<&str>, client_tty: Option<&str>) -> Option<String> {
    client_tty
        .filter(|t| !t.is_empty())
        .map(|t| {
            if t.starts_with("/dev/") {
                t.to_string()
            } else {
                format!("/dev/{}", t)
            }
        })
        .or_else(|| hook_tty.filter(|t| !t.is_empty()).map(str::to_string))
}

/// Brief 20260717 §Goal-2: 外层宿主已聚焦 **且** `pane.current` 命中才抑制。
#[inline]
fn herdr_focus_suppression_hit(outer_host_focused: bool, pane_current_matches: bool) -> bool {
    outer_host_focused && pane_current_matches
}

fn is_herdr_bound_session_focused(evidence: &SurfaceEvidence, front_app: Option<&str>) -> bool {
    // herdr-remote(brief 20260807-herdr-remote-jumpback-fix):agent 渲染在远程机,
    // 本地 outer-host + pane.current 判定没有意义 —— 远程 socket 在本地解析不出
    // client tty,会退化成用本地 Ghostty herdr tab 判 outer,把远程 blocked popup
    // 误判已聚焦而吞掉。与 2026-08-07 产品决策「远程聚焦 ≠ 人在看」对齐:永不判聚焦
    // (不抑制弹窗、focus-watcher 不关)。
    if crate::surface::is_herdr_remote_source(evidence.source.as_deref()) {
        return false;
    }
    let Some(sock) = evidence
        .herdr_socket_path
        .as_deref()
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let Some(pane) = evidence.herdr_pane_id.as_deref().filter(|s| !s.is_empty()) else {
        return false;
    };
    let sock_path = std::path::Path::new(sock);
    // 先 outer 再 pane:herdr-in-Ghostty 时 frontApp 常为 cmux,单靠 pane.current
    // 会在 Chrome/Finder 前台误抑制(brief §Risks)。
    let (outer, client_tty) = herdr_outer_host_focused(evidence, sock_path, front_app);
    let pane_hit = crate::herdr::is_herdr_pane_focused(sock_path, pane);
    let suppress = herdr_focus_suppression_hit(outer, pane_hit);
    crate::log_info!(
        "popup",
        "herdr_focus_check pane={} outer={} pane_hit={} suppress={} front_app={:?} client_tty={:?} hook_tty={:?}",
        pane,
        outer,
        pane_hit,
        suppress,
        front_app,
        client_tty,
        evidence.terminal_tty,
    );
    suppress
}

/// 与 [`crate::commands::focus::pick_ghostty_herdr_terminal`] 保持同步(legacy 无 socket 路径)。
fn ghostty_herdr_anchor_query_script() -> &'static str {
    r#"tell application "Ghostty"
    try
        set ids to ""
        set sep to ASCII character 30
        repeat with aWindow in windows
            repeat with aTab in tabs of aWindow
                repeat with aTerminal in terminals of aTab
                    if (name of aTerminal as text) starts with "herdr" then
                        if ids is "" then
                            set ids to (id of aTerminal as text)
                        else
                            set ids to ids & sep & (id of aTerminal as text)
                        end if
                    end if
                end repeat
            end repeat
        end repeat
        return ids
    on error
        return ""
    end try
end tell"#
}

/// Ghostty 前台且唯一 herdr tab 为当前 focused terminal。
fn is_ghostty_herdr_tab_focused() -> bool {
    let raw = crate::osascript::run(
        ghostty_herdr_anchor_query_script(),
        Duration::from_secs(2),
        "popup",
        "ghostty_herdr_query",
    )
    .unwrap_or_default();
    let ids: Vec<&str> = raw.split('\u{001e}').filter(|s| !s.is_empty()).collect();
    if ids.len() != 1 {
        return false;
    }
    let escaped = ids[0].replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"tell application "System Events"
    set frontApp to name of first application process whose frontmost is true
end tell
if frontApp is not "Ghostty" then return false
using terms from application id "com.mitchellh.ghostty"
    tell application id "com.mitchellh.ghostty"
        try
            return (id of focused terminal of selected tab of front window) is "{escaped}"
        end try
    end tell
end using terms from
return false"#
    );
    crate::osascript::run_bool(&script, "popup", "ghostty_herdr_focused")
}

/// 只读版 `focus_herdr_client_host_by_tty`:按 herdr UI client tty 探测外层宿主聚焦。
/// 必须带 `front_app` gate -- cmux tree 的 active tty 在 cmux 非前台时也会命中,
/// 会把 herdr-in-WezTerm 误判成 already_focused(仅有声无 popup)。
fn is_herdr_client_host_focused_by_tty(
    client_tty: &str,
    herdr_socket: &std::path::Path,
    front_app: Option<&str>,
    evidence: &SurfaceEvidence,
) -> bool {
    let bare = client_tty
        .trim()
        .strip_prefix("/dev/")
        .unwrap_or(client_tty.trim());
    let tty_dev = if client_tty.contains("/dev/") {
        client_tty.to_string()
    } else {
        format!("/dev/{bare}")
    };
    if is_terminal_session_focused(Some(&tty_dev), None) {
        return true;
    }
    if is_wezterm_frontmost(front_app) {
        return crate::wezterm::is_tty_focused(
            &tty_dev,
            evidence.wezterm_unix_socket.as_deref(),
            evidence.wezterm_executable.as_deref(),
        );
    }
    if front_app == Some("cmux") {
        return crate::cmux::is_cmux_terminal_focused(Some(&tty_dev), None);
    }
    if crate::commands::focus::client_tty_belongs_to_ghostty(client_tty)
        && crate::commands::focus::is_ghostty_herdr_tab_focused_for_socket(herdr_socket)
    {
        return true;
    }
    // Otty:herdr 跑在 Otty pane 里时,前窗口 selected tab 的 tty == herdr client tty
    // 即用户正看着承载 herdr 的 tab -> 抑制。front_app 已是 Otty 才查(对齐
    // wezterm/cmux 分支的 frontmost gate)。
    if is_otty_frontmost(front_app) && crate::commands::focus::otty_selected_tab_has_tty(&tty_dev) {
        return true;
    }
    if is_orca_frontmost(front_app)
        && crate::commands::focus::orca_herdr_host_focused(evidence.workspace_path.as_deref())
    {
        return true;
    }
    false
}

/// 镜像 `focus_herdr_host_anchor` 只读逻辑;禁止按 macOS frontApp 单点分支
/// (herdr-in-Ghostty 时 front 常为 cmux,client_tty 不在 cmux tree)。
///
/// 返回 `(focused, client_tty)`:第二个字段用来喂 caller 的日志,让 caller 省掉一次
/// `client_tty_for_socket` 调用(每次要 1 lsof + 2 ps),见 Bug 3。
fn herdr_outer_host_focused(
    evidence: &SurfaceEvidence,
    sock: &std::path::Path,
    front_app: Option<&str>,
) -> (bool, Option<String>) {
    if let Some(client_tty) = crate::commands::focus::resolve_herdr_client_tty(
        sock,
        evidence.wezterm_unix_socket.as_deref(),
        evidence.wezterm_executable.as_deref(),
        evidence.orca_pane_key.as_deref(),
        evidence.orca_worktree_id.as_deref(),
    ) {
        if is_herdr_client_host_focused_by_tty(&client_tty, sock, front_app, evidence) {
            return (true, Some(client_tty));
        }
        // 对齐 focus jump 的 Ghostty activate 兜底:精确 tab/tty 探针瞬时 miss 时,
        // Ghostty 已是 macOS 前台仍视为 outer 命中(内层 pane.current 继续校验)。
        if front_app == Some("Ghostty")
            && crate::commands::focus::client_tty_belongs_to_ghostty(&client_tty)
        {
            return (true, Some(client_tty));
        }
        if is_wezterm_frontmost(front_app)
            && crate::commands::focus::client_tty_belongs_to_wezterm(&client_tty)
        {
            return (true, Some(client_tty));
        }
        return (false, Some(client_tty));
    }

    if crate::commands::focus::is_ghostty_herdr_tab_focused_for_socket(sock) {
        return (true, None);
    }

    if is_ghostty_herdr_tab_focused() {
        return (true, None);
    }

    if front_app == Some("cmux") {
        let hit = crate::cmux::is_cmux_herdr_client_anchor_focused()
            || crate::cmux::is_cmux_terminal_focused(
                evidence.terminal_tty.as_deref(),
                evidence.cmux_surface_id.as_deref(),
            );
        return (hit, None);
    }

    if is_cursor_frontmost(front_app)
        && evidence.source.as_deref() == Some("cursor")
        && !is_present_opt(&evidence.cmux_surface_id)
    {
        return (
            is_cursor_workspace_focused(evidence.workspace_path.as_deref(), front_app),
            None,
        );
    }

    let outer_tty = herdr_outer_tty_for_focus(evidence.terminal_tty.as_deref(), None);
    (
        is_terminal_session_focused(outer_tty.as_deref(), evidence.terminal_id.as_deref()),
        None,
    )
}

/// otty binding 存在判定:只需 `otty_pane_id`。聚焦抑制是否命中由
/// `is_otty_bound_session_focused` 比对 selected tab 决定,不依赖 tab_id
/// (capture 补全 tab_id 前也能正确抑制;tab_id 缺失时现场 `pane show` 反查)。
fn has_otty_binding(evidence: &SurfaceEvidence) -> bool {
    is_present_opt(&evidence.otty_pane_id)
}

fn is_otty_frontmost(front_app: Option<&str>) -> bool {
    front_app.is_some_and(|name| name == "Otty")
}

fn is_otty_bound_session_focused(evidence: &SurfaceEvidence) -> bool {
    let Some(pane) = evidence.otty_pane_id.as_deref() else {
        return false;
    };
    crate::commands::focus::otty_bound_pane_focused(
        pane,
        evidence.otty_tab_id.as_deref(),
        evidence.otty_window_id.as_deref(),
    )
}

fn has_orca_binding(evidence: &SurfaceEvidence) -> bool {
    is_present_opt(&evidence.orca_pane_key)
}

fn is_orca_frontmost(front_app: Option<&str>) -> bool {
    front_app == Some("Orca")
}

fn is_orca_bound_session_focused(evidence: &SurfaceEvidence) -> bool {
    let Some(pane_key) = evidence.orca_pane_key.as_deref() else {
        return false;
    };
    let focused = crate::commands::focus::orca_bound_pane_focused(
        pane_key,
        evidence.orca_worktree_id.as_deref(),
        evidence.orca_tab_id.as_deref(),
    );
    // 2026-08-07 教训:orca 聚焦判定失败是静默的(日志只有普通的
    // already_focused=false),排障只能靠翻 CLI help。对齐 herdr 的
    // focus_check 日志:前台命中 + worktree/layout 哪一步挂了,一眼可见。
    crate::log_info!(
        "popup",
        "orca_focus_check pane={} worktree_present={} tab_hint_present={} focused={}",
        pane_key,
        evidence
            .orca_worktree_id
            .as_deref()
            .is_some_and(|w| !w.is_empty()),
        evidence
            .orca_tab_id
            .as_deref()
            .is_some_and(|t| !t.is_empty()),
        focused,
    );
    focused
}

fn is_cursor_workspace_focused(workspace_path: Option<&str>, front_app: Option<&str>) -> bool {
    let Some(path) = workspace_path.filter(|p| !p.is_empty()) else {
        return false;
    };
    let Some(basename) = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    // 只允许 Cursor 家族的前台名(如 "Cursor" / "Cursor Insiders" / "Cursor Nightly")
    // 进入 tell process,防注入。任何非 Cursor 家族一律回退默认 "Cursor",宁可探测失败
    // 也不放任意字符串进 AppleScript。
    let cursor_process = front_app
        .filter(|name| *name == "Cursor" || name.starts_with("Cursor "))
        .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '))
        .unwrap_or("Cursor");
    let escaped = basename.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"tell application "System Events"
    set frontName to name of first application process whose frontmost is true
    if frontName is not "{cursor_process}" then return false
    tell process "{cursor_process}"
        repeat with w in windows
            try
                if (name of w as text) contains "{escaped}" then return true
            end try
        end repeat
    end tell
end tell
return false"#,
    );
    crate::osascript::run_bool(&script, "popup", "cursor_workspace_focus")
}

/// 生成 osascript 文本。抽出 pure fn 仅为便于单测验证 script 结构
/// (单测不能跑 osascript,但能验证生成的 script 包含双重验证 + 对应 ID 匹配)。
///
/// 0.0.5+ Ghostty 支持:Ghostty 没有 tty,改按 terminal_id 匹配 focused terminal。
/// tty 和 terminal_id 至少要给一个,否则返回 None(调用方退化为 false)。
fn build_focus_check_script(tty: Option<&str>, terminal_id: Option<&str>) -> Option<String> {
    let tty = tty.filter(|t| !t.is_empty());
    let terminal_id = terminal_id.filter(|t| !t.is_empty());
    if tty.is_none() && terminal_id.is_none() {
        return None;
    }

    // 各端分支按 frontApp 名称 dispatch,缺失对应 ID 的端跳过(返回 false)。
    //
    // 2026-07-14(focus_check syntax error 根因):`tell application "iTerm2"` 块里
    // 用了 iTerm2 专属术语(tty of current session / focused terminal),当该 app
    // **没运行**时,osascript 无法加载其脚本字典 -> 编译期 syntax error
    // (`预期是","，却找到标识符`)。frontApp 永远不会等于一个没运行的 app,所以
    // 该分支运行期不会执行,但 AppleScript 是整脚本编译,无法惰性跳过。
    // 修法:用 `using terms from application id "<bundle>"` + `tell application id
    // "<bundle>"` -- `using terms from` 从**已安装** app 的 sdef 文件加载术语,不需
    // app 运行,编译期就能解析 tty/focused terminal 等术语。bundle id 形式比名字
    // 形式更可靠(iTerm2 用名字 "iTerm2" 时 using terms from 仍解析失败,bundle id
    // "com.googlecode.iterm2" 才行 -- 实测)。
    let iterm2_branch = match tty {
        Some(t) => format!(
            r#"if frontApp is "iTerm2" then
    using terms from application id "com.googlecode.iterm2"
        tell application id "com.googlecode.iterm2"
            try
                if frontmost of current window is false then return false
                return (tty of current session of current tab of current window) is "{t}"
            end try
        end tell
    end using terms from"#,
            t = t,
        ),
        None => r#"if frontApp is "iTerm2" then
    return false"#
            .to_string(),
    };
    let terminal_branch = match tty {
        Some(t) => format!(
            r#"else if frontApp is "Terminal" then
    using terms from application "Terminal"
        tell application "Terminal"
            try
                if frontmost of front window is false then return false
                return (tty of selected tab of front window) is "{t}"
            end try
        end tell
    end using terms from"#,
            t = t,
        ),
        None => r#"else if frontApp is "Terminal" then
    return false"#
            .to_string(),
    };
    // Ghostty 的 `focused terminal` 在 `selected tab` 上(不在 window),0.0.5
    // ghostty 跳转那波踩过这个坑(commands.rs:capture_ghostty_focused_terminal_id)。
    // NOTE: Ghostty AppleScript 不支持 `frontmost of front window` 属性（抛错），
    // 且我们已通过 System Events 确认 frontApp is "Ghostty"（进程级 frontmost），
    // `front window` 在 Ghostty 里不存在 iTerm2 那种 most-recently-used 语义歧义，
    // 所以这里跳过窗口级 frontmost 检查。见 `focused terminal` 在 `selected tab` 上。
    //
    // 2026-07-14:同 iTerm2/Ghostty 没跑时的 syntax error 问题,用 using terms from
    // + bundle id "com.mitchellh.ghostty" 包裹。
    let ghostty_branch = match terminal_id {
        Some(id) => format!(
            r#"else if frontApp is "Ghostty" then
    using terms from application id "com.mitchellh.ghostty"
        tell application id "com.mitchellh.ghostty"
            try
                return (id of focused terminal of selected tab of front window) is "{id}"
            end try
        end tell
    end using terms from"#,
            id = id,
        ),
        None => r#"else if frontApp is "Ghostty" then
    return false"#
            .to_string(),
    };

    Some(format!(
        r#"tell application "System Events"
    set frontApp to name of first application process whose frontmost is true
end tell

{iterm2_branch}
{terminal_branch}
{ghostty_branch}
end if
return false"#,
        iterm2_branch = iterm2_branch,
        terminal_branch = terminal_branch,
        ghostty_branch = ghostty_branch,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        build_focus_check_script, herdr_focus_suppression_hit, herdr_outer_tty_for_focus,
        is_cursor_frontmost, is_herdr_bound_session_focused, is_warp_session_focused,
    };
    use crate::surface::SurfaceEvidence;

    /// brief 20260807-herdr-remote-jumpback-fix:herdr-remote session 永不判聚焦 ——
    /// 远程聚焦 ≠ 本地人在看,且本地 Ghostty herdr tab 匹配会误吞远程 blocked popup。
    /// 门控在函数开头,不做任何 IO,socket 不存在也能验证。
    #[test]
    fn herdr_remote_session_never_counts_as_focused() {
        let evidence = SurfaceEvidence {
            source: Some("herdr-remote".into()),
            terminal_app: None,
            dsh_nav_socket: None,
            task_id: "hr-hello-w1:p1".into(),
            session_id: "sess-x".into(),
            external_session_id: None,
            workspace_path: Some("/Users/remote".into()),
            terminal_tty: None,
            terminal_id: None,
            terminal_id_captured_cwd: None,
            herdr_pane_id: Some("w1:p1".into()),
            herdr_socket_path: Some("/tmp/definitely-not-a-socket.sock".into()),
            cmux_surface_id: None,
            wezterm_pane_id: None,
            wezterm_unix_socket: None,
            wezterm_executable: None,
            otty_pane_id: None,
            otty_tab_id: None,
            otty_window_id: None,
            tmux_pane_id: None,
            tmux_socket_path: None,
            orca_pane_key: None,
            orca_worktree_id: None,
            orca_tab_id: None,
        };
        assert!(
            !is_herdr_bound_session_focused(&evidence, Some("Ghostty")),
            "herdr-remote 永不判聚焦(本地 Ghostty 前台也不抑制)"
        );
    }

    /// 双重验证守门:iTerm2 / Terminal 两个分支都必须先校验
    /// frontmost-of-window 才返回 tty 匹配。任何分支漏掉这条都会让本测 fail。
    #[test]
    fn focus_check_script_requires_window_to_be_frontmost() {
        let script = build_focus_check_script(Some("/dev/ttys999"), None).unwrap();
        assert!(
            script.contains("frontmost of current window is false"),
            "iTerm2 分支必须有 current window 的 frontmost 检查",
        );
        assert!(
            script.contains("frontmost of front window is false"),
            "Terminal 分支必须有 front window 的 frontmost 检查",
        );
    }

    #[test]
    fn focus_check_script_compares_against_target_tty() {
        let script = build_focus_check_script(Some("/dev/ttys123"), None).unwrap();
        assert!(
            script.matches("/dev/ttys123").count() >= 2,
            "目标 tty 必须出现在 iTerm2 + Terminal 两个分支里",
        );
    }

    /// 2026-07-14:focus_check syntax error 根因。app 专属术语(tty of current
    /// session / focused terminal)在 app 没运行时编译期解析失败 -> syntax error。
    /// 必须用 `using terms from application id "<bundle>"` 包裹,从已安装 app
    /// 的 sdef 加载术语,不依赖 app 运行。断言三个 branch 都包了,防回归。
    #[test]
    fn focus_check_script_wraps_branches_in_using_terms_from_bundle_id() {
        // iTerm2 + Terminal(Some tty)分支
        let script = build_focus_check_script(Some("/dev/ttys123"), None).unwrap();
        assert!(
            script.contains(r#"using terms from application id "com.googlecode.iterm2""#),
            "iTerm2 分支必须用 using terms from + bundle id (app 没跑时编译失败): {script}"
        );
        assert!(
            script.contains(r#"tell application id "com.googlecode.iterm2""#),
            "iTerm2 分支必须用 bundle id 形式 tell: {script}"
        );
        assert!(
            script.contains(r#"using terms from application "Terminal""#),
            "Terminal 分支必须用 using terms from: {script}"
        );
        // Ghostty(Some terminal_id)分支
        let script_g =
            build_focus_check_script(None, Some("0123456789abcdef0123456789abcdef")).unwrap();
        assert!(
            script_g.contains(r#"using terms from application id "com.mitchellh.ghostty""#),
            "Ghostty 分支必须用 using terms from + bundle id: {script_g}"
        );
        assert!(
            script_g.contains(r#"tell application id "com.mitchellh.ghostty""#),
            "Ghostty 分支必须用 bundle id 形式 tell: {script_g}"
        );
    }

    #[test]
    fn focus_check_script_falls_back_to_false_for_other_apps() {
        let script = build_focus_check_script(Some("/dev/ttys001"), None).unwrap();
        assert!(
            script.trim_end().ends_with("return false"),
            "frontApp 既不是 iTerm2 / Terminal / Ghostty 时必须 return false",
        );
    }

    /// 0.0.5+:Ghostty 分支(纯 terminal_id,无 tty)
    #[test]
    fn focus_check_script_includes_ghostty_when_terminal_id_given() {
        let script =
            build_focus_check_script(None, Some("0123456789abcdef0123456789abcdef")).unwrap();
        assert!(
            script.contains(r#"frontApp is "Ghostty""#),
            "Ghostty 分支必须存在",
        );
        assert!(
            script.contains("id of focused terminal of selected tab of front window"),
            "Ghostty 分支必须走 selected tab 路径(0.0.5 capture 那波踩过的坑)",
        );
        assert!(
            script.contains("0123456789abcdef0123456789abcdef"),
            "目标 terminal_id 必须出现在 Ghostty 分支里",
        );
    }

    /// tty + terminal_id 同时给:三端分支都全 - iTerm2/Terminal 走 tty,
    /// Ghostty 走 terminal_id(典型情况:用户从 iTerm2 切到 Ghostty 跑同
    /// session 不存在,但 capture id 之后再 reattach 后端到 iTerm2 可能造成
    /// session 同时有 tty + terminal_id)
    #[test]
    fn focus_check_script_both_ids_emits_all_three_branches() {
        let script = build_focus_check_script(Some("/dev/ttys005"), Some("ghost-id-x")).unwrap();
        assert!(script.contains("/dev/ttys005"), "tty 走 iTerm2/Terminal");
        assert!(script.contains("ghost-id-x"), "terminal_id 走 Ghostty");
    }

    /// 两个 ID 都缺 -> 返回 None(调用方退化为 false 不抑制 popup)
    #[test]
    fn focus_check_script_returns_none_when_no_ids() {
        assert!(build_focus_check_script(None, None).is_none());
        assert!(build_focus_check_script(Some(""), Some("")).is_none());
    }

    #[test]
    fn herdr_outer_tty_prefers_client_tty_over_hook_agent_tty() {
        assert_eq!(
            herdr_outer_tty_for_focus(Some("/dev/ttys005"), Some("ttys000")).as_deref(),
            Some("/dev/ttys000")
        );
        assert_eq!(
            herdr_outer_tty_for_focus(Some("/dev/ttys005"), None).as_deref(),
            Some("/dev/ttys005")
        );
    }

    /// Brief 20260717: herdr 抑制须 outer **且** pane 同时命中。
    #[test]
    fn herdr_focus_suppression_matrix() {
        assert!(
            !herdr_focus_suppression_hit(false, true),
            "Chrome 前台 + stale pane.current 不得抑制"
        );
        assert!(
            herdr_focus_suppression_hit(true, true),
            "外层宿主在看 + pane 命中 -> 抑制"
        );
        assert!(
            !herdr_focus_suppression_hit(true, false),
            "外层在看但 herdr 里是别的 pane -> 弹"
        );
        assert!(!herdr_focus_suppression_hit(false, false));
    }

    /// Cursor 前台判断:兼容 Insiders / Nightly 变体(Bug 4)。
    #[test]
    fn is_cursor_frontmost_accepts_family_variants() {
        assert!(is_cursor_frontmost(Some("Cursor")));
        assert!(is_cursor_frontmost(Some("Cursor Insiders")));
        assert!(is_cursor_frontmost(Some("Cursor Nightly")));
    }

    /// 非 Cursor 家族不匹配(避免 Terminal / iTerm2 / 假冒名 "CursorX" 误伤)。
    #[test]
    fn is_cursor_frontmost_rejects_non_family() {
        assert!(!is_cursor_frontmost(None));
        assert!(!is_cursor_frontmost(Some("")));
        assert!(!is_cursor_frontmost(Some("iTerm2")));
        assert!(!is_cursor_frontmost(Some("Ghostty")));
        // 前缀但没有分隔空格:避免 "Cursor2 / CursorX" 类假冒名混入
        assert!(!is_cursor_frontmost(Some("CursorX")));
    }

    /// Warp app 级聚焦判定:前台是 Warp 家族 -> 抑制;其它前台 / None -> 不抑制。
    /// 名单与 focus 回跳自查(`commands/focus/warp.rs::frontmost_is_warp`)共用。
    #[test]
    fn warp_session_focused_matches_warp_family_only() {
        for name in ["Warp", "stable", "preview", "dev", "oss", "warp"] {
            assert!(
                is_warp_session_focused(Some(name)),
                "{name} 前台应判 Warp 聚焦"
            );
        }
        for name in [
            "iTerm2", "Ghostty", "Terminal", "WezTerm", "Cursor", "Finder",
        ] {
            assert!(
                !is_warp_session_focused(Some(name)),
                "{name} 前台不应判 Warp 聚焦"
            );
        }
        assert!(!is_warp_session_focused(None));
    }
}

#[cfg(test)]
mod dispatch_table_tests {
    use super::*;

    #[test]
    fn focus_checks_table_order_matches_original_chain() {
        // W7(brief 20260813-focus-dispatch-table)守护:表行顺序 = W6 前链的
        // 逐条顺序(herdr → wezterm → orca → otty → warp → cmux → cursor →
        // dsh → terminal-tty 兜底)。插错顺序 = 判定短路错位 = 漏弹/误弹。
        let names: Vec<&str> = FOCUS_CHECKS.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec![
                "herdr",
                "wezterm",
                "orca",
                "otty",
                "warp",
                "cmux",
                "cursor",
                "dsh",
                "terminal-tty",
            ]
        );
        // terminal-tty 兜底必须永远负责(原链末行无条件执行)。
        let evidence = SurfaceEvidence::from_session(&session_stub());
        let last = FOCUS_CHECKS.last().unwrap().1;
        assert!(
            last(&evidence, None).is_some(),
            "terminal-tty 兜底必须永远 Some"
        );
    }

    #[test]
    fn check_herdr_responsible_only_when_binding_present() {
        let evidence = SurfaceEvidence::from_session(&session_stub());
        assert!(
            check_herdr(&evidence, None).is_none(),
            "无 herdr 绑定不负责"
        );
        let mut s = session_stub();
        s.herdr_pane_id = Some("w3:p1".into());
        s.herdr_socket_path = Some("/s".into());
        s.surface = crate::surface::SurfaceBinding::Herdr {
            pane_id: "w3:p1".into(),
            workspace_id: None,
            socket_path: Some("/s".into()),
        };
        let ev = SurfaceEvidence::from_session(&s);
        // 前台非 herdr 相关 → 判定不聚焦(但负责,返回 Some(false))
        assert_eq!(check_herdr(&ev, Some("SomeOtherApp")), Some(false));
    }

    fn session_stub() -> crate::sessions::Session {
        let now = chrono::Utc::now();
        crate::sessions::Session {
            id: "s1".into(),
            task_id: "cc-1".into(),
            external_session_id: None,
            title: "t".into(),
            message: "m".into(),
            source: Some("claude-code".into()),
            priority: Default::default(),
            status: crate::sessions::SessionStatus::Running,
            created_at: now,
            updated_at: now,
            terminal_tty: Some("/dev/ttys001".into()),
            workspace_path: None,
            failure_reason: None,
            aborted_at: None,
            terminal_id: None,
            terminal_id_captured_cwd: None,
            transcript_path: None,
            herdr_pane_id: None,
            herdr_workspace_id: None,
            herdr_socket_path: None,
            cmux_surface_id: None,
            cmux_workspace_id: None,
            wezterm_pane_id: None,
            wezterm_unix_socket: None,
            wezterm_executable: None,
            otty_pane_id: None,
            otty_tab_id: None,
            otty_window_id: None,
            tmux_pane_id: None,
            tmux_socket_path: None,
            orca_pane_key: None,
            orca_worktree_id: None,
            orca_tab_id: None,
            terminal_app: None,
            dsh_nav_socket: None,
            surface: Default::default(),
        }
    }
}
