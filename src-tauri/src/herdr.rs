//! herdr JSON socket 最小客户端(newline-delimited JSON over Unix socket)。
//!
//! herdr 是本地 agent-aware terminal 复用器(https://herdr.dev/)。当 CC 跑在
//! herdr pane 里而不是直接在 Ghostty tab 里时,CC 的 OSC title 会被 herdr 拦截,
//! pokepoke 现有的 Ghostty tab 定位机制(title 前缀匹配)会自动切换到匹配 herdr
//! 自动写的 `<ws-label> · cc-<hex8>` title —— 这一部分不需要动。**本模块**只负责
//! "跳到目标 Ghostty tab 之后,让 herdr 内部把焦点切到 CC pane"这一段(通过
//! `agent.focus <pane_id>` 方法调用)。
//!
//! 无鉴权 —— herdr socket 依赖 fs 权限(0600 + owner uid)。所有调用先做
//! ownership check;所有 IO 有超时(轮询/探活/gate 500ms,focus 路径 2s,
//! 见 `FOCUS_SOCKET_TIMEOUT`);错误一律吞成 Option/false,让 caller
//! 无缝降级到"没接入 herdr"路径。

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const SOCKET_TIMEOUT: Duration = Duration::from_millis(500);

/// focus 路径专用超时(brief 20260807-herdr-remote-jumpback-fix):`workspace.focus` /
/// `agent.focus` 经 SSH 转发打到远程 herdr,刚连接 / 刚切 workspace 后响应会超过
/// 500ms(实测稳态 130~220ms,边缘 >500ms)。只放宽 focus 路径 —— 轮询 `list_agents` /
/// 探活 / popup gate 的 `pane.current` 在热路径,全局抬会拖慢 notify 与轮询。
const FOCUS_SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingInfo {
    pub protocol: u32,
    pub version: String,
}

/// herdr pane 的 agent 状态(herdr 0.7.3 pane.list 公开字段,grok 审查实测)。
///
/// 取值 idle/working/blocked/done/unknown。PokePoke 只把它当作 herdr 绑定
/// session 的探活第二信源:前四种视为 pane 存在且 agent 会话存活,unknown
/// (shell pane / herdr 无法判定)视为"无法判定",由调用方退回 tty 探活。
/// 未知字符串/缺省一律归 Unknown,不让新状态值破坏 pane.list 整体解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HerdrAgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    Unknown,
}

impl<'de> Deserialize<'de> for HerdrAgentStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(deserializer)?;
        Ok(match s.as_deref() {
            Some("idle") => HerdrAgentStatus::Idle,
            Some("working") => HerdrAgentStatus::Working,
            Some("blocked") => HerdrAgentStatus::Blocked,
            Some("done") => HerdrAgentStatus::Done,
            _ => HerdrAgentStatus::Unknown,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrPaneSnapshot {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: Option<String>,
    pub cwd: String,
    pub foreground_cwd: String,
    /// brief 20260804-herdr-agent-status:pane.list 公开字段,探活第二信源。
    pub agent_status: HerdrAgentStatus,
    /// pi 等 agent 的会话 jsonl 路径(herdr 侧知道,pi 扩展不报)。暂只解析+日志,
    /// transcript watcher 接入留 P3 后续。
    pub agent_session: Option<String>,
}

#[derive(Serialize)]
struct Request<'a> {
    id: &'a str,
    method: &'a str,
    params: serde_json::Value,
}

pub(crate) fn build_request(id: &str, method: &str, params: serde_json::Value) -> String {
    let req = Request { id, method, params };
    let mut s = serde_json::to_string(&req).unwrap_or_default();
    s.push('\n');
    s
}

#[derive(Deserialize)]
struct RawPane {
    pane_id: String,
    workspace_id: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    foreground_cwd: String,
    #[serde(default)]
    agent_status: HerdrAgentStatus,
    #[serde(default)]
    agent_session: Option<String>,
}

#[derive(Deserialize)]
struct PaneListResult {
    #[serde(default)]
    panes: Vec<RawPane>,
}

#[derive(Deserialize)]
struct EnvelopeOk<T> {
    #[allow(dead_code)]
    id: Option<String>,
    result: T,
}

#[derive(Deserialize)]
struct EnvelopeErr {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    error: serde_json::Value,
}

pub(crate) fn parse_pane_list(raw: &str) -> Result<Vec<HerdrPaneSnapshot>, String> {
    if let Ok(err) = serde_json::from_str::<EnvelopeErr>(raw) {
        if !err.error.is_null() {
            return Err(format!("herdr error response: {}", err.error));
        }
    }
    let ok: EnvelopeOk<PaneListResult> =
        serde_json::from_str(raw).map_err(|e| format!("parse pane_list: {e}"))?;
    Ok(ok
        .result
        .panes
        .into_iter()
        .map(|p| HerdrPaneSnapshot {
            pane_id: p.pane_id,
            workspace_id: p.workspace_id,
            agent: p.agent,
            cwd: p.cwd,
            foreground_cwd: p.foreground_cwd,
            agent_status: p.agent_status,
            agent_session: p.agent_session,
        })
        .collect())
}

#[derive(Deserialize)]
struct FocusResult {
    #[serde(default)]
    agent: FocusAgent,
}

#[derive(Deserialize, Default)]
struct FocusAgent {
    #[serde(default)]
    focused: bool,
}

pub(crate) fn parse_focus_result(raw: &str) -> Result<bool, String> {
    if let Ok(err) = serde_json::from_str::<EnvelopeErr>(raw) {
        if !err.error.is_null() {
            return Err(format!("herdr error: {}", err.error));
        }
    }
    let ok: EnvelopeOk<FocusResult> =
        serde_json::from_str(raw).map_err(|e| format!("parse focus: {e}"))?;
    Ok(ok.result.agent.focused)
}

/// 确认 socket 属主是当前进程 uid,防跨用户误连(macOS 罕见但正确)。
#[cfg(unix)]
fn socket_ownership_ok(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(path) {
        Ok(m) => {
            let cur = unsafe { libc::getuid() };
            m.uid() == cur
        }
        Err(_) => false,
    }
}
#[cfg(not(unix))]
fn socket_ownership_ok(_p: &Path) -> bool {
    false
}

/// 发一行 request 读一行 response(newline-delimited JSON);所有错误吞成 None。
/// pub(crate) 供 `herdr_remote`(远程 herdr 监视器)复用。
pub(crate) fn send_once(socket_path: &Path, request_line: &str) -> Option<String> {
    send_once_with_timeout(socket_path, request_line, SOCKET_TIMEOUT)
}

/// `send_once` 带显式超时版本。focus 路径用 [`FOCUS_SOCKET_TIMEOUT`],
/// 轮询 / 探活 / popup gate 走默认 [`SOCKET_TIMEOUT`]。
pub(crate) fn send_once_with_timeout(
    socket_path: &Path,
    request_line: &str,
    timeout: Duration,
) -> Option<String> {
    if !socket_ownership_ok(socket_path) {
        return None;
    }
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    stream.write_all(request_line.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

pub fn ping(socket_path: &Path) -> Option<PingInfo> {
    let req = build_request("poke-ping", "ping", serde_json::json!({}));
    let raw = send_once(socket_path, &req)?;
    #[derive(Deserialize)]
    struct PingResult {
        protocol: u32,
        version: String,
    }
    let ok: EnvelopeOk<PingResult> = serde_json::from_str(&raw).ok()?;
    Some(PingInfo {
        protocol: ok.result.protocol,
        version: ok.result.version,
    })
}

pub fn focus_workspace(socket_path: &Path, workspace_id: &str) -> bool {
    let req = build_request(
        "poke-ws-focus",
        "workspace.focus",
        serde_json::json!({"workspace_id": workspace_id}),
    );
    match send_once_with_timeout(socket_path, &req, FOCUS_SOCKET_TIMEOUT) {
        Some(raw) => parse_workspace_focus_result(&raw).unwrap_or(false),
        None => false,
    }
}

pub(crate) fn parse_workspace_focus_result(raw: &str) -> Result<bool, String> {
    if let Ok(err) = serde_json::from_str::<EnvelopeErr>(raw) {
        if !err.error.is_null() {
            return Err(format!("herdr error: {}", err.error));
        }
    }
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("parse ws focus: {e}"))?;
    let focused = v
        .pointer("/result/workspace/focused")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    Ok(focused)
}

/// focus 路径结果分型(brief 20260807-herdr-remote-jumpback-fix)。
/// caller 按类型决定是否保留 binding / 是否重试:
/// - [`HerdrFocusOutcome::TransportFailure`] = 瞬态(超时/断连/无响应)→ 可重试、保留 binding
/// - [`HerdrFocusOutcome::ExplicitError`] = herdr 显式错误(如 not_found)→ pane 确定不存在,重试无意义
/// - [`HerdrFocusOutcome::NotFocused`] = 有响应但未聚焦成功(pane 存在)→ 保留 binding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HerdrFocusOutcome {
    Focused,
    NotFocused,
    TransportFailure,
    ExplicitError,
}

pub fn focus_pane(socket_path: &Path, pane_id: &str) -> HerdrFocusOutcome {
    let req = build_request(
        "poke-focus",
        "agent.focus",
        serde_json::json!({"target": pane_id}),
    );
    match send_once_with_timeout(socket_path, &req, FOCUS_SOCKET_TIMEOUT) {
        Some(raw) => match parse_focus_result(&raw) {
            Ok(true) => HerdrFocusOutcome::Focused,
            Ok(false) => HerdrFocusOutcome::NotFocused,
            Err(_) => HerdrFocusOutcome::ExplicitError,
        },
        None => HerdrFocusOutcome::TransportFailure,
    }
}

/// 段 B:先 `workspace.focus` 再 `agent.focus`。同 herdr server 多 workspace 时,
/// 只 focus pane 可能还在错的 workspace 视图里。返回 `agent.focus` 的分型结果
/// (workspace.focus 结果只记日志 —— 重试只重发 agent.focus,不重发 workspace)。
pub fn focus_agent_pane(
    socket_path: &Path,
    pane_id: &str,
    workspace_id: Option<&str>,
) -> HerdrFocusOutcome {
    if let Some(ws) = workspace_id.filter(|s| !s.is_empty()) {
        let ws_ok = focus_workspace(socket_path, ws);
        crate::log_info!("herdr", "focus_workspace workspace_id={} -> {}", ws, ws_ok);
    }
    focus_pane(socket_path, pane_id)
}

/// 让 herdr server 通知其 UI client 改写外层终端标题(OSC 2),返回 server 侧
/// 是否受理。brief 20260818-herdr-remote-warp-tab-focus 段 C 用:切到
/// `herdr --remote` client 所在 Warp tab 前,先经 remote server socket 把 tab
/// 标题改成唯一 marker(本机 tab 不会出现同标题),随后 warp local_control
/// `tab.activate` 按标题全等命中该 tab。
pub fn set_client_window_title(socket_path: &Path, title: &str) -> bool {
    let req = build_request(
        "poke-title-set",
        "client.window_title.set",
        serde_json::json!({ "title": title }),
    );
    send_once(socket_path, &req).is_some()
}

/// 按 herdr **server socket** 定位当前 attach 的 UI client 所在 tty。
///
/// 每个 named session 有独立 `session_dir` + `herdr-client.log`;UI client 写
/// 该 log → `lsof session_dir/herdr-client.log` 可 session-scoped 找 client pid,
/// 避免全局搜 title=="herdr" 在多 server 时撞车。
///
/// 枚举该 herdr server 当前 attach 的全部 UI client tty(0~N)。
#[cfg(unix)]
pub fn client_ttys_for_socket(server_socket: &Path) -> Vec<String> {
    let session_dir = match server_socket.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let client_log = session_dir.join("herdr-client.log");
    if !client_log.exists() {
        return Vec::new();
    }
    let output = match std::process::Command::new("lsof")
        .args(["-t", client_log.to_str().unwrap_or("")])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let mut ttys: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .filter(|pid| is_herdr_ui_client_pid(*pid))
        .filter_map(tty_for_pid)
        .collect();
    ttys.sort();
    ttys.dedup();
    ttys
}

/// 0 / 多个 UI client 写同一 log 时返 None(fail-safe)。
#[cfg(unix)]
pub fn client_tty_for_socket(server_socket: &Path) -> Option<String> {
    let ttys = client_ttys_for_socket(server_socket);
    match ttys.len() {
        1 => {
            crate::log_debug!(
                "herdr",
                "client_tty_for_socket sock={} tty={}",
                server_socket.display(),
                ttys[0]
            );
            Some(ttys[0].clone())
        }
        n => {
            if n > 1 {
                crate::log_info!(
                    "herdr",
                    "client_tty_for_socket skip: {} ui client ttys for sock={}",
                    n,
                    server_socket.display()
                );
            }
            None
        }
    }
}

#[cfg(not(unix))]
pub fn client_tty_for_socket(_server_socket: &Path) -> Option<String> {
    None
}

#[cfg(unix)]
fn is_herdr_ui_client_pid(pid: u32) -> bool {
    let output = match std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let comm = String::from_utf8_lossy(&output.stdout);
    comm.trim() == "herdr"
}

#[cfg(unix)]
fn tty_for_pid(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if tty.is_empty() || tty == "??" || tty == "-" {
        return None;
    }
    Some(tty)
}

/// `herdr --remote <target>` UI client 所在终端 tty。
///
/// `herdr --remote <target>` 的 wrapper 进程会在本机建一个本地转发 socket
/// (`/var/folders/<...>/herdr-remote-<wrapper_pid>-<target>-default.sock`),UI
/// client 进程连接它经 SSH 桥到远程 server。PokePoke 与 wrapper 的连接是两条
/// 独立转发(见 herdr_remote.rs),但目标都是同一远程 server —— 段 B 把远程
/// focus 切对之后,需要激活承载本地 herdr client 的终端窗口(段 C),否则用户
/// 眼睛在公司机看不到任何回跳变化。
///
/// 用 `lsof -U` 匹配 wrapper 转发 socket 名拿到 wrapper pid,再取该进程 tty
/// (与 client 同终端)。多次启动遗留多个实例时取 socket 文件 mtime 最新的。
pub fn herdr_remote_client_tty(target: &str) -> Option<String> {
    let out = std::process::Command::new("lsof")
        .args(["-U"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let matches = parse_herdr_remote_socket_lines(&String::from_utf8_lossy(&out.stdout), target);
    if matches.is_empty() {
        return None;
    }
    // 多实例取 socket 文件 mtime 最新(每次 herdr --remote 启动都会新建转发
    // socket,mtime = 启动时刻)。
    let mut newest: Option<(u32, std::time::SystemTime)> = None;
    for (pid, path) in matches {
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let keep = match (&newest, mtime) {
            (None, Some(t)) => Some((pid, t)),
            (None, None) => Some((pid, std::time::UNIX_EPOCH)),
            (Some((_, best)), Some(t)) if t > *best => Some((pid, t)),
            _ => newest,
        };
        newest = keep;
    }
    let pid = newest.map(|(p, _)| p)?;
    crate::log_info!(
        "herdr",
        "herdr_remote_client_tty target={} pid={} tty=?",
        target,
        pid
    );
    tty_for_pid(pid)
}

/// 解析 `lsof -U` 输出中匹配 `herdr-remote-*-<target>-default.sock` 的行,
/// 返回 `(pid, socket 路径)`。纯函数,便于单测。
pub(crate) fn parse_herdr_remote_socket_lines(lsof_text: &str, target: &str) -> Vec<(u32, String)> {
    let suffix = format!("-{}-default.sock", target);
    let mut out = Vec::new();
    for line in lsof_text.lines() {
        if !line.contains("herdr-remote-") || !line.ends_with(&suffix) {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(pid_s), Some(path)) = (fields.get(1), fields.last()) else {
            continue;
        };
        let Ok(pid) = pid_s.parse::<u32>() else {
            continue;
        };
        out.push((pid, path.to_string()));
    }
    out
}

#[derive(Deserialize, Default)]
struct PaneCurrentResult {
    #[serde(default)]
    pane: PaneCurrentInfo,
}

#[derive(Deserialize, Default)]
struct PaneCurrentInfo {
    #[serde(default)]
    pane_id: String,
    #[serde(default)]
    focused: bool,
}

/// 从 herdr server socket 路径解析 named session 目录名。
///
/// - `~/.config/herdr/herdr.sock` → `None`(default 实例)
/// - `~/.config/herdr/sessions/work/herdr.sock` → `Some("work")`
pub(crate) fn named_session_from_socket_path(sock: &Path) -> Option<String> {
    let parent = sock.parent()?;
    if parent.parent()?.file_name()? == std::ffi::OsStr::new("sessions") {
        return parent
            .file_name()?
            .to_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
    }
    None
}

/// Ghostty tab title 是否对应该 herdr server 实例。
///
/// herdr 0.7.3 把 tab 写成 `herdr` 或 `herdr --session <name>`;多实例并存时
/// 不能只靠 `starts with "herdr"` 唯一命中,必须按 socket 推导的 session 名消歧。
pub(crate) fn ghostty_tab_title_matches_named_session(
    tab_title: &str,
    named_session: Option<&str>,
) -> bool {
    let name = tab_title.trim();
    if !name.starts_with("herdr") {
        return false;
    }
    match named_session.filter(|s| !s.is_empty() && *s != "default") {
        None => name == "herdr" || name == "herdr --session default",
        Some(session) => {
            let prefix = format!("herdr --session {session}");
            name == prefix || name.starts_with(&(prefix.clone() + " "))
        }
    }
}

pub(crate) fn parse_pane_current(raw: &str) -> Result<Option<String>, String> {
    if let Ok(err) = serde_json::from_str::<EnvelopeErr>(raw) {
        if !err.error.is_null() {
            return Err(format!("herdr error response: {}", err.error));
        }
    }
    let ok: EnvelopeOk<PaneCurrentResult> =
        serde_json::from_str(raw).map_err(|e| format!("parse pane_current: {e}"))?;
    if !ok.result.pane.focused || ok.result.pane.pane_id.is_empty() {
        return Ok(None);
    }
    Ok(Some(ok.result.pane.pane_id))
}

/// 查询 herdr 当前 focused pane id。失败/超时吞成 None。
pub fn current_focused_pane_id(socket_path: &Path) -> Option<String> {
    let req = build_request("poke-current", "pane.current", serde_json::json!({}));
    match send_once(socket_path, &req) {
        Some(raw) => parse_pane_current(&raw).ok().flatten(),
        None => None,
    }
}

/// session 绑定的 herdr pane 是否为 herdr 内部当前焦点。
pub fn is_herdr_pane_focused(socket_path: &Path, pane_id: &str) -> bool {
    current_focused_pane_id(socket_path).as_deref() == Some(pane_id)
}

pub fn list_agent_panes(socket_path: &Path) -> Vec<HerdrPaneSnapshot> {
    let req = build_request("poke-list", "pane.list", serde_json::json!({}));
    match send_once(socket_path, &req) {
        Some(raw) => parse_pane_list(&raw).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// 单个 pane 快照在 herdr 侧是否**明确**存活。
///
/// - `Some(true)`:agent_status ∈ {idle, working, blocked, done} → 活。
/// - `None`:unknown(shell pane / herdr 无法判定)→ 无法判定,调用方退 tty 探活。
///
/// herdr 侧永不返回"确认死亡":socket 抖动 / 字段缺失都不该把活会话判死,
/// 与"探活失败倾向不删"的既有哲学一致。
pub fn pane_snapshot_alive(pane: &HerdrPaneSnapshot) -> Option<bool> {
    match pane.agent_status {
        HerdrAgentStatus::Idle
        | HerdrAgentStatus::Working
        | HerdrAgentStatus::Blocked
        | HerdrAgentStatus::Done => Some(true),
        HerdrAgentStatus::Unknown => None,
    }
}

/// herdr 绑定 session 的探活第二信源(brief 20260804-herdr-agent-status):
/// pane.list 查到该 pane 且状态明确报活 → `Some(true)`;查不到 pane /
/// unknown / socket 失败(send_once 吞错,500ms 超时)一律 `None`,由 caller
/// 退回既有 tty 探活。agent_session 暂只记 debug 日志,不做 transcript 接入。
pub fn pane_liveness(socket_path: &Path, pane_id: &str) -> Option<bool> {
    let panes = list_agent_panes(socket_path);
    let pane = panes.iter().find(|p| p.pane_id == pane_id)?;
    crate::log_debug!(
        "herdr",
        "pane_liveness pane_id={} agent_status={:?} agent_session={:?}",
        pane_id,
        pane.agent_status,
        pane.agent_session,
    );
    pane_snapshot_alive(pane)
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_request_produces_ndjson_line() {
        let line = super::build_request("req_1", "ping", serde_json::json!({}));
        assert!(line.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(parsed["id"], "req_1");
        assert_eq!(parsed["method"], "ping");
        assert!(parsed["params"].is_object());
    }

    #[test]
    fn parse_pane_list_extracts_only_claude_agents() {
        let raw = r#"{"id":"2","result":{"type":"pane_list","panes":[
            {"pane_id":"w3:p1","workspace_id":"w3","agent":"claude","cwd":"/a","foreground_cwd":"/a"},
            {"pane_id":"w3:p2","workspace_id":"w3","agent":null,"cwd":"/b","foreground_cwd":"/b"}
        ]}}"#;
        let panes = super::parse_pane_list(raw).unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pane_id, "w3:p1");
        assert_eq!(panes[0].agent.as_deref(), Some("claude"));
        assert_eq!(panes[1].agent, None);
        // 缺省字段:agent_status → Unknown,agent_session → None
        assert_eq!(panes[0].agent_status, super::HerdrAgentStatus::Unknown);
        assert_eq!(panes[0].agent_session, None);
    }

    #[test]
    fn parse_pane_list_extracts_agent_status_and_session() {
        let raw = r#"{"id":"2","result":{"type":"pane_list","panes":[
            {"pane_id":"w3:p1","workspace_id":"w3","agent":"pi","cwd":"/a","foreground_cwd":"/a","agent_status":"working","agent_session":"/tmp/pi/session.jsonl"},
            {"pane_id":"w3:p2","workspace_id":"w3","agent":null,"cwd":"/b","foreground_cwd":"/b","agent_status":"unknown"}
        ]}}"#;
        let panes = super::parse_pane_list(raw).unwrap();
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].agent_status, super::HerdrAgentStatus::Working);
        assert_eq!(
            panes[0].agent_session.as_deref(),
            Some("/tmp/pi/session.jsonl")
        );
        assert_eq!(panes[1].agent_status, super::HerdrAgentStatus::Unknown);
        assert_eq!(panes[1].agent_session, None);
    }

    #[test]
    fn parse_pane_list_unknown_status_value_falls_back_to_unknown() {
        // herdr 未来新增状态值不得破坏 pane.list 整体解析 → 归 Unknown。
        let raw = r#"{"id":"2","result":{"type":"pane_list","panes":[
            {"pane_id":"w3:p1","workspace_id":"w3","agent":"pi","cwd":"/a","foreground_cwd":"/a","agent_status":"super_new_state"}
        ]}}"#;
        let panes = super::parse_pane_list(raw).unwrap();
        assert_eq!(panes[0].agent_status, super::HerdrAgentStatus::Unknown);
    }

    #[test]
    fn pane_snapshot_alive_accepts_known_statuses_only() {
        use super::HerdrAgentStatus as S;
        let mk = |st| super::HerdrPaneSnapshot {
            pane_id: "w3:p1".into(),
            workspace_id: "w3".into(),
            agent: None,
            cwd: "/a".into(),
            foreground_cwd: "/a".into(),
            agent_status: st,
            agent_session: None,
        };
        for st in [S::Idle, S::Working, S::Blocked, S::Done] {
            assert_eq!(super::pane_snapshot_alive(&mk(st)), Some(true), "{st:?}");
        }
        assert_eq!(super::pane_snapshot_alive(&mk(S::Unknown)), None);
    }

    #[test]
    fn pane_liveness_roundtrip_with_real_socket() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let req: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
            let resp = r#"{"id":"poke-list","result":{"type":"pane_list","panes":[{"pane_id":"w3:p1","workspace_id":"w3","agent":"pi","cwd":"/a","foreground_cwd":"/a","agent_status":"blocked","agent_session":"/tmp/pi/s.jsonl"}]}}"#;
            stream.write_all(resp.as_bytes()).unwrap();
            stream.write_all(b"\n").unwrap();
            req
        });
        let alive = super::pane_liveness(&sock, "w3:p1");
        let received = handler.join().unwrap();
        assert_eq!(alive, Some(true));
        assert_eq!(received["method"], "pane.list");
    }

    #[test]
    fn pane_liveness_none_when_pane_missing_or_socket_down() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("missing.sock");
        assert_eq!(super::pane_liveness(&sock, "w3:p9"), None);
    }

    #[test]
    fn parse_pane_list_returns_err_on_error_response() {
        let raw = r#"{"id":"2","error":{"code":"internal","message":"nope"}}"#;
        assert!(super::parse_pane_list(raw).is_err());
    }

    #[test]
    fn parse_pane_current_reads_focused_pane_id() {
        let raw = r#"{"id":"c1","result":{"type":"pane_current","pane":{"pane_id":"w3:p8","focused":true}}}"#;
        assert_eq!(
            super::parse_pane_current(raw).unwrap().as_deref(),
            Some("w3:p8")
        );
    }

    #[test]
    fn parse_pane_current_none_when_not_focused() {
        let raw = r#"{"id":"c1","result":{"type":"pane_info","pane":{"pane_id":"w3:p8","focused":false}}}"#;
        assert_eq!(super::parse_pane_current(raw).unwrap(), None);
    }

    #[test]
    fn parse_focus_result_reads_type_agent_info() {
        let raw = r#"{"id":"3","result":{"type":"agent_info","agent":{"pane_id":"w3:p1","focused":true}}}"#;
        assert!(super::parse_focus_result(raw).unwrap());
    }

    #[test]
    fn parse_focus_result_returns_false_when_focused_false() {
        let raw = r#"{"id":"3","result":{"type":"agent_info","agent":{"pane_id":"w3:p1","focused":false}}}"#;
        assert!(!super::parse_focus_result(raw).unwrap());
    }

    #[test]
    fn parse_focus_result_returns_err_on_error() {
        let raw = r#"{"id":"3","error":{"code":"not_found"}}"#;
        assert!(super::parse_focus_result(raw).is_err());
    }

    #[test]
    fn focus_pane_classifies_focused_response() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("focus.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            stream
                .write_all(
                    b"{\"id\":\"poke-focus\",\"result\":{\"type\":\"agent_info\",\"agent\":{\"pane_id\":\"w1:p1\",\"focused\":true}}}\n",
                )
                .unwrap();
        });
        let out = super::focus_pane(&sock, "w1:p1");
        handler.join().unwrap();
        assert_eq!(out, super::HerdrFocusOutcome::Focused);
    }

    #[test]
    fn focus_pane_classifies_not_focused_response() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("focus2.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            stream
                .write_all(
                    b"{\"id\":\"poke-focus\",\"result\":{\"type\":\"agent_info\",\"agent\":{\"pane_id\":\"w1:p1\",\"focused\":false}}}\n",
                )
                .unwrap();
        });
        let out = super::focus_pane(&sock, "w1:p1");
        handler.join().unwrap();
        assert_eq!(out, super::HerdrFocusOutcome::NotFocused);
    }

    #[test]
    fn focus_pane_classifies_error_envelope() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("focus3.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handler = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            stream
                .write_all(b"{\"id\":\"poke-focus\",\"error\":{\"code\":\"not_found\"}}\n")
                .unwrap();
        });
        let out = super::focus_pane(&sock, "w1:p1");
        handler.join().unwrap();
        assert_eq!(out, super::HerdrFocusOutcome::ExplicitError);
    }

    #[test]
    fn focus_pane_classifies_transport_failure_when_socket_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("missing-focus.sock");
        let out = super::focus_pane(&sock, "w1:p1");
        assert_eq!(out, super::HerdrFocusOutcome::TransportFailure);
    }

    #[test]
    fn named_session_from_socket_path_default() {
        let p = std::path::Path::new("/Users/me/.config/herdr/herdr.sock");
        assert_eq!(super::named_session_from_socket_path(p), None);
    }

    #[test]
    fn named_session_from_socket_path_work() {
        let p = std::path::Path::new("/Users/me/.config/herdr/sessions/work/herdr.sock");
        assert_eq!(
            super::named_session_from_socket_path(p).as_deref(),
            Some("work")
        );
    }

    #[test]
    fn ghostty_tab_title_matches_default_session() {
        assert!(super::ghostty_tab_title_matches_named_session(
            "herdr", None
        ));
        assert!(super::ghostty_tab_title_matches_named_session(
            "herdr --session default",
            None
        ));
        assert!(!super::ghostty_tab_title_matches_named_session(
            "herdr --session work",
            None
        ));
    }

    #[test]
    fn ghostty_tab_title_matches_named_work_session() {
        assert!(super::ghostty_tab_title_matches_named_session(
            "herdr --session work",
            Some("work")
        ));
        assert!(!super::ghostty_tab_title_matches_named_session(
            "herdr",
            Some("work")
        ));
        assert!(!super::ghostty_tab_title_matches_named_session(
            "herdr --session default",
            Some("work")
        ));
    }

    #[test]
    fn parse_workspace_focus_result_reads_focused_flag() {
        let raw = r#"{"id":"1","result":{"type":"workspace_info","workspace":{"workspace_id":"w3","focused":true}}}"#;
        assert!(super::parse_workspace_focus_result(raw).unwrap());
    }

    #[test]
    fn parse_workspace_focus_result_false_when_not_focused() {
        let raw = r#"{"id":"1","result":{"type":"workspace_info","workspace":{"workspace_id":"w3","focused":false}}}"#;
        assert!(!super::parse_workspace_focus_result(raw).unwrap());
    }

    #[test]
    fn parse_herdr_remote_socket_lines_matches_target() {
        let text = r#"COMMAND     PID  USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
herdr     90871  edy    3u  unix 0x465f85e1ef90fd5e      0t0        0t0 /var/folders/61/x/T/herdr-remote-90871-hello-default.sock
herdr     91031  edy    4u  unix 0x1c740891f4fa3cc4      0t0        0t0 ->0x5da2a7bc4580f922
ssh      91470  edy   10u  unix 0x7ecefceb77dbb909      0t0        0t0 /Users/edy/.pokepoke/herdr-remote-hello.sock
"#;
        let m = super::parse_herdr_remote_socket_lines(text, "hello");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, 90871);
        assert!(m[0].1.contains("herdr-remote-90871-hello-default.sock"));
    }

    #[test]
    fn parse_herdr_remote_socket_lines_ignores_other_targets() {
        let text = r#"herdr 90871 edy 3u unix 0x1 0t0 /var/folders/x/herdr-remote-90871-hello-default.sock
herdr 77777 edy 3u unix 0x2 0t0 /var/folders/x/herdr-remote-77777-work-default.sock
"#;
        assert!(super::parse_herdr_remote_socket_lines(text, "hello").len() == 1);
        assert!(super::parse_herdr_remote_socket_lines(text, "work").len() == 1);
        assert!(super::parse_herdr_remote_socket_lines(text, "other").is_empty());
    }
}

#[cfg(test)]
mod real_tests {
    use super::*;
    #[test]
    #[ignore] // 需要本机跑着 herdr,只手工触发
    fn real_socket_smoke() {
        let p = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join(".config/herdr/herdr.sock");
        if !p.exists() {
            eprintln!("no herdr running, skip");
            return;
        }
        let ping = ping(&p);
        eprintln!("ping = {:?}", ping);
        assert!(ping.is_some(), "herdr running but ping failed");
        let panes = list_agent_panes(&p);
        eprintln!("{} panes", panes.len());
    }
}
