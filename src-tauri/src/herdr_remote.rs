//! herdr_remote.rs — PokePoke 远程 herdr 监视器。
//!
//! brief `20260804-herdr-remote-notification`(2026-08-07 修订:路径 B 变体)。
//! 用户期望:公司机(正在使用的机器)装 PokePoke、远程机零安装、要 PokePoke popup
//! 不要 herdr toast。实现方式:复用 herdr 对外公开的 socket API(`agent.list` /
//! `events.subscribe`)——herdr 界面显示的 agent 状态就是这个 API 的数据,远程模式
//! 下 PokePoke 走同一条公开 API(经 SSH unix socket 转发),拿到的就是同一份状态。
//!
//! 两种连接模式(自动发现,零配置):
//! - **Local**:本地 herdr server socket(`~/.config/herdr/herdr.sock`)直连。
//! - **Remote**:复用 `herdr --remote` 已建立的 SSH control socket
//!   (`/tmp/herdr-ssh-*/ctl`),用 `ssh -S <ctl> -O forward -L <local>:<abs-remote>
//!   <target>` 在**已有连接**上加 unix socket 转发——不新建 SSH 连接、不重复认证。
//!   远程 socket 绝对路径先经 `ssh -S <ctl> <target> 'echo $HOME'` 探测(control
//!   master 复用跑远程命令,实测可用)。`~` 相对路径转发实测不通,必须绝对路径。
//!
//! 状态映射(herdr 语义状态 → PokePoke):
//! ```text
//!   working → running   (不弹,进列表)
//!   blocked → pending   (弹「需要你的注意」+ request 音)
//!   done    → idle      (弹「干完了」+ done 音)
//!   idle    → idle      (弹「干完了」+ done 音,与本地 hook 的每轮 idle 一致)
//!   unknown / agent 记录消失 → 不喂;消失连续 3 轮确认 → session_end 清理
//!
//! 首次观测例外(brief 20260810-herdr-remote-first-observation-silent):监视开始
//! 前就已 done/idle 的 pane **不喂**——回溯状态不是「进入 idle」迁移,弹「干完了」
//! 是噪音(用户实测:herdr --remote 刚连上即弹 + app 重启后重弹,根因同一)。
//! blocked 首观测仍喂(刚连上就有 agent 等输入,正是远程通知的用途);working 首
//! 观测仍喂(running 不弹,静默建 session)。真实迁移(working → idle 等)不变。
//! ```
//!
//! focused 语义:herdr 报 `focused=true`(herdr client 当前聚焦那个 pane)。
//! **2026-08-07 用户实测反馈后取消抑制**:远程场景聚焦 ≠ 人在看(herdr client 渲染
//! 在公司机,用户可能正干别的),抑制会把 blocked 吞掉漏弹;弹窗有 cooldown/ack
//! 兜底防重复,故所有状态变化一律喂。
//!
//! 翻译产物是 `NotifyRequest`,喂进既有 `http_server::process_notify` —— 完整复用
//! 本地状态机 / popup gate(already_focused / cooldown / acked / quiet_mode)/
//! popup UI / 声音。远程 session 的 task_id 用 `hr-<pane_id>` 前缀,与本地
//! (cc-/cdx-/cursor-/pi- 等)不冲突。
//!
//! `source` 按连接区分(brief 20260810-herdr-remote-source-mislabel):
//! - **Local** → `"herdr"`:本地 herdr 监视,聚焦抑制走 pane 级
//!   (`popup/focus.rs::is_herdr_bound_session_focused` 完整 outer+pane 判定),
//!   点击回跳走段 A 本地宿主锚定。
//! - **Remote** → `"herdr-remote"`:SSH 转发到远程 herdr,段 A 短路 + 永不判聚焦
//!   (20260807 产品决策:远程聚焦 ≠ 人在看)。
//! 之前 Local 也标 "herdr-remote",导致本地 herdr session 被两处 herdr-remote
//! 例外(段 A 短路 / 永不聚焦抑制)误伤。
//!
//! 已知边界(brief Notes):tmux 式 `ssh host && herdr`(herdr 整个跑在远程)本地
//! 没有通路可复用,自动发现不到,静默跳过——不覆盖。

use crate::herdr::{ping, send_once, HerdrAgentStatus};
use crate::http_server::{process_notify, AppState, NotifyRequest};
use crate::log_debug;
use crate::log_info;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// agent 从 `agent.list` 消失连续多少轮后才发 session_end(防 herdr 偶发丢数据误清)。
const DISAPPEAR_THRESHOLD: u32 = 3;
/// 远程转发 socket 前缀(放 PokePoke 数据目录,随 target 区分)。
const FORWARD_PREFIX: &str = "herdr-remote-";

/// herdr `agent.list` 的 agent 记录(只取本模块关心的字段,未知字段忽略)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAgent {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: Option<String>,
    pub status: HerdrAgentStatus,
    pub cwd: String,
    pub foreground_cwd: String,
    pub focused: bool,
}

/// 已建立的连接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    Local(PathBuf),
    Remote {
        /// 转发到本地的 socket 路径(供 socket API 调用)。
        local_socket: PathBuf,
        target: String,
        control: PathBuf,
        remote_socket: PathBuf,
    },
}

impl Connection {
    pub fn socket_path(&self) -> &Path {
        match self {
            Connection::Local(p) => p,
            Connection::Remote { local_socket, .. } => local_socket,
        }
    }

    fn describe(&self) -> String {
        match self {
            Connection::Local(p) => format!("local:{}", p.display()),
            Connection::Remote { target, .. } => format!("remote:{target}"),
        }
    }
}

// ==================== agent.list 解析 ====================

#[derive(serde::Deserialize)]
struct AgentListEnvelope {
    #[serde(default)]
    result: AgentListResult,
}

#[derive(serde::Deserialize, Default)]
struct AgentListResult {
    #[serde(default)]
    agents: Vec<RawAgent>,
}

#[derive(serde::Deserialize)]
struct RawAgent {
    #[serde(default)]
    pane_id: String,
    #[serde(default)]
    workspace_id: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    agent_status: HerdrAgentStatus,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    foreground_cwd: String,
    #[serde(default)]
    focused: bool,
}

pub(crate) fn parse_agent_list(raw: &str) -> Result<Vec<RemoteAgent>, String> {
    let env: AgentListEnvelope =
        serde_json::from_str(raw).map_err(|e| format!("parse agent.list: {e}"))?;
    Ok(env
        .result
        .agents
        .into_iter()
        .filter(|a| !a.pane_id.is_empty())
        .map(|a| RemoteAgent {
            pane_id: a.pane_id,
            workspace_id: a.workspace_id,
            agent: a.agent,
            status: a.agent_status,
            cwd: a.cwd,
            foreground_cwd: a.foreground_cwd,
            focused: a.focused,
        })
        .collect())
}

/// 通过连接 socket 拉一次 `agent.list`。失败(连接断了 / 超时)返回 None。
pub fn list_agents(socket: &Path) -> Option<Vec<RemoteAgent>> {
    let req = crate::herdr::build_request("hr-list", "agent.list", serde_json::json!({}));
    let raw = send_once(socket, &req)?;
    parse_agent_list(&raw).ok()
}

// ==================== 发现与连接 ====================

fn local_socket_candidate() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/herdr/herdr.sock"))
}

/// 扫描 herdr `--remote` 的 SSH control socket(`/tmp/herdr-ssh-*/ctl`)。
/// herdr 源码 `create_remote_ssh_config_dir` 用 `temp_dir()` + `/tmp` 两个 base。
/// 从 `ps` 进程表里找 `herdr --remote <target>` 进程,提取 (pid, target)。
/// 这是发现远程连接最直接的信号:herdr client 进程带 target,control socket
/// 目录名带 client pid(`herdr-ssh-<pid>-<attempt>/ctl`,herdr 源码
/// `create_remote_ssh_config_dir` 用进程 pid)。
fn find_remote_clients() -> Vec<(String, String)> {
    let out = Command::new("ps")
        .args(["-axo", "pid,command"])
        .output()
        .ok();
    let Some(out) = out else {
        return Vec::new();
    };
    remote_clients_from_ps_text(&String::from_utf8_lossy(&out.stdout))
}

/// 纯函数:`ps -axo pid,command` 输出 → [(pid, target)]。ps 右对齐 pid 列,
/// max pid 越过 4 位后行带前导空格(2026-08-18 实测 " 8748 ..."),旧 splitn
/// 整行跳过 → 发现链全灭;必须 trim_start 后再切分。
fn remote_clients_from_ps_text(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in text.lines() {
        let Some((pid, rest)) = line.trim_start().split_once(char::is_whitespace) else {
            continue;
        };
        if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // `herdr --remote <target>` 或 `herdr --remote=<target>`
        let trimmed = rest.trim_start();
        let target = if let Some(idx) = trimmed.find("--remote") {
            let after = &trimmed[idx + "--remote".len()..];
            if let Some(eq) = after.strip_prefix('=') {
                eq.split_whitespace().next().map(str::to_string)
            } else if let Some(space) = after.strip_prefix(' ') {
                space.split_whitespace().next().map(str::to_string)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(t) = target {
            if !t.is_empty() && t != "--" {
                found.push((pid.to_string(), t));
            }
        }
    }
    found
}

/// 从 `ps` 进程表里找用这个 control socket 的 ssh 进程,提取目标 host。
/// 注意:herdr 的 control master 进程标题是 `ssh: <ctl> [mux]`(无 target),
/// 这里只处理带 `-T <target>` 的普通 ssh 连接进程(herdr 引导/探测时短暂存在)。
/// 主路径已由 `find_remote_clients`(herdr --remote 进程)覆盖。
/// 扫 herdr `--remote` 的本地桥接端点(`herdr-remote-<pid>-<target>-<session>.sock`)。
/// 文件名带 pid 与 sanitized target,是发现远程连接最确定性的信号
/// (herdr 源码 `local_forward_socket_path`,放 temp_dir() 或 /tmp)。
/// 从桥接端点文件名解析 (pid, target):`herdr-remote-<pid>-<target>-<session>.sock`。
/// target 是 sanitize 过的(别名场景无损;ssh:// URL 场景会被 mangle,由 ps 兜底)。
/// 由桥接端点 pid 反查 control socket:`/tmp/herdr-ssh-<pid>-<attempt>/ctl`。
/// 由 herdr client pid 反查 control socket:`herdr-ssh-<pid>-<attempt>/ctl`。
/// herdr 的临时目录在 `temp_dir()`(macOS /var/folders/...)或 /tmp(路径超长时),
/// 两个都要查。
/// 探测远程 `$HOME`(control master 复用,不重新认证),拼远程 herdr.sock 绝对路径。
/// 实测 `~` 相对路径的 unix socket 转发不通,必须绝对路径。
fn remote_socket_abs(control: &Path, target: &str) -> Option<PathBuf> {
    let home = Command::new("ssh")
        .arg("-o")
        .arg("ConnectTimeout=2")
        .arg("-S")
        .arg(control)
        .arg(target)
        .arg("echo $HOME")
        .output()
        .ok()?;
    if !home.status.success() {
        return None;
    }
    let home = String::from_utf8_lossy(&home.stdout).trim().to_string();
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config/herdr/herdr.sock"))
}

fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(48)
        .collect()
}

fn forward_socket_path(target: &str) -> PathBuf {
    let data_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".pokepoke");
    data_dir.join(format!(
        "{FORWARD_PREFIX}{}.sock",
        sanitize_component(target)
    ))
}

/// 在 herdr 已有 SSH control master 上加一条 unix socket 转发(复用连接)。
/// 在 PokePoke **自建**的 SSH control master 上加一条 unix socket 转发。
/// 不复用 herdr 的 master:herdr --remote 的 master 带远程命令
/// (`exec herdr remote-client-bridge`),实测不支持新增转发(-O forward 与
/// -N -L 都 rc=0 但不建 socket)。herdr 连接只当发现信号,传输走自己的连接。
fn forward_remote_socket(control: &Path, target: &str, remote_socket: &Path) -> Option<PathBuf> {
    let local = forward_socket_path(target);
    // 现有转发 socket 还活着就直接复用,不重建——monitor 每 10s RESCAN 都会调
    // discover_all,若每次都删了重建会破坏 master 上的转发条目(实测 connection lost)。
    if std::os::unix::net::UnixStream::connect(&local).is_ok() {
        return Some(local);
    }
    // 重建前:先取消 master 上同 spec 的旧转发条目(忽略失败,防条目冲突)。
    let spec = format!("{}:{}", local.display(), remote_socket.display());
    let _ = Command::new("ssh")
        .arg("-o")
        .arg("ConnectTimeout=2")
        .arg("-S")
        .arg(control)
        .arg("-O")
        .arg("cancel")
        .arg("-L")
        .arg(&spec)
        .arg(target)
        .output();
    // 上次遗留的转发 socket 文件会挡 -O forward,先清。
    let _ = std::fs::remove_file(&local);
    let out = Command::new("ssh")
        .arg("-o")
        .arg("ConnectTimeout=2")
        .arg("-S")
        .arg(control)
        .arg("-O")
        .arg("forward")
        .arg("-L")
        .arg(&spec)
        .arg(target)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // -O forward 是异步建立的:等本地 socket 可连(最多 ~1s),否则 ping 会撞未就绪。
    let deadline = std::time::Instant::now() + Duration::from_millis(1000);
    while std::time::Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(&local).is_ok() {
            return Some(local);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// PokePoke 自建 master 的 control socket 路径(`~/.pokepoke/herdr-master-<target>.sock`)。
fn master_socket_path(target: &str) -> PathBuf {
    let data_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".pokepoke");
    data_dir.join(format!("herdr-master-{}.sock", sanitize_component(target)))
}

/// 确保 PokePoke 自建 SSH control master 存活;返回 master socket 路径。
/// 首次建 master 走用户 ssh config + ssh-agent 认证(与 herdr --remote 同链路);
/// ControlPersist=600 保证断开后 10 分钟内可复用、PokePoke 退出后自动清理。
fn ensure_master(target: &str) -> Option<PathBuf> {
    let master = master_socket_path(target);
    // 已活直接复用
    let check = Command::new("ssh")
        .arg("-o")
        .arg("ConnectTimeout=2")
        .arg("-S")
        .arg(&master)
        .arg("-O")
        .arg("check")
        .arg(target)
        .output()
        .ok();
    if check.as_ref().is_some_and(|o| o.status.success()) {
        return Some(master);
    }
    // 建 master(-N 无交互;后台进程由 ControlPersist 自清理)。BatchMode=yes +
    // stdin 不继承:ControlPersist=600 过期后 master 重建时,若 ssh-agent 已失效,
    // 直接失败返回而非卡在密码提示挂起 monitor 线程(brief 20260807 用户确认)。
    let spawn = Command::new("ssh")
        .arg("-N")
        .arg("-M")
        .arg("-S")
        .arg(&master)
        .arg("-o")
        .arg("ControlPersist=600")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    drop(spawn); // 子进程继续跑(孤儿),由 ControlPersist 自清理
                 // 等待 master 就绪(最多 8s):轮询 -O check
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        let ok = Command::new("ssh")
            .arg("-o")
            .arg("ConnectTimeout=2")
            .arg("-S")
            .arg(&master)
            .arg("-O")
            .arg("check")
            .arg(target)
            .output()
            .ok()
            .is_some_and(|o| o.status.success());
        if ok {
            return Some(master);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

/// 自动发现:本地 herdr socket 直连,否则复用 `herdr --remote` 的 SSH 连接。
/// 自动发现**所有**可用连接:本地 herdr socket + 每个 `herdr --remote` 远程连接。
/// 用户机器上本地 herdr server 可能常驻,**同时**开着 `herdr --remote`——两个都要
/// 监视(本地 agent + 远程 agent),不能像旧实现那样本地优先只返回一个。
pub fn discover_all() -> Vec<Connection> {
    let mut out = Vec::new();

    // 1) 本地模式
    if let Some(p) = local_socket_candidate() {
        if ping(&p).is_some() {
            out.push(Connection::Local(p));
        }
    }
    // 2) 远程模式:herdr --remote 进程是发现信号(target),传输走 PokePoke 自建
    //    SSH control master(herdr 的 master 不支持新增转发,见 forward_remote_socket)。
    let mut seen_targets: HashSet<String> = HashSet::new();
    for (_, target) in find_remote_clients() {
        if !seen_targets.insert(target.clone()) {
            continue;
        }
        let Some(master) = ensure_master(&target) else {
            continue;
        };
        let Some(remote_socket) = remote_socket_abs(&master, &target) else {
            continue;
        };
        let Some(local) = forward_remote_socket(&master, &target, &remote_socket) else {
            continue;
        };
        if ping(&local).is_some() {
            out.push(Connection::Remote {
                local_socket: local,
                target,
                control: master,
                remote_socket,
            });
        }
    }
    out
}

// ==================== 翻译 → NotifyRequest ====================

fn project_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cwd.to_string())
}

/// herdr-remote session 的 task_id:`hr-<sanitized target>-<pane_id>`。
/// 含 target 避免两台远程主机同名 pane(如 `w1:p1`)在 session store 里互相覆盖
/// (brief 20260807-herdr-remote-jumpback-fix,grok review);本地连接(无 target)
/// 保持 `hr-<pane_id>` 既有格式。
fn remote_task_id(conn: &Connection, pane_id: &str) -> String {
    match conn {
        Connection::Remote { target, .. } => {
            let t = sanitize_component(target);
            if t.is_empty() {
                format!("hr-{}", pane_id)
            } else {
                format!("hr-{}-{}", t, pane_id)
            }
        }
        Connection::Local(_) => format!("hr-{}", pane_id),
    }
}

/// herdr 状态 → NotifyRequest。`unknown` 无法判定,返回 None(不喂)。
fn notify_request(agent: &RemoteAgent, conn: &Connection) -> Option<NotifyRequest> {
    let agent_name = agent.agent.as_deref().unwrap_or("agent");
    let title = project_name(&agent.cwd);
    let (event_type, status, message) = match agent.status {
        HerdrAgentStatus::Working => (None, Some("running"), format!("{agent_name} 工作中")),
        HerdrAgentStatus::Blocked => (Some("pending"), None, format!("{agent_name} 需要你的注意")),
        HerdrAgentStatus::Done | HerdrAgentStatus::Idle => {
            (None, Some("idle"), format!("{agent_name} 干完了"))
        }
        HerdrAgentStatus::Unknown => return None,
    };
    let event_type = event_type.map(str::to_string);
    let status = status.map(str::to_string);
    // brief 20260810-herdr-remote-source-mislabel:Local 标 "herdr"(本地宿主,
    // 段 A 锚定 + pane 级聚焦抑制),Remote 标 "herdr-remote"(段 A 短路 + 永不判
    // 聚焦)。此前 Local 也标 "herdr-remote",两条例外链误伤本地 herdr。
    let source = match conn {
        Connection::Local(_) => Some("herdr".to_string()),
        Connection::Remote { .. } => Some("herdr-remote".to_string()),
    };
    Some(NotifyRequest {
        protocol_version: Some(1), // W9:wire 协议版本
        title,
        message,
        task_id: remote_task_id(conn, &agent.pane_id),
        external_session_id: None,
        source,
        priority: None,
        event_type,
        status,
        terminal_tty: None,
        workspace_path: Some(agent.cwd.clone()),
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
        transcript_path: None,
        herdr_pane_id: Some(agent.pane_id.clone()),
        herdr_workspace_id: Some(agent.workspace_id.clone()),
        herdr_socket_path: Some(conn.socket_path().to_string_lossy().into_owned()),
        cmux_surface_id: None,
        cmux_workspace_id: None,
        wezterm_pane_id: None,
        wezterm_unix_socket: None,
        wezterm_executable: None,
        otty_pane_id: None,
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    })
}

fn session_end_request(task_id: &str) -> NotifyRequest {
    NotifyRequest {
        title: String::new(),
        message: String::new(),
        task_id: task_id.to_string(),
        protocol_version: Some(1), // W9:wire 协议版本
        external_session_id: None,
        // 与 notify_request 的 Local/Remote 区分无关紧要(session_end 不参与
        // popup gate),但保持 source 语义一致,避免后续排查混淆。
        source: Some("herdr".into()),
        priority: None,
        event_type: Some("session_end".into()),
        status: None,
        terminal_tty: None,
        workspace_path: None,
        failure_reason: None,
        aborted: None,
        terminal_app: None,
        dsh_nav_socket: None,
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
        tmux_pane_id: None,
        tmux_socket_path: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        orca_tab_id: None,
    }
}

/// 喂一条 NotifyRequest 进既有管线。失败只记日志(process_notify 总是返回成功
/// 状态码,这里防御性吞错)。
fn feed(state: &AppState, req: NotifyRequest) {
    process_notify(state, req);
}

// ==================== 轮询 diff 与状态机 ====================

/// 每个远程 pane 的本地跟踪状态。
#[derive(Debug, Clone)]
pub struct Known {
    /// 最近观测到的 herdr 状态。
    observed: HerdrAgentStatus,
    /// 最近一次喂给 PokePoke 的状态(None = 还没喂过;首观测 done/idle 跳过喂时
    /// 用 Some 标记"已对齐",防止下一轮把回溯状态当成状态变化再喂)。
    reported: Option<HerdrAgentStatus>,
    /// 连续消失轮数。
    disappear_count: u32,
}

fn feed_for(agent: &RemoteAgent, conn: &Connection, state: &AppState) {
    if let Some(req) = notify_request(agent, conn) {
        feed(state, req);
    }
}

#[path = "herdr_remote/agent_capability.rs"]
mod agent_capability;

use agent_capability::is_hook_capable_agent;

/// 一轮轮询:diff agent.list 与本地跟踪,喂状态变化 / 补喂 / 清理消失。
/// pub 供 `tests/http_integration.rs` 集成测试直接驱动。
pub fn poll_once(
    state: &AppState,
    conn: &Connection,
    known: &mut HashMap<String, Known>,
    agents: &[RemoteAgent],
) {
    let mut seen: HashSet<&str> = HashSet::new();

    for agent in agents {
        seen.insert(agent.pane_id.as_str());
        // brief 20260813-herdr-triple-popup Phase 2:本地 pane 跑的是自带 hook 的
        // 八端 agent(名单/判定见 herdr_remote/agent_capability.rs:cc / codex /
        // cursor / opencode / pi / omp / kimi / grok),该 pane 的会话已由本地
        // hook 通路以更高保真度接管(title/message/tty/jump 绑定都在 agent
        // session 上),pane 级任务再喂会把同一次完成弹两次(2026-08-13 用户实测
        // pi + hr-<pane> 双弹)并在面板留重复条目。仍维护 known 对齐(disappear
        // 计数照常),但永不 feed。Remote 连接不跳过:远程 agent 的 hook 打不到
        // 本机,pane 级监视是唯一通道(brief 20260804「本地走 hook / 远程走监视」分工)。
        if matches!(conn, Connection::Local(_)) && is_hook_capable_agent(agent.agent.as_deref()) {
            match known.entry(agent.pane_id.clone()) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    log_debug!(
                        "herdr-remote",
                        "skip local hook-capable pane={} agent={:?} status={:?}",
                        agent.pane_id,
                        agent.agent,
                        agent.status,
                    );
                    v.insert(Known {
                        observed: agent.status,
                        reported: Some(agent.status),
                        disappear_count: 0,
                    });
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    let k = o.get_mut();
                    k.observed = agent.status;
                    k.reported = Some(agent.status);
                    k.disappear_count = 0;
                }
            }
            continue;
        }
        let entry = known.entry(agent.pane_id.clone());
        match entry {
            std::collections::hash_map::Entry::Vacant(v) => {
                // 首次观测(监视开始前就存在 / app 重启后重新发现):
                // - Working:照喂 running(不弹,静默建 session,panel 可见)
                // - Blocked:照喂 pending(刚连上就有 agent 等输入,弹「需要你的注意」)
                // - Done/Idle/Unknown:不喂——回溯状态不是「进入 idle」迁移,弹「干完了」
                //   是噪音(brief 20260810:herdr --remote 刚连上即弹 + 重启后重弹);
                //   只标记已对齐,后续真实状态迁移(working→idle 等)再正常喂。
                if matches!(
                    agent.status,
                    HerdrAgentStatus::Working | HerdrAgentStatus::Blocked
                ) {
                    feed_for(agent, conn, state);
                }
                v.insert(Known {
                    observed: agent.status,
                    reported: Some(agent.status),
                    disappear_count: 0,
                });
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let k = o.get_mut();
                k.observed = agent.status;
                // 2026-08-07 用户实测反馈:远程场景「聚焦」≠ 人在看(herdr client
                // 渲染在公司机,用户可能正干别的),focused 抑制会把 blocked 吞掉,
                // 漏弹。改为一律喂状态变化(弹窗有 cooldown/ack 兜底防重复)。
                if k.reported != Some(agent.status) {
                    feed_for(agent, conn, state);
                    k.reported = Some(agent.status);
                }
                k.disappear_count = 0;
            }
        }
    }

    // 消失清理:连续 N 轮不在列表 → session_end。
    let mut to_remove: Vec<String> = Vec::new();
    for (pane_id, k) in known.iter_mut() {
        if seen.contains(pane_id.as_str()) {
            continue;
        }
        k.disappear_count += 1;
        if k.disappear_count >= DISAPPEAR_THRESHOLD {
            to_remove.push(pane_id.clone());
        }
    }
    for pane_id in to_remove {
        let task_id = remote_task_id(conn, &pane_id);
        log_info!(
            "herdr-remote",
            "agent disappeared, session_end task_id={}",
            task_id
        );
        feed(state, session_end_request(&task_id));
        known.remove(&pane_id);
    }
}

// ==================== 监视线程 ====================

/// 启动远程 herdr 监视线程(PokePoke setup 阶段调用一次)。
/// W3(brief 20260813-watcher-supervisor):迁移到 supervisor::spawn_loop_guarded
/// (命名线程 + catch_unwind),保留原 sleep/退避结构;loop 开头 heartbeat,
/// 其余 body 逻辑零变化。
pub fn spawn_monitor(state: AppState) {
    crate::supervisor::spawn_loop_guarded("herdr-remote-monitor", POLL_INTERVAL, move |entry| {
        monitor_loop(&state, entry);
    });
}

/// 连接发现后的轮询间隔 / 重扫间隔。
const RESCAN_INTERVAL: Duration = Duration::from_secs(10);

fn monitor_loop(state: &AppState, entry: &std::sync::Arc<crate::supervisor::WatcherEntry>) {
    // 连接缓存:本地 + 每个远程;key = socket_path(本地与远程 forward socket 路径
    // 天然不同,不会撞)。poll 失败的连接剔除,下次 RESCAN 重新发现重建。
    let mut conns: Vec<Connection> = Vec::new();
    let mut knowns: HashMap<String, HashMap<String, Known>> = HashMap::new();
    let mut last_rescan = Instant::now();

    loop {
        crate::supervisor::heartbeat(entry);
        // 定期重扫:新出现的 herdr --remote 连接自动纳入;已剔除的重新发现。
        if last_rescan.elapsed() >= RESCAN_INTERVAL {
            last_rescan = Instant::now();
            let fresh = discover_all();
            let existing: HashSet<String> = conns
                .iter()
                .map(|c| c.socket_path().to_string_lossy().into_owned())
                .collect();
            for c in fresh {
                let key = c.socket_path().to_string_lossy().into_owned();
                if !existing.contains(&key) {
                    log_info!(
                        "herdr-remote",
                        "discovered {} socket={}",
                        c.describe(),
                        c.socket_path().display()
                    );
                    conns.push(c);
                }
            }
        }

        // 对每个连接轮询一次;失败剔除(下次 RESCAN 重建)。
        let mut dead: Vec<String> = Vec::new();
        for conn in &conns {
            let key = conn.socket_path().to_string_lossy().into_owned();
            match list_agents(conn.socket_path()) {
                Some(agents) => {
                    poll_once(state, conn, knowns.entry(key).or_default(), &agents);
                }
                None => {
                    log_info!(
                        "herdr-remote",
                        "connection lost {} socket={}",
                        conn.describe(),
                        conn.socket_path().display()
                    );
                    dead.push(key);
                }
            }
        }
        if !dead.is_empty() {
            conns.retain(|c| {
                let k = c.socket_path().to_string_lossy().into_owned();
                !dead.contains(&k)
            });
            // 立刻重扫,把掉线的连接重新拉起来(而不等 10s)。
            last_rescan = Instant::now() - RESCAN_INTERVAL;
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_list_extracts_status_and_focus() {
        let raw = r#"{"id":"1","result":{"type":"agent_list","agents":[
            {"terminal_id":"t1","agent":"pi","agent_status":"blocked","workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p1","focused":true,"cwd":"/a","foreground_cwd":"/a"},
            {"terminal_id":"t2","agent":"claude","agent_status":"working","workspace_id":"w1","tab_id":"w1:t2","pane_id":"w1:p2","focused":false,"cwd":"/b","foreground_cwd":"/b"}
        ]}}"#;
        let agents = parse_agent_list(raw).expect("agent list JSON 应可解析");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].status, HerdrAgentStatus::Blocked);
        assert!(agents[0].focused);
        assert_eq!(agents[1].status, HerdrAgentStatus::Working);
        assert!(!agents[1].focused);
    }

    #[test]
    fn parse_agent_list_drops_unknown_status_and_missing_pane() {
        let raw = r#"{"id":"1","result":{"type":"agent_list","agents":[
            {"agent":"pi","agent_status":"super_new","pane_id":"w1:p1"},
            {"agent":"x","agent_status":"idle"}
        ]}}"#;
        let agents = parse_agent_list(raw).expect("agent list JSON 应可解析");
        // 未知状态归 Unknown,缺 pane_id 的整条丢弃。
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].status, HerdrAgentStatus::Unknown);
    }

    #[test]
    fn notify_request_maps_statuses() {
        let conn = Connection::Local(PathBuf::from("/tmp/fake.sock"));
        let mk = |pane: &str, status: HerdrAgentStatus, focused: bool| RemoteAgent {
            pane_id: pane.into(),
            workspace_id: "w1".into(),
            agent: Some("claude".into()),
            status,
            cwd: "/Users/x/MyProject".into(),
            foreground_cwd: "/Users/x/MyProject".into(),
            focused,
        };

        let working = notify_request(&mk("w1:p1", HerdrAgentStatus::Working, false), &conn)
            .expect("notify_request 应成功");
        assert_eq!(working.status.as_deref(), Some("running"));
        assert_eq!(working.event_type, None);

        let blocked = notify_request(&mk("w1:p1", HerdrAgentStatus::Blocked, false), &conn)
            .expect("notify_request 应成功");
        assert_eq!(blocked.event_type.as_deref(), Some("pending"));
        assert!(blocked.message.contains("需要你的注意"));

        let done = notify_request(&mk("w1:p1", HerdrAgentStatus::Done, false), &conn)
            .expect("notify_request 应成功");
        assert_eq!(done.status.as_deref(), Some("idle"));
        assert!(done.message.contains("干完了"));

        let idle = notify_request(&mk("w1:p1", HerdrAgentStatus::Idle, false), &conn)
            .expect("notify_request 应成功");
        assert_eq!(idle.status.as_deref(), Some("idle"));

        assert!(notify_request(&mk("w1:p1", HerdrAgentStatus::Unknown, false), &conn).is_none());

        // task_id 用 hr- 前缀 + pane_id,与本地通道不冲突
        assert_eq!(working.task_id, "hr-w1:p1");
        assert_eq!(working.herdr_pane_id.as_deref(), Some("w1:p1"));
        // brief 20260810-herdr-remote-source-mislabel:Local 连接标 "herdr"
        // (本地宿主锚定 + pane 级聚焦抑制),不标 "herdr-remote"。
        assert_eq!(working.source.as_deref(), Some("herdr"));
        assert_eq!(working.title, "MyProject");
    }

    #[test]
    fn remote_task_id_includes_target_to_avoid_cross_host_collision() {
        // brief 20260807-herdr-remote-jumpback-fix(grok review):两台远程主机各有
        // w1:p1 时,hr-<pane_id> 会在 session store 里互相覆盖;含 target 后唯一。
        use std::path::PathBuf;
        let mk = |pane: &str, status: HerdrAgentStatus, focused: bool| RemoteAgent {
            pane_id: pane.into(),
            workspace_id: "w1".into(),
            agent: Some("claude".into()),
            status,
            cwd: "/Users/x/MyProject".into(),
            foreground_cwd: "/Users/x/MyProject".into(),
            focused,
        };
        let mk_conn = |target: &str| Connection::Remote {
            local_socket: PathBuf::from("/tmp/fwd.sock"),
            target: target.into(),
            control: PathBuf::from("/tmp/ctl"),
            remote_socket: PathBuf::from("/Users/x/.config/herdr/herdr.sock"),
        };
        let a = notify_request(
            &mk("w1:p1", HerdrAgentStatus::Working, false),
            &mk_conn("hello"),
        )
        .expect("notify_request 应成功");
        let b = notify_request(
            &mk("w1:p1", HerdrAgentStatus::Working, false),
            &mk_conn("other"),
        )
        .expect("notify_request 应成功");
        assert_eq!(a.task_id, "hr-hello-w1:p1");
        assert_eq!(b.task_id, "hr-other-w1:p1");
        assert_ne!(a.task_id, b.task_id, "跨主机同 pane 的 task_id 必须不撞");
        // Remote 连接保持 "herdr-remote"(段 A 短路 + 永不判聚焦的例外链只给真远程)。
        assert_eq!(a.source.as_deref(), Some("herdr-remote"));
        assert_eq!(b.source.as_deref(), Some("herdr-remote"));
    }

    #[test]
    fn remote_task_id_falls_back_to_pane_only_when_target_sanitizes_empty() {
        use std::path::PathBuf;
        let mk = |pane: &str, status: HerdrAgentStatus, focused: bool| RemoteAgent {
            pane_id: pane.into(),
            workspace_id: "w1".into(),
            agent: Some("claude".into()),
            status,
            cwd: "/Users/x/MyProject".into(),
            foreground_cwd: "/Users/x/MyProject".into(),
            focused,
        };
        let conn = Connection::Remote {
            local_socket: PathBuf::from("/tmp/fwd.sock"),
            target: "  --  ".into(), // sanitize_component -> ""
            control: PathBuf::from("/tmp/ctl"),
            remote_socket: PathBuf::from("/Users/x/.config/herdr/herdr.sock"),
        };
        let req = notify_request(&mk("w1:p1", HerdrAgentStatus::Working, false), &conn)
            .expect("notify_request 应成功");
        assert_eq!(req.task_id, "hr-w1:p1");
    }

    #[test]
    fn sanitize_component_keeps_aliases_and_collapses_urls() {
        assert_eq!(sanitize_component("hello"), "hello");
        assert_eq!(sanitize_component("work.box"), "work.box");
        assert_eq!(
            sanitize_component("ssh://you@host:2222"),
            "ssh---you-host-2222"
        );
        assert_eq!(sanitize_component("  --  "), "");
    }

    #[test]
    fn remote_clients_from_ps_text_handles_right_aligned_pid_column() {
        // 回归(2026-08-18):ps 右对齐 pid 列带前导空格时旧 splitn 整行跳过。
        let text = concat!(
            "  PID COMMAND\n",
            " 8748 herdr --remote hello\n",
            " 7777 herdr --remote=work.box\n",
            "12345 ssh -T other\n",
        );
        assert_eq!(
            remote_clients_from_ps_text(text),
            vec![
                ("8748".to_string(), "hello".to_string()),
                ("7777".to_string(), "work.box".to_string()),
            ]
        );
        // 无前导空格(max pid ≤ 4 位)同样解析;无 --remote 的行不产出。
        assert_eq!(
            remote_clients_from_ps_text("8748 herdr --remote hello\n herdr server\n"),
            vec![("8748".to_string(), "hello".to_string())]
        );
    }

    #[test]
    fn project_name_uses_basename() {
        assert_eq!(project_name("/Users/x/MyProject"), "MyProject");
        assert_eq!(project_name("/"), "/");
        assert_eq!(project_name(""), "");
    }

    // is_hook_capable_agent 的测试随函数搬到 herdr_remote/agent_capability.rs。
}
