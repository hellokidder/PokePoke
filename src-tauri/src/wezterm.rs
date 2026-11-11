//! WezTerm CLI 最小客户端:解析 `wezterm cli list --format json`,shell-out
//! `wezterm cli activate-pane`。只服务直接跑在 WezTerm pane 里的 CC 回跳;
//! cmux / herdr 内层场景由各自 short-circuit 优先。

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WeztermPaneRow {
    pub pane_id: u64,
    #[serde(default)]
    pub tab_id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub tty_name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WeztermClientRow {
    pub focused_pane_id: u64,
}

const MACOS_WEZTERM_CLI: &str = "/Applications/WezTerm.app/Contents/MacOS/wezterm";
const HOMEBREW_WEZTERM_CLI: &str = "/opt/homebrew/bin/wezterm";

/// macOS WezTerm pane 里的 `WEZTERM_EXECUTABLE` 常指向 `wezterm-gui`,该二进制没有
/// `cli` 子命令;`wezterm cli list` / `activate-pane` 必须改用同目录的 `wezterm`。
pub(crate) fn normalize_wezterm_cli_executable(executable: &str) -> String {
    let path = Path::new(executable.trim());
    match path.file_name().and_then(|n| n.to_str()) {
        Some("wezterm-gui") => path.with_file_name("wezterm").display().to_string(),
        _ => executable.trim().to_string(),
    }
}

fn resolve_wezterm_cli_executable(executable: Option<&str>) -> PathBuf {
    if let Some(exe) = executable.filter(|s| !s.is_empty()) {
        let normalized = normalize_wezterm_cli_executable(exe);
        if Path::new(&normalized).is_file() {
            return PathBuf::from(normalized);
        }
    }
    for candidate in [MACOS_WEZTERM_CLI, HOMEBREW_WEZTERM_CLI] {
        if Path::new(candidate).is_file() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("wezterm")
}

fn command_with_socket(executable: Option<&str>, unix_socket: &str, args: &[&str]) -> Command {
    let bin = resolve_wezterm_cli_executable(executable);
    let mut cmd = Command::new(bin);
    cmd.env("WEZTERM_UNIX_SOCKET", unix_socket);
    cmd.args(args);
    cmd
}

/// 纯函数:从 `wezterm cli list --format json` 输出解析 pane 行。
pub(crate) fn parse_pane_list_json(raw: &str) -> Option<Vec<WeztermPaneRow>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

/// 纯函数:从 `wezterm cli list-clients --format json` 输出解析 client 行。
pub(crate) fn parse_clients_json(raw: &str) -> Option<Vec<WeztermClientRow>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

/// 把 WezTerm `list` 的 `cwd` 字段归一成本地路径。
/// WezTerm macOS 常见 `file://hostname/Users/...`,不是 `file:///Users/...`。
fn normalize_wezterm_pane_cwd(pane_cwd: &str) -> Option<PathBuf> {
    let trimmed = pane_cwd.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let path = if without_scheme.starts_with('/') {
        without_scheme
    } else if let Some(idx) = without_scheme.find("/Users/") {
        &without_scheme[idx..]
    } else if let Some(idx) = without_scheme.find("/private/Users/") {
        without_scheme[idx..]
            .strip_prefix("/private")
            .unwrap_or(&without_scheme[idx..])
    } else {
        let idx = without_scheme.find("/Volumes/")?;
        &without_scheme[idx..]
    };
    Some(PathBuf::from(path))
}

fn tty_matches(row_tty: &str, session_tty: &str) -> bool {
    let row_tty = row_tty.trim();
    let session_tty = session_tty.trim();
    if row_tty.is_empty() || session_tty.is_empty() {
        return false;
    }
    row_tty == session_tty
        || format!("/dev/{row_tty}") == session_tty
        || row_tty == session_tty.strip_prefix("/dev/").unwrap_or(session_tty)
}

/// 纯函数:WezTerm `cwd` 字段(`file://...`)与 session `workspace_path` 是否兼容。
pub(crate) fn cwd_compatible(pane_cwd: &str, workspace_path: &str) -> bool {
    let workspace = workspace_path.trim();
    if workspace.is_empty() {
        return true;
    }
    let Some(pane_path) = normalize_wezterm_pane_cwd(pane_cwd) else {
        return false;
    };
    let workspace_path = Path::new(workspace);
    pane_path == workspace_path
        || pane_path.starts_with(workspace_path)
        || workspace_path.starts_with(&pane_path)
}

/// 纯函数:pane cwd 是否落在 workspace 内(含自身),**不含**父目录。
/// capture 发现路径专用:双向 `cwd_compatible` 会让一个 `$HOME` shell 匹配
/// `/Users/edy/proj` workspace(`workspace.starts_with(pane)`),把误绑的 pane
/// 写进 binding,后续点击跳到 home shell 而非 agent pane。发现路径必须方向匹配。
pub(crate) fn cwd_within_workspace(pane_cwd: &str, workspace_path: &str) -> bool {
    let workspace = workspace_path.trim();
    if workspace.is_empty() {
        return true;
    }
    let Some(pane_path) = normalize_wezterm_pane_cwd(pane_cwd) else {
        return false;
    };
    let workspace_path = Path::new(workspace);
    pane_path == workspace_path || pane_path.starts_with(workspace_path)
}

/// 纯函数:在 list 结果里找目标 pane;workspace_path 非空时优先 cwd,tty 作兜底。
pub(crate) fn find_pane_row<'a>(
    rows: &'a [WeztermPaneRow],
    pane_id: u64,
    workspace_path: Option<&str>,
    terminal_tty: Option<&str>,
) -> Option<&'a WeztermPaneRow> {
    let row = rows.iter().find(|r| r.pane_id == pane_id)?;
    if let Some(ws) = workspace_path.filter(|s| !s.is_empty()) {
        if cwd_compatible(&row.cwd, ws) {
            return Some(row);
        }
        if terminal_tty
            .filter(|t| !t.is_empty())
            .is_some_and(|tty| tty_matches(&row.tty_name, tty))
        {
            return Some(row);
        }
        return None;
    }
    Some(row)
}

/// 纯函数:按 workspace cwd 唯一匹配 pane。0 / 多命中返回 None。
///
/// 用方向匹配 `cwd_within_workspace`(pane 在 workspace 内),不用双向
/// `cwd_compatible` -- 发现路径下父目录 pane(如 home shell)不应被误绑。
pub(crate) fn find_unique_pane_by_workspace<'a>(
    rows: &'a [WeztermPaneRow],
    workspace_path: &str,
) -> Option<&'a WeztermPaneRow> {
    let ws = workspace_path.trim();
    if ws.is_empty() {
        return None;
    }
    let matches: Vec<_> = rows
        .iter()
        .filter(|r| cwd_within_workspace(&r.cwd, ws))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

/// 纯函数:`list-clients` 的 `focused_pane_id` 是否等于目标 pane。
pub(crate) fn clients_focus_pane(clients: &[WeztermClientRow], pane_id: u64) -> bool {
    clients.iter().any(|c| c.focused_pane_id == pane_id)
}

fn list_panes(executable: Option<&str>, unix_socket: &str) -> Option<Vec<WeztermPaneRow>> {
    let cli = resolve_wezterm_cli_executable(executable);
    let output = command_with_socket(
        executable,
        unix_socket,
        &["cli", "list", "--format", "json"],
    )
    .output()
    .ok()?;
    if !output.status.success() {
        crate::log_info!(
            "focus",
            "wezterm list cli={} exit={} stderr={}",
            cli.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    parse_pane_list_json(&String::from_utf8_lossy(&output.stdout))
}

fn list_clients(executable: Option<&str>, unix_socket: &str) -> Option<Vec<WeztermClientRow>> {
    let output = command_with_socket(
        executable,
        unix_socket,
        &["cli", "list-clients", "--format", "json"],
    )
    .output()
    .ok()?;
    if !output.status.success() {
        crate::log_info!(
            "popup",
            "wezterm list-clients exit={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    parse_clients_json(&String::from_utf8_lossy(&output.stdout))
}

fn activate_pane(executable: Option<&str>, unix_socket: &str, pane_id: u64) -> bool {
    let pane = pane_id.to_string();
    let output = command_with_socket(
        executable,
        unix_socket,
        &["cli", "activate-pane", "--pane-id", pane.as_str()],
    )
    .output();
    match output {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            crate::log_info!(
                "focus",
                "wezterm activate-pane pane_id={} exit={} stderr={}",
                pane_id,
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            );
            false
        }
        Err(e) => {
            crate::log_info!(
                "focus",
                "wezterm activate-pane spawn err={} pane_id={}",
                e,
                pane_id
            );
            false
        }
    }
}

fn activate_tab(executable: Option<&str>, unix_socket: &str, tab_id: u64) -> bool {
    let tab = tab_id.to_string();
    command_with_socket(
        executable,
        unix_socket,
        &["cli", "activate-tab", "--tab-id", tab.as_str()],
    )
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn activate_wezterm_application() {
    let _ = Command::new("open").args(["-a", "WezTerm"]).status();
}

#[cfg(not(target_os = "macos"))]
fn activate_wezterm_application() {}

/// server 侧 cwd 补 capture:workspace 唯一命中才返回 pane_id 字符串。
pub fn capture_pane_by_workspace(
    unix_socket: &str,
    executable: Option<&str>,
    workspace_path: &str,
) -> Option<String> {
    let socket = unix_socket.trim();
    if socket.is_empty() {
        return None;
    }
    let rows = list_panes(executable, socket)?;
    let row = find_unique_pane_by_workspace(&rows, workspace_path)?;
    Some(row.pane_id.to_string())
}

/// 纯函数:`list-clients` 当前聚焦 pane 的 tty 是否等于目标 tty。
pub(crate) fn focused_pane_tty_matches(
    clients: &[WeztermClientRow],
    panes: &[WeztermPaneRow],
    terminal_tty: &str,
) -> bool {
    clients.iter().any(|c| {
        panes
            .iter()
            .find(|p| p.pane_id == c.focused_pane_id)
            .is_some_and(|p| tty_matches(&p.tty_name, terminal_tty))
    })
}

fn discover_wezterm_gui_sockets() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let dir = PathBuf::from(home).join(".local/share/wezterm");
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut socks: Vec<String> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gui-sock-"))
        })
        .map(|p| p.display().to_string())
        .collect();
    socks.sort();
    socks.dedup();
    socks
}

fn is_tty_focused_in_socket(
    terminal_tty: &str,
    unix_socket: &str,
    executable: Option<&str>,
) -> bool {
    let socket = unix_socket.trim();
    if socket.is_empty() || terminal_tty.trim().is_empty() {
        return false;
    }
    let clients = match list_clients(executable, socket) {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };
    let panes = match list_panes(executable, socket) {
        Some(p) => p,
        None => return false,
    };
    focused_pane_tty_matches(&clients, &panes, terminal_tty)
}

/// 纯函数:`list` 里是否存在唯一 pane 的 tty 等于目标。
pub(crate) fn unique_pane_for_tty<'a>(
    rows: &'a [WeztermPaneRow],
    terminal_tty: &str,
) -> Option<&'a WeztermPaneRow> {
    let matches: Vec<_> = rows
        .iter()
        .filter(|r| tty_matches(&r.tty_name, terminal_tty))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

fn focus_tty_in_socket(terminal_tty: &str, unix_socket: &str, executable: Option<&str>) -> bool {
    let socket = unix_socket.trim();
    if socket.is_empty() || terminal_tty.trim().is_empty() {
        return false;
    }
    let rows = match list_panes(executable, socket) {
        Some(r) => r,
        None => return false,
    };
    let Some(row) = unique_pane_for_tty(&rows, terminal_tty) else {
        return false;
    };
    if !activate_pane(executable, socket, row.pane_id) {
        return false;
    }
    if row.tab_id > 0 {
        let tab_ok = activate_tab(executable, socket, row.tab_id);
        crate::log_info!(
            "focus",
            "wezterm focus_tty activate-tab tab_id={} pane_id={} tty={} ok={}",
            row.tab_id,
            row.pane_id,
            terminal_tty,
            tab_ok
        );
    }
    activate_wezterm_application();
    crate::log_info!(
        "focus",
        "wezterm focus_tty ok pane_id={} socket={} tty={}",
        row.pane_id,
        socket,
        terminal_tty
    );
    true
}

/// herdr-in-WezTerm 段 A:按 pane tty 定位并 `activate-pane`。
pub fn focus_tty(terminal_tty: &str, unix_socket: Option<&str>, executable: Option<&str>) -> bool {
    if terminal_tty.trim().is_empty() {
        return false;
    }
    if let Some(sock) = unix_socket.filter(|s| !s.is_empty()) {
        return focus_tty_in_socket(terminal_tty, sock, executable);
    }
    for sock in discover_wezterm_gui_sockets() {
        if focus_tty_in_socket(terminal_tty, &sock, executable) {
            return true;
        }
    }
    false
}

/// 该 tty 是否出现在指定(或任一) WezTerm 实例的 pane 列表里。
pub fn pane_exists_for_tty(
    terminal_tty: &str,
    unix_socket: Option<&str>,
    executable: Option<&str>,
) -> bool {
    if terminal_tty.trim().is_empty() {
        return false;
    }
    let check = |sock: &str| {
        list_panes(executable, sock)
            .is_some_and(|rows| unique_pane_for_tty(&rows, terminal_tty).is_some())
    };
    if let Some(sock) = unix_socket.filter(|s| !s.is_empty()) {
        return check(sock);
    }
    discover_wezterm_gui_sockets().iter().any(|s| check(s))
}

/// herdr-in-WezTerm 外层宿主聚焦:WezTerm 前台且 `list-clients` 聚焦 pane 的 tty 命中。
/// `unix_socket` 为空时扫描 `~/.local/share/wezterm/gui-sock-*`(多实例取首个命中)。
pub fn is_tty_focused(
    terminal_tty: &str,
    unix_socket: Option<&str>,
    executable: Option<&str>,
) -> bool {
    if terminal_tty.trim().is_empty() {
        return false;
    }
    if let Some(sock) = unix_socket.filter(|s| !s.is_empty()) {
        return is_tty_focused_in_socket(terminal_tty, sock, executable);
    }
    discover_wezterm_gui_sockets()
        .iter()
        .any(|sock| is_tty_focused_in_socket(terminal_tty, sock, executable))
}

/// popup / focus-watcher:`list-clients.focused_pane_id` + pane cwd 校验。
pub fn is_pane_focused(
    pane_id: &str,
    unix_socket: &str,
    executable: Option<&str>,
    workspace_path: Option<&str>,
) -> bool {
    let Ok(want) = pane_id.trim().parse::<u64>() else {
        return false;
    };
    let socket = unix_socket.trim();
    if socket.is_empty() {
        return false;
    }
    let clients = match list_clients(executable, socket) {
        Some(c) => c,
        None => return false,
    };
    if !clients_focus_pane(&clients, want) {
        return false;
    }
    let rows = match list_panes(executable, socket) {
        Some(r) => r,
        None => return false,
    };
    find_pane_row(&rows, want, workspace_path, None).is_some()
}

/// 聚焦 WezTerm pane 并在成功后校验 pane 仍存在(防 pane_id 数字复用误跳)。
pub fn focus_pane(
    pane_id: &str,
    unix_socket: &str,
    executable: Option<&str>,
    workspace_path: Option<&str>,
    terminal_tty: Option<&str>,
) -> bool {
    let Ok(pane_num) = pane_id.trim().parse::<u64>() else {
        crate::log_info!("focus", "wezterm focus_pane invalid pane_id={pane_id:?}");
        return false;
    };
    if unix_socket.trim().is_empty() {
        return false;
    }
    let socket = unix_socket.trim();
    let cli = resolve_wezterm_cli_executable(executable);
    crate::log_info!(
        "focus",
        "wezterm focus_pane pane_id={} socket={} cli={} workspace={} tty={:?}",
        pane_num,
        socket,
        cli.display(),
        workspace_path.unwrap_or("<none>"),
        terminal_tty
    );

    let rows = match list_panes(executable, socket) {
        Some(rows) => rows,
        None => return false,
    };
    let Some(row) = find_pane_row(&rows, pane_num, workspace_path, terminal_tty) else {
        crate::log_info!(
            "focus",
            "wezterm focus_pane pre-check miss pane_id={} (stale or cwd/tty mismatch)",
            pane_num
        );
        return false;
    };

    if !activate_pane(executable, socket, pane_num) {
        return false;
    }
    if row.tab_id > 0 {
        let tab_ok = activate_tab(executable, socket, row.tab_id);
        crate::log_info!(
            "focus",
            "wezterm activate-tab tab_id={} pane_id={} ok={}",
            row.tab_id,
            pane_num,
            tab_ok
        );
    }
    activate_wezterm_application();

    let rows = match list_panes(executable, socket) {
        Some(rows) => rows,
        None => return true,
    };
    let ok = find_pane_row(&rows, pane_num, workspace_path, terminal_tty).is_some();
    if ok {
        crate::log_info!("focus", "wezterm focus_pane ok pane_id={}", pane_num);
    } else {
        crate::log_info!(
            "focus",
            "wezterm focus_pane post-check miss pane_id={}",
            pane_num
        );
    }
    ok
}

/// focus 失败后是否应清 wezterm binding:仅 pane 从 list 消失才算 stale;
/// cwd 不兼容 / CLI 临时失败保留 binding 供重试。
pub fn should_clear_wezterm_binding_after_focus_fail(
    pane_id: &str,
    unix_socket: &str,
    executable: Option<&str>,
) -> bool {
    let Ok(pane_num) = pane_id.trim().parse::<u64>() else {
        return true;
    };
    let socket = unix_socket.trim();
    if socket.is_empty() {
        return false;
    }
    match list_panes(executable, socket) {
        Some(rows) => !rows.iter().any(|r| r.pane_id == pane_num),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_wezterm_cli_executable_rewrites_gui_binary() {
        assert_eq!(
            normalize_wezterm_cli_executable(
                "/Applications/WezTerm.app/Contents/MacOS/wezterm-gui"
            ),
            "/Applications/WezTerm.app/Contents/MacOS/wezterm"
        );
        assert_eq!(
            normalize_wezterm_cli_executable("/opt/homebrew/bin/wezterm"),
            "/opt/homebrew/bin/wezterm"
        );
    }

    #[test]
    fn parse_pane_list_json_reads_rows() {
        let rows = parse_pane_list_json(
            r#"[{"pane_id":3,"tab_id":1,"title":"cc","cwd":"file:///Users/edy/proj","tty_name":"/dev/ttys001"}]"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pane_id, 3);
        assert_eq!(rows[0].tab_id, 1);
    }

    #[test]
    fn parse_clients_json_reads_focused_pane_id() {
        let clients = parse_clients_json(r#"[{"focused_pane_id":7}]"#).unwrap();
        assert_eq!(clients[0].focused_pane_id, 7);
        assert!(clients_focus_pane(&clients, 7));
        assert!(!clients_focus_pane(&clients, 8));
    }

    #[test]
    fn unique_pane_for_tty_requires_single_match() {
        let rows = parse_pane_list_json(
            r#"[{"pane_id":3,"tty_name":"ttys041"},{"pane_id":4,"tty_name":"ttys099"}]"#,
        )
        .unwrap();
        assert_eq!(
            unique_pane_for_tty(&rows, "/dev/ttys041").map(|r| r.pane_id),
            Some(3)
        );
        assert!(unique_pane_for_tty(&rows, "ttys041").is_some());
        assert!(unique_pane_for_tty(&rows, "/dev/ttys000").is_none());
    }

    #[test]
    fn focused_pane_tty_matches_client_row() {
        let clients = parse_clients_json(r#"[{"focused_pane_id":7}]"#).unwrap();
        let panes = parse_pane_list_json(
            r#"[{"pane_id":7,"tty_name":"ttys041"},{"pane_id":8,"tty_name":"ttys099"}]"#,
        )
        .unwrap();
        assert!(focused_pane_tty_matches(&clients, &panes, "/dev/ttys041"));
        assert!(focused_pane_tty_matches(&clients, &panes, "ttys041"));
        assert!(!focused_pane_tty_matches(&clients, &panes, "/dev/ttys099"));
    }

    #[test]
    fn find_unique_pane_by_workspace_requires_single_match() {
        let rows = vec![
            WeztermPaneRow {
                pane_id: 1,
                tab_id: 0,
                title: "a".into(),
                cwd: "file:///Users/edy/proj".into(),
                tty_name: String::new(),
            },
            WeztermPaneRow {
                pane_id: 2,
                tab_id: 0,
                title: "b".into(),
                cwd: "file:///Users/edy/proj".into(),
                tty_name: String::new(),
            },
        ];
        assert!(find_unique_pane_by_workspace(&rows, "/Users/edy/proj").is_none());
        let single = vec![rows[0].clone()];
        assert_eq!(
            find_unique_pane_by_workspace(&single, "/Users/edy/proj").map(|r| r.pane_id),
            Some(1)
        );
    }

    #[test]
    fn find_unique_pane_by_workspace_rejects_parent_dir_pane() {
        // capture 发现路径:仅 home shell 在 list 里时,不能匹配深层 workspace
        // (双向 cwd_compatible 会因 workspace.starts_with(pane) 误命中)。
        let home_shell = vec![WeztermPaneRow {
            pane_id: 1,
            tab_id: 0,
            title: "shell".into(),
            cwd: "file:///Users/edy".into(),
            tty_name: String::new(),
        }];
        assert!(find_unique_pane_by_workspace(&home_shell, "/Users/edy/proj").is_none());

        // 子目录 pane 仍命中(方向匹配:pane 在 workspace 内)。
        let child = vec![WeztermPaneRow {
            pane_id: 2,
            tab_id: 0,
            title: "cc".into(),
            cwd: "file:///Users/edy/proj/src".into(),
            tty_name: String::new(),
        }];
        assert_eq!(
            find_unique_pane_by_workspace(&child, "/Users/edy/proj").map(|r| r.pane_id),
            Some(2)
        );

        // 父目录 pane 与子目录 pane 并存 -> 0 命中(父不匹配,子匹配 -> 1 命中)
        // 这里确认父目录被方向匹配排除,只剩子目录唯一命中。
        let mixed = vec![
            WeztermPaneRow {
                pane_id: 1,
                tab_id: 0,
                title: "shell".into(),
                cwd: "file:///Users/edy".into(),
                tty_name: String::new(),
            },
            child[0].clone(),
        ];
        assert_eq!(
            find_unique_pane_by_workspace(&mixed, "/Users/edy/proj").map(|r| r.pane_id),
            Some(2)
        );
    }

    #[test]
    fn cwd_compatible_accepts_wezterm_hostname_file_url() {
        assert!(cwd_compatible(
            "file://kidder.local/Users/edy/Documents/Kidder/poke-poke",
            "/Users/edy/Documents/Kidder/poke-poke"
        ));
    }

    #[test]
    fn cwd_compatible_accepts_prefix_and_exact() {
        assert!(cwd_compatible(
            "file:///Users/edy/proj/src",
            "/Users/edy/proj"
        ));
        assert!(!cwd_compatible(
            "file:///Users/edy/other",
            "/Users/edy/proj"
        ));
    }

    #[test]
    fn find_pane_row_accepts_tty_fallback_when_cwd_url_differs() {
        let rows = vec![WeztermPaneRow {
            pane_id: 10,
            tab_id: 1,
            title: "cc".into(),
            cwd: "file://kidder.local/Users/edy/proj".into(),
            tty_name: "/dev/ttys049".into(),
        }];
        assert!(find_pane_row(&rows, 10, Some("/Users/edy/other"), Some("/dev/ttys049")).is_some());
        assert!(find_pane_row(&rows, 10, Some("/Users/edy/other"), None).is_none());
    }

    #[test]
    fn find_pane_row_requires_cwd_when_workspace_given() {
        let rows = vec![WeztermPaneRow {
            pane_id: 7,
            tab_id: 0,
            title: "cc".into(),
            cwd: "file:///Users/edy/proj".into(),
            tty_name: String::new(),
        }];
        assert!(find_pane_row(&rows, 7, Some("/Users/edy/proj"), None).is_some());
        assert!(find_pane_row(&rows, 7, Some("/Users/edy/other"), None).is_none());
    }
}
