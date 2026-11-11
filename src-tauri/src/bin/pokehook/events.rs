//! Hook 事件处理(CC/Codex/Cursor/opencode/pi/kimi/grok 共用):事件归一、
//! payload 构造。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::env_collect::{
    collect_cmux_env, collect_herdr_env, collect_otty_env, collect_wezterm_env,
};
use super::orca_env;
use super::source::Source;
use super::subagents;
use super::tmux_env;
use super::{flag_path, get_tty, post_notify};
use serde_json::Value;
use std::env;
use std::fs;

/// Normalize Cursor camelCase and Grok snake_case event names to internal PascalCase.
pub(crate) fn normalize_event(raw: &str) -> String {
    match raw {
        "sessionStart" => "SessionStart".into(),
        "session_start" => "SessionStart".into(),
        "beforeSubmitPrompt" => "UserPromptSubmit".into(),
        "user_prompt_submit" => "UserPromptSubmit".into(),
        "stop" => "Stop".into(),
        "sessionEnd" => "SessionEnd".into(),
        "session_end" => "SessionEnd".into(),
        "notification" => "Notification".into(),
        "stop_failure" | "stopFailure" => "StopFailure".into(),
        "subagent_start" | "subagentStart" => "SubagentStart".into(),
        "subagent_stop" | "subagentStop" => "SubagentStop".into(),
        other => other.into(),
    }
}

// ==================== Event handlers ====================

/// cc/codex 的 hook payload 携带
/// `transcript_path` 指向 ~/.claude/projects/.../{session}.jsonl;cursor 用
/// camelCase。任何事件,有就 stash 进 payload,pokepoke 后续推送时读 jsonl
/// 抽 session 摘要。空就不带,server 端只在 Some(_) 时覆盖现有值。
fn add_transcript_path(payload: &mut Value, transcript_path: Option<&str>) {
    if let Some(p) = transcript_path {
        if !p.is_empty() {
            payload["transcript_path"] = Value::String(p.to_string());
        }
    }
}

/// DSH 是 GUI/Web 宿主:插件进程可能继承"启动 DSH 的那个终端"的 tty,但那
/// 不是会话所在的终端。一律不采 tty,否则会被当成终端会话跳到错误 tab
/// (brief 20261104-dsh-integration)。
fn tty_for_source(source: Source) -> Option<String> {
    if source == Source::Dsh {
        None
    } else {
        get_tty()
    }
}

/// DSH 的本地控制通道(host 半插件自建的 unix socket)。focus 时 PokePoke
/// 通过它请求渲染进程 `openSession`;没有它就只能退化成"只激活窗口"。
fn add_dsh_nav_socket(payload: &mut Value, source: Source) {
    if source != Source::Dsh {
        return;
    }
    if let Some(sock) = super::dsh::nav_socket_from_env() {
        payload["dsh_nav_socket"] = Value::String(sock);
    }
}

/// brief 20260527-ghostty-capture-retry:把 `terminal_app` 字段从 SessionStart
/// **专属** 扩展到所有用户交互事件(UserPromptSubmit / Notification /
/// PermissionRequest / Stop / StopFailure)。
///
/// 动机:b20520 当时只在 SessionStart 报 `terminal_app`,因为 capture 只在
/// SessionStart 触发一次。实测 SessionStart capture 命中率 72%(多 tab 并行 /
/// cc daemon resume / cwd-mismatch reject),server 端需要在后续事件再补一次
/// 幂等 capture —— 因此后续事件也必须带 `terminal_app` 让 server 知道这是
/// Ghostty session。
///
/// 空 env 不输出该字段(`terminal_app()` 返回 None),与 b20520 SessionStart
/// 行为对称,server 端 capture_eligible gate 兼容(None 也试探,见 http_server)。
///
/// Cursor 是 GUI 应用,不产生 TTY,server gate(`ghostty_capture_eligible`)已显式
/// 排除 Cursor,这里也跳过省一次 env 探测。DSH(GUI/Web 宿主)同理跳过。
fn add_terminal_app(payload: &mut Value, source: Source) {
    if matches!(source, Source::Cursor | Source::Dsh) {
        return;
    }
    if let Some(ta) = terminal_app() {
        payload["terminal_app"] = Value::String(ta);
    }
}

/// brief 20260709-{herdr,cmux}-jump-support + 20260724-wezterm:host-agent env
/// 从 SessionStart 专属放开到**所有交互事件**。
///
/// 动机:hook 部署时机不受控——用户 `pnpm install:hook` 或 app 自愈 deploy
/// 只覆盖到"下一次 hook fork"。已在跑的 CC 进程不会重发 SessionStart,
/// 只会继续发 UPS/Stop。当时若这两条不带 host env,server 端就永远拿不到
/// binding,回跳只能掉 ladder。任何事件都带 env,与 `add_terminal_app` 的
/// b20527 处理对称,server 端 `set_{herdr,cmux}_binding` 幂等,重复上报无副作用。
fn add_host_agent_env(payload: &mut Value, source: Source) {
    // DSH 进程环境里可能残留 herdr/cmux/tmux 变量(从终端启动 DSH 时),但会话
    // 不在那些 pane 里,误采会把回跳拽到无关终端。
    if source == Source::Dsh {
        return;
    }
    if let Some((pane, ws, sock)) = collect_herdr_env() {
        payload["herdr_pane_id"] = Value::String(pane);
        payload["herdr_workspace_id"] = Value::String(ws);
        payload["herdr_socket_path"] = Value::String(sock);
    } else if let Some((surface, ws)) = collect_cmux_env() {
        // herdr agent 在 herdr 内层 pty,外层宿主由 herdr client 决定;
        // 不与 cmux binding 混报(否则 iTerm2+herdr 会被残留 CMUX_* 拽去 cmux)。
        payload["cmux_surface_id"] = Value::String(surface);
        payload["cmux_workspace_id"] = Value::String(ws);
    } else if let Some((pane, sock, executable)) = collect_wezterm_env() {
        payload["wezterm_pane_id"] = Value::String(pane);
        payload["wezterm_unix_socket"] = Value::String(sock);
        if let Some(exe) = executable {
            payload["wezterm_executable"] = Value::String(exe);
        }
    } else if let Some(pane) = collect_otty_env() {
        payload["otty_pane_id"] = Value::String(pane);
    } else if let Some(tmux) = tmux_env::collect_tmux_env() {
        // tmux 优先级最低:herdr/cmux/wezterm/otty 任一命中都不采(agent 在
        // 内层宿主时,残留 TMUX_* 只可能是外层 shell,由更精确的 binding 决定)。
        payload["tmux_pane_id"] = Value::String(tmux.pane_id);
        payload["tmux_socket_path"] = Value::String(tmux.socket_path);
    }
    // Orca 外层宿主 hint:与 herdr/cmux 等并存,供 herdr-in-Orca 段 A 定位 + 消歧。
    // 不走 else-if,否则 herdr 在内层时 server 永远拿不到 orca_pane_key。
    if let Some(orca) = orca_env::collect_orca_env() {
        payload["orca_pane_key"] = Value::String(orca.pane_key);
        if !orca.worktree_id.is_empty() {
            payload["orca_worktree_id"] = Value::String(orca.worktree_id);
        }
        if let Some(tab) = orca.tab_id {
            payload["orca_tab_id"] = Value::String(tab);
        }
    }
}

pub(crate) fn handle_session_start(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    source: Source,
    transcript_path: Option<&str>,
) {
    subagents::clear_subagent_count(task_id);
    let tty = tty_for_source(source);
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "running",
        "title": project.to_string(),
        "message": format!("Session started\n{}", cwd),
        "source": source.as_str(),
        "status": "running",
        "workspace_path": cwd,
    });
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    add_host_agent_env(&mut payload, source);
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    // Ghostty 跳转兼容(brief 20260520-ghostty-terminal-focus):
    // 仅 SessionStart 上报 TERM_PROGRAM,触发 server 端 osascript capture
    // stable terminal id。其它事件不带,避免重 capture。空 env 不输出字段。
    // opencode 在终端里跑,与 cc/codex 一样带 terminal_app 走 Ghostty 跳转;
    // 仅 Cursor(GUI,无 TTY)被 add_terminal_app 内部按 source 跳过。
    add_terminal_app(&mut payload, source);
    add_dsh_nav_socket(&mut payload, source);
    add_transcript_path(&mut payload, transcript_path);
    post_notify(&payload);
}

/// 标识终端宿主,小写归一化。已知值:`ghostty` / `iterm.app` / `apple_terminal`
/// / `vscode` / `cursor` 等。空 / 未识别返回 None。server 端用
/// `terminal_app == "ghostty"` 触发 id capture。
///
/// 探测顺序(任一中即返回):
/// 1. `TERM_PROGRAM` —— 大多数终端的标准约定,iTerm2 / Terminal.app / VS Code
///    / Cursor / Ghostty 都设。**正常路径首选**。
/// 2. **Ghostty fallback**(实测踩坑):cc 在 resume / compact / background
///    reconnect 等场景从 daemon 进程 spawn hook 时,`TERM_PROGRAM` 经常丢
///    (env 来自 cc daemon 启动时的环境,不是用户当初终端的)。但 Ghostty 把
///    `TERM=xterm-ghostty`(终端 terminfo)、`GHOSTTY_RESOURCES_DIR`、
///    `GHOSTTY_SHELL_FEATURES` 都 export 进 shell 启动 env,这些常驻信号
///    更不易被剥。任一中即认 ghostty。
///
/// 注意:Apple Terminal 也有自己的 `TERM=xterm-256color`,但没专属 env,
/// 所以 step 2 只覆盖 Ghostty。其它终端的兼容缺口出现再加。
pub(crate) fn terminal_app() -> Option<String> {
    // brief 20260709-cmux-jump-support:cmux 用 libghostty 渲染,pane 内
    // `TERM_PROGRAM=ghostty` 会让 server 端误跑真 Ghostty AppleScript capture
    // (枚举不到 cmux surface,退 cwd first-match 撞无关 Ghostty 窗口)。
    // `CMUX_SURFACE_ID` 存在即认定 cmux 宿主,报回 "cmux";server 端 Ghostty
    // capture 的 gate 只匹配 `ghostty` 字符串,天然过掉。cmux 里的精准跳回
    // 走 cmux short-circuit(参见 focus_cmux_surface)。
    if std::env::var("CMUX_SURFACE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return Some("cmux".to_string());
    }
    if let Some(s) = env::var("TERM_PROGRAM")
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        return Some(s);
    }
    // Warp 兜底(brief 20260810-warp-terminal-support):Warp 设置
    // `TERM_PROGRAM=WarpTerminal`(正常路径已命中),但 cc / codex 从 daemon
    // 进程 spawn hook 时 TERM_PROGRAM 经常丢(Ghostty 同款坑)。Warp 起 shell
    // 时固定 export `WARP_IS_LOCAL_SHELL_SESSION=1`(unix.rs:383),常驻信号更
    // 不易被剥。任一在即认 warpterminal。
    if env::var("WARP_IS_LOCAL_SHELL_SESSION")
        .ok()
        .filter(|s| s == "1")
        .is_some()
    {
        return Some("warpterminal".to_string());
    }
    // Ghostty 兜底:任一专属 env 在即识别为 ghostty
    let is_ghostty = env::var("TERM")
        .map(|t| t.eq_ignore_ascii_case("xterm-ghostty"))
        .unwrap_or(false)
        || env::var("GHOSTTY_RESOURCES_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        || env::var("GHOSTTY_SHELL_FEATURES")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some();
    if is_ghostty {
        return Some("ghostty".to_string());
    }
    None
}

pub(crate) fn handle_user_prompt_submit(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    source: Source,
    transcript_path: Option<&str>,
) {
    let lock_file = flag_path(task_id, "registered");

    // Clear pending flag if set
    let pending_flag = flag_path(task_id, "pending");
    if pending_flag.exists() {
        let _ = fs::remove_file(&pending_flag);
    }

    if lock_file.exists() {
        let mut payload = serde_json::json!({
            "task_id": task_id,
            "event_type": "running",
            "title": project.to_string(),
            "message": format!("Working...\n{}", cwd),
            "source": source.as_str(),
            "status": "running",
        });
        if let Some(external_session_id) = external_session_id {
            payload["external_session_id"] = Value::String(external_session_id.to_string());
        }
        add_host_agent_env(&mut payload, source);
        add_transcript_path(&mut payload, transcript_path);
        add_terminal_app(&mut payload, source);
        add_dsh_nav_socket(&mut payload, source);
        post_notify(&payload);
        return;
    }

    let _ = fs::write(&lock_file, std::process::id().to_string());

    let tty = tty_for_source(source);
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "running",
        "title": project.to_string(),
        "message": format!("Working...\n{}", cwd),
        "source": source.as_str(),
        "status": "running",
        "workspace_path": cwd,
    });
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    add_host_agent_env(&mut payload, source);
    add_transcript_path(&mut payload, transcript_path);
    add_terminal_app(&mut payload, source);
    add_dsh_nav_socket(&mut payload, source);
    post_notify(&payload);
}

pub(crate) fn handle_notification(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    message: &str,
    cwd: &str,
    source: Source,
    transcript_path: Option<&str>,
) {
    let flag = flag_path(task_id, "pending");
    let _ = fs::write(&flag, message);

    let tty = tty_for_source(source);
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "pending",
        "title": project.to_string(),
        "message": message,
        "source": source.as_str(),
        "status": "pending",
        "workspace_path": cwd,
    });
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    add_host_agent_env(&mut payload, source);
    add_transcript_path(&mut payload, transcript_path);
    add_terminal_app(&mut payload, source);
    add_dsh_nav_socket(&mut payload, source);
    post_notify(&payload);
}

#[allow(clippy::too_many_arguments)] // 与 handle_stop_failure / handle_session_start 同族 handler 对称,payload 字段直传
pub(crate) fn handle_stop(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    source: Source,
    transcript_path: Option<&str>,
    suppress: bool,
    aborted: bool,
) {
    let _ = fs::remove_file(flag_path(task_id, "registered"));
    let _ = fs::remove_file(flag_path(task_id, "pending"));

    // 后台仍有任务在跑(子 agent / 后台 shell):Stop 只是 turn 边界、不是完成。
    // 不 post → server 端 session 保持 Running(由前序 UPS/SessionStart 设),不弹"完成"。
    // flag 照常清理(保持 registered/pending 状态机与现状一致)。background_tasks
    // 缺失/全完成 → suppress=false,照常发 idle。详见 memory: project_stop_background_tasks_signal。
    if suppress {
        return;
    }

    subagents::clear_subagent_count(task_id);

    let tty = tty_for_source(source);
    // Task C 新语义：Stop 事件表示"这一轮结束，agent 仍活着"，对应 Idle 状态。
    // 后端仍兼容老字符串 "success" via serde alias，但新 binary 统一发 "idle"。
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "stop",
        "title": project.to_string(),
        "message": "Session completed",
        "source": source.as_str(),
        "status": "idle",
        "workspace_path": cwd,
    });
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    // T1(brief 20260805-t1-cursor-aborted)扩展到 pi(brief 20260820-pi-aborted-no-completion-sound):
    // 用户主动取消(aborted)带标记,server 据此写 aborted_at + 抑制"完成"音,
    // 并识别同轮 last_failed artifact echo。正常完成不带该字段。
    if aborted {
        payload["aborted"] = Value::Bool(true);
    }
    add_host_agent_env(&mut payload, source);
    add_transcript_path(&mut payload, transcript_path);
    add_terminal_app(&mut payload, source);
    add_dsh_nav_socket(&mut payload, source);
    post_notify(&payload);
}

#[allow(clippy::too_many_arguments)] // 与 handle_session_start / handle_stop 同族 handler 对称,payload 字段直传
pub(crate) fn handle_stop_failure(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    source: Source,
    reason: &str,
    detail_message: &str,
    transcript_path: Option<&str>,
) {
    // 与 handle_stop 一样清理 flag，StopFailure 同样是"这一轮结束"的信号
    let _ = fs::remove_file(flag_path(task_id, "registered"));
    let _ = fs::remove_file(flag_path(task_id, "pending"));
    subagents::clear_subagent_count(task_id);

    let tty = tty_for_source(source);
    // failure_reason 走 i18n；message 存 CC last_assistant_message，供 unknown 兜底展示。
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "stop",
        "title": project.to_string(),
        "message": detail_message,
        "source": source.as_str(),
        "status": "last_failed",
        "workspace_path": cwd,
        "failure_reason": reason,
    });
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    add_host_agent_env(&mut payload, source);
    add_transcript_path(&mut payload, transcript_path);
    add_terminal_app(&mut payload, source);
    add_dsh_nav_socket(&mut payload, source);
    post_notify(&payload);
}

pub(crate) fn handle_cursor_stop(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    hook_status: &str,
    suppress: bool,
) {
    let _ = fs::remove_file(flag_path(task_id, "registered"));
    let _ = fs::remove_file(flag_path(task_id, "pending"));

    // Cursor 的结束分支(completed/aborted/error):
    // - completed/aborted → Idle(aborted 是用户主动中止,不是失败)
    // - error 或 hookStatus 缺失/未知 → LastFailed + failure_reason=api_error,
    //   走 server 现有 last_failed 链路(红色 popup + builtin:failed 音)。
    //   历史实现三分支全 idle,error 分支播"完成"音,与事实相反
    //   (integration-gaps P2-①);hookStatus 缺失曾默认 completed,错误被报成
    //   "完成",一并修正(grok review §2.4)。
    // Cursor subagent(SubagentStart/SubagentStop 计数 >0)仍跑时 main stop
    // 只是 turn 边界 → suppress(对齐 Kimi,2026-08-04 grok review §4-②)。
    if suppress {
        return;
    }
    subagents::clear_subagent_count(task_id);

    let (status, msg) = match hook_status {
        "completed" => ("idle", "Agent turn completed"),
        "aborted" => ("idle", "Agent turn aborted"),
        _ => ("last_failed", "Agent turn ended with error"),
    };

    // 对称地与 handle_stop / handle_session_start 保持一致：Cursor stop 也
    // 要带上 tty。PokePoke 若在 Cursor 会话开始之后才启动、错过了
    // SessionStart/UserPromptSubmit，这一刻就是它第一次有机会把 tty 写进
    // session；漏抓的话后续点击 popup 会永远降级到 CursorGui 档。
    // 详见 docs/briefs/20260420-cursor-stop-tty.md。
    let tty = get_tty();
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "stop",
        "title": project.to_string(),
        "message": msg,
        "source": "cursor",
        "status": status,
        "workspace_path": cwd,
    });
    if status == "last_failed" {
        payload["failure_reason"] = Value::String("api_error".into());
    }
    // T1(brief 20260805-t1-cursor-aborted):aborted(用户取消)带标记,
    // server 据此写 aborted_at + 抑制"完成"音,并识别 40ms 后同轮双发
    // 的 last_failed echo。completed/error 分支不带该字段。
    if hook_status == "aborted" {
        payload["aborted"] = Value::Bool(true);
    }
    if let Some(ref t) = tty {
        payload["terminal_tty"] = Value::String(t.clone());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    add_host_agent_env(&mut payload, Source::Cursor);
    post_notify(&payload);
}

pub(crate) fn session_end_payload(
    task_id: &str,
    project: &str,
    cwd: &str,
    source: Source,
    tty: Option<&str>,
    external_session_id: Option<&str>,
) -> Value {
    let mut payload = serde_json::json!({
        "task_id": task_id,
        "event_type": "session_end",
        "title": project.to_string(),
        "message": "Session ended",
        "source": source.as_str(),
        "status": "idle",
        "workspace_path": cwd,
    });
    if let Some(t) = tty {
        payload["terminal_tty"] = Value::String(t.to_string());
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id.to_string());
    }
    payload
}

pub(crate) fn handle_session_end(
    task_id: &str,
    external_session_id: Option<&str>,
    project: &str,
    cwd: &str,
    source: Source,
) {
    let _ = fs::remove_file(flag_path(task_id, "registered"));
    let _ = fs::remove_file(flag_path(task_id, "pending"));
    subagents::clear_subagent_count(task_id);

    // 和 handle_cursor_stop 对称补 tty：SessionEnd 可能是 PokePoke 观测到的
    // 这条 session 的第一个（也是唯一一个）事件，缺了 tty 后续没有补抓机会。
    // 详见 docs/briefs/20260420-cursor-stop-tty.md。
    let tty = tty_for_source(source);
    let mut payload = session_end_payload(
        task_id,
        project,
        cwd,
        source,
        tty.as_deref(),
        external_session_id,
    );
    add_host_agent_env(&mut payload, source);
    post_notify(&payload);
}
