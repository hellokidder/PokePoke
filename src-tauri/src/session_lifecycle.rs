//! Session 宿主探活与清理 — 从 `lib.rs` 抽出,供 bootstrap reaper 复用。

use crate::popup;
use crate::sessions::{Session, SessionStore};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// Close the popup associated with a session and remove it from the store.
/// Does NOT emit "sessions-updated" — caller is responsible for batching emits.
pub fn remove_session_with_cleanup(
    app: &AppHandle,
    store: &Arc<Mutex<SessionStore>>,
    popup_list: &popup::PopupList,
    session_id: &str,
    reason: popup::PopupCloseReason,
) {
    popup::close_popup(app, session_id, popup_list, reason);
    store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove_session(session_id);
}

/// 判断一个 session 的宿主（agent 进程 / IDE）是否还活着。
pub fn is_session_alive(session: &Session) -> bool {
    // brief 20260804-herdr-agent-status:herdr 绑定 session 的探活第二信源。
    // herdr pane.list 明确报活(agent_status ∈ idle/working/blocked/done,对 pi
    // 这类 comm 探活脆弱的 agent 尤其有价值)即活;查不到 pane / unknown /
    // socket 失败一律退既有 tty 探活 —— herdr 侧永不判死(见 pane_snapshot_alive),
    // 不改"无 tty 判死"等既有语义。
    if let Some((pane_id, _, Some(sock))) = crate::surface::SurfaceBinding::herdr_fields(session) {
        if let Some(alive) = crate::herdr::pane_liveness(std::path::Path::new(sock), pane_id) {
            return alive;
        }
        // brief 20260813-herdr-triple-popup:pane_liveness 返回 None(探活超时 /
        // agent_status=unknown / pane 暂不可见)时 herdr 侧无法判定。本地 herdr
        // 监视器任务(source="herdr")的生死由监视器 disappear→session_end 管理,
        // reaper 不判死 —— 与 herdr-remote 同原则("herdr 侧永不判死")。此前
        // 落回 `_ => false` 判死 → session 被误删 → 监视器补喂 idle 重建
        // is_new+Idle session → 幽灵"完成" popup(2026-08-13 实测三连弹)。
        if session.source.as_deref() == Some("herdr") {
            return true;
        }
    }

    let source = session.source.as_deref().unwrap_or("").to_ascii_lowercase();

    // herdr_remote(远程监视器)管理的 session:agent 跑在远程机,本地探活必然失败。
    // 生命周期由 monitor 的 agent.list 消失 3 轮 → session_end 管理,reaper 不判死。
    if source == "herdr-remote" {
        return true;
    }

    match source.as_str() {
        "claude-code" => probe_cli_agent_alive(session, "claude"),
        // codex 有两种宿主:CLI(终端跑,有 tty)与 ChatGPT 桌面端(Electron,无 tty)。
        // 后者无 tty 但活着 -- 走桌面端探活,不能套用"CLI agent 必然有 tty"的判死逻辑。
        "codex" => probe_codex_alive(session),
        "opencode" => probe_cli_agent_alive(session, "opencode"),
        // pi CLI 是 node 脚本:已安装版 `process.title=APP_NAME` 让 comm=pi,
        // 但 dev/tsx/wrapper 启动路径 comm 是 node/tsx/bun。探活必须认多 comm
        // 候选,命中 node/tsx/bun 时附加 args 校验,避免把 tty 上无关的 node
        // 进程误判为 pi 存活。见 brief 20260804 §4-① + §5.2 对线结论。
        "pi" => probe_pi_alive(session),
        // omp(pi 的 fork)同样是 node/bun 脚本:comm 可能是 omp(二进制装法)
        // 或 node/bun/tsx(args 带 `oh-my-pi`)。
        "omp" => probe_omp_alive(session),
        // Kimi CLI 因安装方式不同,comm 可能是 kimi 也可能是 kimi-code
        // (~/.kimi-code/bin/kimi → kimi,TOML hook 安装 → kimi-code)。
        // 只探一个会把存活 session 连续 2 次 miss 误清,popup 被 reaper 回收。
        "kimi" => probe_cli_agent_alive_many(session, &["kimi", "kimi-code"]),
        // grok CLI 是原生 Mach-O 二进制(20260804 审查核实本机安装),comm 恒为
        // "grok",无 node/tsx/bun wrapper 风险,不需要多候选 + args 校验。
        "grok" => probe_cli_agent_alive(session, "grok"),
        // DSH 是 Electron/Web 宿主,无 tty、无 CLI 进程:插件自建的 socket 还在
        // 就说明 host 进程活着;socket 缺失时退回查 App 主进程。
        "dsh" => probe_dsh_alive(session),
        "cursor" => probe_cursor_alive(session),
        _ => false,
    }
}

/// codex 探活:按 tty 分流两种宿主。
///
/// - 有 tty:codex CLI 跑在终端,探 tty 上的 codex 进程(同其它 CLI agent)。
/// - 无 tty:ChatGPT 桌面端会话(codex 跑在 Electron 里,不是终端 CLI)。此时
///   `probe_cli_agent_alive` 的"无 tty = 死"前提不成立 -- 探 ChatGPT.app 进程。
///
/// 历史背景:`probe_cli_agent_alive` 无 tty 分支按 docs/product-spec.md
/// §CLI-agent-no-tty 判死,前提是"CLI agent 必然跑在 TTY 上"。桌面端打破该
/// 前提,所以 codex 单独分流,不共用 `probe_cli_agent_alive`。
fn probe_codex_alive(session: &Session) -> bool {
    match session.terminal_tty.as_deref() {
        Some(t) if !t.is_empty() => probe_cli_agent_alive_on_tty(t, "codex"),
        _ => probe_codex_desktop_alive(),
    }
}

/// DSH 探活:socket 文件存在 = host 进程还在(brief 20261104-dsh-integration)。
/// 没有 socket(旧 session / 插件尚未接到事件)时退回 App 主进程探测;探测工具
/// 失败保守判活,避免误清用户正在用的会话。
fn probe_dsh_alive(session: &Session) -> bool {
    if let Some(socket) = session.dsh_nav_socket.as_deref() {
        if !socket.is_empty() {
            return std::path::Path::new(socket).exists();
        }
    }
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("pgrep")
            .arg("-x")
            .arg("DeepSeek Harness")
            .output()
        {
            Ok(output) => !output.stdout.is_empty(),
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// ChatGPT 桌面端探活:ChatGPT.app(bundle id `com.openai.codex`,Electron)主进程
/// comm 名为 "ChatGPT"。`pgrep -x` 精确匹配主进程(不撞 Helper/Renderer 子进程)。
///
/// 探活工具失败时保守判活(返 true):与 `probe_cli_agent_alive_on_tty` / `probe_cursor_alive`
/// 一致 -- 探活失败时倾向不删,避免误清用户正用的桌面端会话。桌面端会话无 tty,
/// 无法靠 tty 复探,工具失败时的错杀代价高于留僵尸(下次 SessionEnd hook 会兜底清)。
fn probe_codex_desktop_alive() -> bool {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("pgrep")
            .arg("-x")
            .arg("ChatGPT")
            .output()
        {
            Ok(output) => !output.stdout.is_empty(),
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn probe_cli_agent_alive(session: &Session, pname: &str) -> bool {
    probe_cli_agent_alive_many(session, &[pname])
}

fn probe_cli_agent_alive_many(session: &Session, pnames: &[&str]) -> bool {
    match session.terminal_tty.as_deref() {
        Some(t) if !t.is_empty() => probe_cli_agent_alive_on_tty_many(t, pnames),
        _ => {
            // v1.0.1 review M3:回归 docs/product-spec.md §CLI-agent-no-tty 的原始设计
            // (Task C 3d11a0d)。CLI agent 必然跑在 TTY 上;无 TTY 时任何 ps -A 全局
            // 命名匹配都会被"别处一个 claude / codex 进程"骗到,把死 session 长留面板、
            // popup 不清。宁可这一次错判死(下次 hook 事件重报),也不留僵尸。
            //
            // 回归动机:refactor 67c0913 modularize 时无意引入了 name-only fallback,
            // 与 spec 直接违背(review 二审 CONFIRMED)。
            crate::log_debug!(
                "reaper",
                "tty=None → dead (session_id={} pname={})",
                session.id,
                pnames.join("/")
            );
            false
        }
    }
}

fn probe_cli_agent_alive_on_tty(tty: &str, pname: &str) -> bool {
    probe_cli_agent_alive_on_tty_many(tty, &[pname])
}

fn probe_cli_agent_alive_on_tty_many(tty: &str, pnames: &[&str]) -> bool {
    if !std::path::Path::new(tty).exists() {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        let tty_short = tty.trim_start_matches("/dev/");
        match std::process::Command::new("ps")
            .arg("-t")
            .arg(tty_short)
            .arg("-o")
            .arg("comm=")
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(|line| comm_matches_any(line, pnames))
            }
            Err(_) => true,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = pnames;
        true
    }
}

fn comm_matches_any(line: &str, pnames: &[&str]) -> bool {
    let trimmed = line.trim();
    let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed);
    pnames.contains(&leaf)
}

/// ps -o comm=,args= 单行里 pi 运行时 comm 的候选(node/tsx/bun 需 args 校验)。
/// 已安装版 comm=pi 直接命中;dev/tsx/wrapper 路径的运行时名在这里。
const PI_RUNTIME_COMMS: &[&str] = &["node", "tsx", "bun"];
/// pi 进程 args 的特征串:已安装包路径 `@earendil-works/pi-coding-agent`
/// (含 coding-agent 子串)与 dev 入口 cli.ts。
const PI_ARGS_HINTS: &[&str] = &["coding-agent", "cli.ts"];

/// pi 探活:多 comm 候选 + args 校验,无 tty 判死语义与其它 CLI agent 一致。
fn probe_pi_alive(session: &Session) -> bool {
    match session.terminal_tty.as_deref() {
        Some(t) if !t.is_empty() => probe_pi_alive_on_tty(t),
        _ => {
            crate::log_debug!(
                "reaper",
                "tty=None → dead (session_id={} pname=pi)",
                session.id,
            );
            false
        }
    }
}

fn probe_pi_alive_on_tty(tty: &str) -> bool {
    if !std::path::Path::new(tty).exists() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let tty_short = tty.trim_start_matches("/dev/");
        match std::process::Command::new("ps")
            .arg("-t")
            .arg(tty_short)
            .arg("-o")
            .arg("comm=,args=")
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(pi_process_line_alive)
            }
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = tty;
        true
    }
}

/// 判定一行 `ps -o comm=,args=` 输出是否属于 pi 进程。
///
/// comm 精确命中 pi → 活(已安装版 process.title 副作用);comm 是 node/tsx/bun
/// 且 args 含 pi 特征 → 活(dev/wrapper 启动);其余一律不算 —— 避免把 tty 上
/// 无关的 node 进程误判为 pi 存活。
fn pi_process_line_alive(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // 两列输出:第一个空白分隔 token 是 comm,其余是 args。comm 不含空白
    // (可执行名);路径含空白的罕见 case 只影响 args 起点,不影响判定。
    let (comm, args) = match trimmed.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (trimmed, ""),
    };
    let leaf = comm.rsplit('/').next().unwrap_or(comm);
    if leaf == "pi" {
        return true;
    }
    PI_RUNTIME_COMMS.contains(&leaf) && PI_ARGS_HINTS.iter().any(|h| args.contains(h))
}

/// omp 进程 args 的特征串:包路径 `@oh-my-pi/pi-coding-agent` 与 omp 自身数据根。
/// **不能沿用 pi 的 `coding-agent` 特征** —— omp 的包名 `pi-coding-agent` 含该
/// 子串,tty 上同时存在 pi 时会互相误判。
const OMP_ARGS_HINTS: &[&str] = &["oh-my-pi", ".omp"];
/// `bin/omp` shim 的路径段名:fnm / npm 装的 `omp` 入口是 `#!/usr/bin/env bun`
/// 的脚本,进程行长这样(2026-09-24 本机实测):
/// `bun /Users/…/fnm/node-versions/v24.14.1/installation/bin/omp`
/// —— comm 是 bun、args 既没有 `oh-my-pi` 也没有 `.omp`,唯一特征是**末尾路径段
/// 就是 omp**。漏了这条会让 reaper 每轮判死:面板丢会话 + popup 被连带回收。
const OMP_SHIM_LEAF: &str = "omp";

/// omp 探活:多 comm 候选 + args 校验,无 tty 判死语义与其它 CLI agent 一致。
fn probe_omp_alive(session: &Session) -> bool {
    match session.terminal_tty.as_deref() {
        Some(t) if !t.is_empty() => probe_omp_alive_on_tty(t),
        _ => {
            crate::log_debug!(
                "reaper",
                "tty=None → dead (session_id={} pname=omp)",
                session.id,
            );
            false
        }
    }
}

fn probe_omp_alive_on_tty(tty: &str) -> bool {
    if !std::path::Path::new(tty).exists() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let tty_short = tty.trim_start_matches("/dev/");
        match std::process::Command::new("ps")
            .arg("-t")
            .arg(tty_short)
            .arg("-o")
            .arg("comm=,args=")
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(omp_process_line_alive)
            }
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = tty;
        true
    }
}

/// 判定一行 `ps -o comm=,args=` 输出是否属于 omp 进程。三种装法(均真机实测):
///
/// - 编译版二进制 / `bun link`:comm 精确命中 `omp`;
/// - **fnm 装的 `bin/omp` shim**:comm=bun、args 末尾路径段是 `omp`(见
///   `OMP_SHIM_LEAF` 注释);
/// - npm/bun 全局装:`node|bun|tsx …/cli.js`,args 含 `oh-my-pi` / `.omp`。
///
/// **args 判定不看 comm**:macOS `ps -o comm=` 会把超长路径截断(实测
/// `comm=/Users/edy/.bun/`,尾斜杠),此时 comm 既不是裸运行时名也不是 `omp`,
/// 若在 comm 上做闸门就会漏判存活 → reaper 误清。args 特征本身就是充分条件
/// (tty 上有 omp 进程 = 会话活着),宁可多留一个 tick 也不误杀。
fn omp_process_line_alive(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let (comm, args) = match trimmed.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (trimmed, ""),
    };
    if comm.rsplit('/').next() == Some(OMP_SHIM_LEAF) {
        return true;
    }
    OMP_ARGS_HINTS.iter().any(|h| args.contains(h))
        || args
            .split_whitespace()
            .any(|arg| arg.rsplit('/').next() == Some(OMP_SHIM_LEAF))
}

/// cursor 探活:按 tty 分流两种宿主(同 codex 的双宿主模式)。
///
/// - 有 tty:**cursor-agent CLI** 跑在终端里(cmux / iTerm2 / Ghostty 等),
///   hook 能抓到 tty,探 tty 上的 cursor-agent 进程。历史缺口(2026-09-01
///   真机排查):此前 cursor 只有 `pgrep -x Cursor` 的 app 级探活(GUI 假设),
///   用户在 cmux 里跑 CLI 且 IDE 没开时探活恒假 → session+popup 约 10s 被
///   reaper 误清(任务结束 popup 闪一下就没、running 记录中途消失导致
///   stop 到达时 prev=None 重建)。
/// - 无 tty:Cursor **IDE 桌面端** GUI hook(无 controlling tty),保留
///   `pgrep -x Cursor` app 级探活(workspace 级无法精确,spec §99 既有取舍)。
fn probe_cursor_alive(session: &Session) -> bool {
    match session.terminal_tty.as_deref() {
        Some(t) if !t.is_empty() => probe_cursor_agent_alive_on_tty(t),
        _ => probe_cursor_desktop_alive(),
    }
}

/// Cursor IDE 桌面端探活:`pgrep -x Cursor` 主进程是否存在。
/// 探活工具失败时保守判活(返 true),与其它探针一致。
fn probe_cursor_desktop_alive() -> bool {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("pgrep")
            .arg("-x")
            .arg("Cursor")
            .output()
        {
            Ok(output) => !output.stdout.is_empty(),
            Err(_) => true,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// tty 上 cursor-agent CLI 探活。cursor-agent 是 bash wrapper,最终
/// `exec -a "$0" node .../cursor-agent/versions/<v>/index.js "$@"` ——
/// comm 列是 argv[0] 被 macOS ps 截断到 16 字符(如 `/Users/edy/.loca`),
/// 精确 comm 匹配必然失败;args 列带完整 argv,按 `cursor-agent` 特征串
/// 匹配(同 pi 的 node/tsx args hint 策略)。
fn probe_cursor_agent_alive_on_tty(tty: &str) -> bool {
    if !std::path::Path::new(tty).exists() {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        let tty_short = tty.trim_start_matches("/dev/");
        match std::process::Command::new("ps")
            .arg("-t")
            .arg(tty_short)
            .arg("-o")
            .arg("comm=,args=")
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().any(cursor_process_line_alive)
            }
            Err(_) => true,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = tty;
        true
    }
}

/// 判定一行 `ps -o comm=,args=` 输出是否属于 cursor-agent CLI 进程。
///
/// - comm leaf 精确为 `cursor-agent`(原生 bin / 未截断情形)→ 活。
/// - comm 是截断路径或 node(wrapper `exec -a` 情形),args 含 `cursor-agent`
///   特征(argv[0] 路径 `.../bin/cursor-agent` 与入口 `.../cursor-agent/versions/<v>/index.js`
///   都含该串)→ 活。
/// - 其余(node 跑无关脚本、shell、hook 子进程等)→ 不算。tty 内 scoped,
///   误判活代价仅为多留一轮(下次 hook / SessionEnd 兜底),与 pi 同信任级。
fn cursor_process_line_alive(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let (comm, args) = match trimmed.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (trimmed, ""),
    };
    let leaf = comm.rsplit('/').next().unwrap_or(comm);
    if leaf == "cursor-agent" {
        return true;
    }
    args.contains("cursor-agent")
}

/// 启动时无条件清空 panel。见 `docs/briefs/20260420-startup-clear-all-sessions.md`。
pub fn clear_sessions_on_startup(store: &Arc<Mutex<SessionStore>>) {
    let mut guard = store.lock().unwrap_or_else(|p| p.into_inner());
    let all_ids: Vec<String> = guard.get_all().iter().map(|s| s.id.clone()).collect();
    for id in &all_ids {
        guard.remove_session(id);
    }
    if !all_ids.is_empty() {
        crate::log_info!(
            "lifecycle",
            "startup reset: cleared {} session(s)",
            all_ids.len()
        );
    }
}

/// 高频探活线程：每 5 秒探活,连续 2 次 miss 才 reap。
/// W3(brief 20260813-watcher-supervisor):迁移到 supervisor::spawn_periodic
/// (命名线程 + catch_unwind + 心跳);miss_counts 状态上移到闭包外跨轮保持,
/// 轮内 body 逻辑零变化。
pub fn spawn_session_reaper(
    store: Arc<Mutex<SessionStore>>,
    popup_list: popup::PopupList,
    app: AppHandle,
) {
    let mut miss_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    crate::supervisor::spawn_periodic(
        "session-reaper",
        std::time::Duration::from_secs(5),
        move || {
            let sessions: Vec<_> = store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get_all()
                .to_vec();

            let mut to_remove: Vec<String> = Vec::new();

            for session in &sessions {
                if is_session_alive(session) {
                    if miss_counts.remove(&session.id).is_some() {
                        crate::log_debug!(
                            "reaper",
                            "recovered task_id={} source={:?} tty={:?}",
                            session.task_id,
                            session.source,
                            session.terminal_tty,
                        );
                    }
                } else {
                    let count = miss_counts.entry(session.id.clone()).or_insert(0);
                    *count += 1;
                    crate::log_debug!(
                        "reaper",
                        "miss task_id={} source={:?} tty={:?} count={}",
                        session.task_id,
                        session.source,
                        session.terminal_tty,
                        *count,
                    );
                    if *count >= 2 {
                        to_remove.push(session.id.clone());
                    }
                }
            }

            miss_counts.retain(|id, _| sessions.iter().any(|s| &s.id == id));

            let mut removed_any = false;
            for session_id in &to_remove {
                miss_counts.remove(session_id);
                let session_meta = store
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_all()
                    .iter()
                    .find(|s| s.id == *session_id)
                    .map(|s| (s.task_id.clone(), s.source.clone(), s.terminal_tty.clone()));
                let should_remove = session_meta.as_ref().is_some_and(|_| {
                    store
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get_all()
                        .iter()
                        .find(|s| s.id == *session_id)
                        .is_some_and(|s| !is_session_alive(s))
                });
                if should_remove {
                    if let Some((task_id, source, tty)) = session_meta {
                        crate::log_debug!(
                            "reaper",
                            "reap task_id={} source={:?} tty={:?}",
                            task_id,
                            source,
                            tty,
                        );
                    }
                    remove_session_with_cleanup(
                        &app,
                        &store,
                        &popup_list,
                        session_id,
                        popup::PopupCloseReason::Reaper,
                    );
                    removed_any = true;
                } else if let Some((task_id, source, tty)) = session_meta {
                    crate::log_debug!(
                        "reaper",
                        "spared task_id={} source={:?} tty={:?}",
                        task_id,
                        source,
                        tty,
                    );
                }
            }

            if removed_any {
                let _ = app.emit("sessions-updated", ());
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{
        comm_matches_any, cursor_process_line_alive, omp_process_line_alive, pi_process_line_alive,
    };
    use crate::sessions::Session;
    use chrono::DateTime;

    fn make_session(id: &str, task_id: &str, source: Option<&str>, tty: Option<&str>) -> Session {
        Session {
            id: id.into(),
            task_id: task_id.into(),
            external_session_id: None,
            title: String::new(),
            message: String::new(),
            source: source.map(|s| s.into()),
            priority: crate::sessions::Priority::Normal,
            status: crate::sessions::SessionStatus::Running,
            created_at: DateTime::default(),
            updated_at: DateTime::default(),
            terminal_tty: tty.map(|s| s.into()),
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
            surface: Default::default(), // W5:内层绑定真源,见 brief 20260813-surface-binding
        }
    }

    // ==================== comm_matches_any ====================

    #[test]
    fn comm_matches_exact() {
        assert!(comm_matches_any("claude", &["claude"]));
    }

    #[test]
    fn comm_matches_with_prefix_path() {
        assert!(comm_matches_any("/usr/bin/claude", &["claude"]));
    }

    #[test]
    fn comm_matches_trailing_space() {
        assert!(comm_matches_any("  /usr/local/bin/claude", &["claude"]));
    }

    #[test]
    fn comm_matches_full_path_only() {
        assert!(comm_matches_any("/opt/homebrew/bin/claude", &["claude"]));
    }

    #[test]
    fn comm_matches_wrong_name() {
        assert!(!comm_matches_any("codex", &["claude"]));
    }

    #[test]
    fn comm_matches_empty_line() {
        assert!(!comm_matches_any("", &["claude"]));
    }

    #[test]
    fn comm_matches_whitespace_only() {
        assert!(!comm_matches_any("   ", &["claude"]));
    }

    #[test]
    fn comm_matches_longer_name_prefix() {
        // "claudecode" 包含 "claude" 前 6 个字符但不是精确匹配
        assert!(!comm_matches_any("claudecode", &["claude"]));
    }

    #[test]
    fn comm_matches_substring_in_path_component() {
        // /bin/claude-coder 不应匹配 claude
        assert!(!comm_matches_any("/bin/claude-coder", &["claude"]));
    }

    #[test]
    fn comm_matches_kimi_code_process_name() {
        assert!(comm_matches_any("kimi-code", &["kimi-code"]));
        assert!(comm_matches_any(
            "/opt/homebrew/bin/kimi-code",
            &["kimi-code"]
        ));
        // 旧误配 "kimi" 探不到真实进程,回归守门
        assert!(!comm_matches_any("kimi-code", &["kimi"]));
    }

    #[test]
    fn comm_matches_any_accepts_both_kimi_binary_names() {
        // Kimi CLI 因安装方式不同 comm 可能是 kimi 或 kimi-code,reaper 必须都认。
        assert!(comm_matches_any("kimi", &["kimi", "kimi-code"]));
        assert!(comm_matches_any("kimi-code", &["kimi", "kimi-code"]));
        assert!(comm_matches_any(
            "/Users/edy/.kimi-code/bin/kimi",
            &["kimi", "kimi-code"]
        ));
        assert!(comm_matches_any(
            "/opt/homebrew/bin/kimi-code",
            &["kimi", "kimi-code"]
        ));
        // 非 kimi 进程不误认
        assert!(!comm_matches_any("codex", &["kimi", "kimi-code"]));
    }

    // ==================== pi 探活(多 comm + args 校验) ====================

    #[test]
    fn pi_line_installed_comm_pi_alive() {
        // 已安装版 process.title=pi → comm 就是 pi(可带路径 / 后续 args)。
        assert!(pi_process_line_alive("pi"));
        assert!(pi_process_line_alive("/usr/local/bin/pi"));
        assert!(pi_process_line_alive("pi --model foo --print"));
    }

    #[test]
    fn pi_line_runtime_comm_requires_pi_args() {
        // dev / wrapper 路径:comm 是 node/tsx/bun,args 带 pi 特征才认。
        assert!(pi_process_line_alive(
            "node /Users/edy/.pi/dist/cli.js coding-agent"
        ));
        assert!(pi_process_line_alive(
            "node .../pi-coding-agent/dist/cli.js --session x"
        ));
        assert!(pi_process_line_alive("tsx src/cli.ts --session x"));
        assert!(pi_process_line_alive("bun run cli.ts"));
    }

    #[test]
    fn pi_line_unrelated_process_not_alive() {
        // tty 上无关的 node/其它进程不带 pi 特征,不得误判为 pi 存活。
        assert!(!pi_process_line_alive("node"));
        assert!(!pi_process_line_alive("node /usr/local/bin/esbuild"));
        assert!(!pi_process_line_alive("node --watch server.js"));
        assert!(!pi_process_line_alive("tsx --version"));
        assert!(!pi_process_line_alive("zsh"));
        assert!(!pi_process_line_alive(""));
        assert!(!pi_process_line_alive("   "));
    }

    #[test]
    fn pi_probe_nonexistent_tty_false() {
        assert!(!super::probe_pi_alive_on_tty(
            "/dev/this_tty_does_not_exist_xyz"
        ));
    }

    // ==================== omp 探活(多 comm + args 校验) ====================

    #[test]
    fn omp_line_binary_comm_alive() {
        // omp.sh 安装的编译版二进制 / bun link 出的 bin:comm 就是 omp。
        assert!(omp_process_line_alive("omp"));
        assert!(omp_process_line_alive("/Users/edy/.local/bin/omp"));
        assert!(omp_process_line_alive("omp --model opus"));
    }

    #[test]
    fn omp_line_runtime_comm_requires_omp_args() {
        // npm/bun 全局装:comm 是 node/bun,args 带包路径特征才认。
        assert!(omp_process_line_alive(
            "bun /Users/edy/.bun/bin/../install/global/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js -p hi"
        ));
        assert!(omp_process_line_alive(
            "node /usr/local/lib/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js"
        ));
        assert!(omp_process_line_alive(
            "node /Users/edy/.omp/agent/some-wrapper.js"
        ));
    }

    #[test]
    fn omp_line_bun_shim_comm_alive() {
        // **回归护栏(2026-09-24 真机)**:fnm 装的 `bin/omp` 是 `#!/usr/bin/env bun`
        // 的脚本,进程行是 `bun …/installation/bin/omp` —— 既没有 `oh-my-pi` 也没有
        // `.omp`。漏判会让 reaper 连续 2 轮判死:面板会话消失 + popup 被 popup-reaper
        // 连带回收(只剩声音)。
        assert!(omp_process_line_alive(
            "bun /Users/edy/.local/share/fnm/node-versions/v24.14.1/installation/bin/omp"
        ));
        assert!(omp_process_line_alive("node /usr/local/bin/omp"));
        // 带子命令/参数的同一进程行
        assert!(omp_process_line_alive(
            "bun /Users/edy/.local/bin/omp --model deepseek/deepseek-v4-flash"
        ));
        // 相似但不同名的入口不得误判
        assert!(!omp_process_line_alive("bun /usr/local/bin/ompx"));
        assert!(!omp_process_line_alive("bun /usr/local/bin/omp-helper.js"));
        assert!(!omp_process_line_alive("bun /usr/local/bin/pomp"));
    }

    #[test]
    fn omp_line_truncated_comm_still_matches_args() {
        // **回归护栏(2026-09-24 真机)**:macOS `ps -o comm=` 会把超长路径截断成
        // `/Users/edy/.bun/`(尾斜杠)。comm 既不是裸运行时名也不是 omp,判活必须
        // 落到 args 特征上,否则会话被 reaper 误清(面板丢条目 + popup 被回收)。
        assert!(omp_process_line_alive(
            "/Users/edy/.bun/ /Users/edy/.bun/bin/bun /Users/edy/.local/share/fnm/node-versions/v24.14.1/installation/lib/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js --profile=volcano"
        ));
        assert!(omp_process_line_alive(
            "/Users/edy/.bun/ /Users/edy/.bun/bin/bun /Users/edy/.omp/agent/sessions/runner.js"
        ));
        // 截断 comm + 无 args 特征 -> 不判活(该 tty 上没有 omp 进程)
        assert!(!omp_process_line_alive(
            "/Users/edy/.bun/ /Users/edy/.bun/bin/bun /Users/edy/project/server.js"
        ));
    }

    #[test]
    fn omp_line_does_not_match_pi_runtime() {
        // 回归护栏:pi 的包名 `@earendil-works/pi-coding-agent` 不含 omp 特征,
        // 不得被 omp 探活误认为存活(否则同 tty 上 pi 死后 omp session 长留面板)。
        assert!(!omp_process_line_alive(
            "node /Users/edy/.nvm/versions/node/v22/lib/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js"
        ));
        assert!(!omp_process_line_alive("node"));
        assert!(!omp_process_line_alive("bun run dev"));
        assert!(!omp_process_line_alive("zsh"));
        assert!(!omp_process_line_alive(""));
    }

    #[test]
    fn omp_probe_nonexistent_tty_false() {
        assert!(!super::probe_omp_alive_on_tty(
            "/dev/this_tty_does_not_exist_xyz"
        ));
    }

    // ==================== cursor-agent CLI 探活(args 特征匹配) ====================

    #[test]
    fn cursor_line_installed_comm_alive() {
        // comm leaf 精确为 cursor-agent(原生 bin / 未截断情形)。
        assert!(cursor_process_line_alive("cursor-agent"));
        assert!(cursor_process_line_alive("/usr/local/bin/cursor-agent"));
    }

    #[test]
    fn cursor_line_exec_a_wrapper_requires_args() {
        // 真机形态:bash wrapper `exec -a "$0" node ... index.js`,comm 被
        // macOS ps 截到 16 字符(`/Users/edy/.loca`),args 带完整 argv。
        assert!(cursor_process_line_alive(
            "/Users/edy/.loca /Users/edy/.local/bin/cursor-agent --use-system-ca /Users/edy/.local/share/cursor-agent/versions/2026.08.25-3e8eec8/index.js"
        ));
        // 截断 comm 但 args 无特征 → 不判活。
        assert!(!cursor_process_line_alive("/Users/edy/.loca"));
    }

    #[test]
    fn cursor_line_unrelated_process_not_alive() {
        // tty 上无关 node / shell / 其它 agent 的 hook 子进程不得误判。
        assert!(!cursor_process_line_alive("node /usr/local/bin/esbuild"));
        assert!(!cursor_process_line_alive(
            "node /Users/edy/.local/bin/poke-hook --source=cursor"
        ));
        assert!(!cursor_process_line_alive("-/bin/zsh -/bin/zsh"));
        assert!(!cursor_process_line_alive("claude --session-id x"));
        assert!(!cursor_process_line_alive(""));
        assert!(!cursor_process_line_alive("   "));
    }

    #[test]
    fn cursor_probe_nonexistent_tty_false() {
        assert!(!super::probe_cursor_agent_alive_on_tty(
            "/dev/this_tty_does_not_exist_xyz"
        ));
    }

    // ==================== is_session_alive ====================

    #[test]
    fn is_session_alive_unknown_source_false() {
        let s = make_session("s1", "t1", Some("vim"), Some("/dev/ttys000"));
        // 未知 source → false (不会因为误判 alive 而留僵尸)
        assert!(!super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_claude_code_no_tty_false() {
        let s = make_session("s2", "t2", Some("claude-code"), None);
        // 无 TTY 的 claude-code → dead (spec 明确不允许 name-only 匹配)
        assert!(!super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_claude_code_empty_tty_false() {
        let s = make_session("s3", "t3", Some("claude-code"), Some(""));
        // 空字符串 TTY → dead
        assert!(!super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_cursor_no_tty_no_panic() {
        // cursor 无 tty(Cursor IDE GUI 会话)走 probe_cursor_desktop_alive(pgrep -x Cursor)
        let s = make_session("s4", "t4", Some("cursor"), None);
        // 真值取决于本机是否开了 Cursor.app,这里只检查不 panic
        let _ = super::is_session_alive(&s);
    }

    #[test]
    fn is_session_alive_cursor_with_tty_uses_tty_probe() {
        // cursor-agent CLI 有 tty:走 tty 探针;不存在的 tty -> false,
        // 证明有 tty 时没误走桌面端 pgrep 路径(IDE 没开也不该判死 CLI 会话)。
        let s = make_session(
            "s4b",
            "t4b",
            Some("cursor"),
            Some("/dev/ttys999_nonexistent"),
        );
        assert!(!super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_codex_no_tty_probes_desktop() {
        // codex 桌面端(ChatGPT.app)无 tty:走 probe_codex_desktop_alive(pgrep ChatGPT),
        // 不走"无 tty = 死"判死。测试环境无论 ChatGPT 是否在跑,只验证不 panic
        // 且不返回"判死"的固定 false(真值取决于本机是否开了 ChatGPT)。
        let s = make_session("s6", "t6", Some("codex"), None);
        let _ = super::is_session_alive(&s);
    }

    #[test]
    fn is_session_alive_codex_with_tty_uses_tty_probe() {
        // codex CLI 有 tty:走 probe_cli_agent_alive_on_tty(同其它 CLI agent)。
        // 不存在的 tty -> false,证明有 tty 时没误走桌面端探活路径。
        let s = make_session("s7", "t7", Some("codex"), Some("/dev/ttys999_nonexistent"));
        assert!(!super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_case_insensitive_source() {
        let s = make_session("s5", "t5", Some("Claude-Code"), None);
        // 大小写不影响 source 匹配
        assert!(!super::is_session_alive(&s)); // 无 tty → dead
    }

    // ==================== herdr binding 探活(brief 20260813-herdr-triple-popup) ====================

    fn with_herdr_binding(mut s: Session, pane_id: &str) -> Session {
        s.surface = crate::surface::SurfaceBinding::Herdr {
            pane_id: pane_id.to_string(),
            workspace_id: Some("w8".to_string()),
            // 不存在的 socket 路径 → send_once 失败 → pane_liveness 返回 None,
            // 恰好复现"herdr 侧无法判定"分支,无需真 herdr。
            socket_path: Some("/nonexistent/herdr-test.sock".to_string()),
        };
        s
    }

    #[test]
    fn is_session_alive_local_herdr_liveness_none_stays_alive() {
        // Fix A:本地 herdr 监视器任务(source="herdr")在 pane_liveness 无法判定
        // (None)时不判死 —— 生死由监视器 disappear→session_end 管理,与
        // herdr-remote 同原则。此前落回 `_ => false` 判死 → session 误删 →
        // 补喂 idle 重建 is_new+Idle → 幽灵"完成" popup。
        let s = with_herdr_binding(
            make_session("s-herdr", "hr-w8:pN", Some("herdr"), None),
            "w8:pN",
        );
        assert!(super::is_session_alive(&s));
    }

    #[test]
    fn is_session_alive_agent_in_herdr_pane_liveness_none_falls_back_to_tty() {
        // Fix A 反向守卫:带 herdr binding 的 agent 源(claude-code)在
        // pane_liveness None 时仍走既有 tty 探活(无 tty → 死),行为不变。
        let s = with_herdr_binding(
            make_session("s-cc", "cc-1", Some("claude-code"), None),
            "w8:pN",
        );
        assert!(!super::is_session_alive(&s));
    }

    // ==================== probe_cli_agent_alive_on_tty ====================

    #[test]
    fn probe_on_nonexistent_tty_false() {
        // 不存在的 tty 立即返回 false
        assert!(!super::probe_cli_agent_alive_on_tty(
            "/dev/this_tty_does_not_exist_xyz",
            "claude"
        ));
    }
}
