use crate::sessions::{SessionStatus, SessionStore};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

pub(crate) mod cli_timeout;
mod short_circuit;
use short_circuit::{run_terminal_short_circuits, SessionFocusBindings};

mod applescript;
use applescript::is_app_running;

mod iterm2;
use iterm2::focus_iterm2;

mod terminal_app;
use terminal_app::focus_terminal_app;

mod codex;
use codex::{focus_codex_desktop, focus_cursor_workspace};

mod dsh;
pub(crate) use dsh::{dsh_session_is_focused, focus_dsh_session, DshFocusOutcome};

mod cmux;

mod wezterm;

mod warp;
use warp::focus_warp_session;
pub(crate) use warp::frontmost_is_warp;

mod tmux;

mod otty;
pub(crate) use otty::{
    apply_otty_binding, decide_otty_capture_reason, diagnose_focus_mismatch,
    otty_bound_pane_focused, otty_selected_tab_has_tty, spawn_otty_capture,
};
mod orca;
pub(crate) use orca::{
    apply_orca_binding, apply_orca_binding_for_herdr_host_if_missing, decide_orca_capture_reason,
    orca_bound_pane_focused, orca_herdr_host_focused, spawn_orca_capture,
};
mod ghostty;
mod ghostty_capture;
mod ghostty_herdr;
mod ghostty_tty;
use ghostty::{focus_ghostty, focus_ghostty_for_session};
pub use ghostty_capture::{capture_ghostty_focused_terminal_id, cwds_compatible};
pub(crate) use ghostty_herdr::is_ghostty_herdr_tab_focused_for_socket;
use ghostty_tty::focus_ghostty_by_tty;

mod herdr;
use herdr::focus_herdr_if_bound;
pub(crate) use herdr::{
    client_tty_belongs_to_ghostty, client_tty_belongs_to_wezterm, focus_herdr_remote_client_window,
    resolve_herdr_client_tty,
};

/// 跳转目标的四种可能。由 `decide_focus_target` 纯函数决策，
/// command 入口只负责执行对应副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FocusTarget {
    /// 精确定位终端 tab（iTerm2 / Terminal.app），tty 匹配失败时 command
    /// 入口会根据 source 走 `fallback_after_terminal_miss`（Cursor → GUI；
    /// CC/Codex → no-op,不再打开 Finder）。
    Terminal { tty: String },
    /// 精确定位 Ghostty terminal surface（stable terminal id）。
    Ghostty { terminal_id: String },
    /// ChatGPT 桌面端会话:`open codex://threads/<thread_id>` deep link 拉起
    /// ChatGPT.app(=`com.openai.codex`,Electron)并切到那条具体对话。codex 桌面端
    /// 会话无 tty,external_session_id(codex thread UUID)是唯一身份。
    CodexDesktop { thread_id: String },
    /// DSH 会话:`openSession` 走 host 半插件的本地 socket(无 deep link)。
    DshSession { session_id: String },
    /// Cursor GUI：`cursor <workspace_path>` 复用或拉起编辑器窗口。
    CursorGui { path: String },
    /// 其它 source 无 tty 时的 last-resort 语义标签:曾经会 `open <path>` 打开
    /// Finder,现在改成 no-op —— 用户跳转的意图是"回到那个 session",落到
    /// Finder 是错答案,不如让 popup/panel 直接关掉。保留变体主要为日志分档,
    /// 方便复盘"我为什么没跳"。
    Finder { path: String },
    /// 全部字段缺失，什么都做不了。
    Nothing,
}

/// 纯函数：根据 session 的 (source, tty, workspace_path) 决定跳转目的地。
///
/// 分档规则（只读参数，无 IO、无副作用）：
/// - 任意 source：有非空 tty → `Terminal`（AppleScript 精确匹配）
/// - 无 tty + source=cursor：`CursorGui`（`cursor <path>` 拉 GUI）
/// - 无 tty + 其它 source：`Finder`（保留档位供日志区分,实际执行为 no-op —
///   曾经会 `open <path>` 打开 Finder,后来觉得跳到 Finder 是错答案,现在直接
///   让 popup/panel 关闭。见 `FocusTarget::Finder` 的 doc。）
/// - 无 tty 且无 path：`Nothing`
///
/// Cursor 之所以要分档，是因为宿主有两种：GUI 聊天面板（hook tty=None）
/// 与 iTerm2 里跑的 `cursor-agent` CLI（hook 能抓到 tty）。
pub(crate) fn decide_focus_target(
    source: Option<&str>,
    terminal_tty: Option<&str>,
    workspace_path: Option<&str>,
    terminal_id: Option<&str>,
    external_session_id: Option<&str>,
) -> FocusTarget {
    use crate::surface::{log_resolution, resolve_surface, SurfaceEvidence, SurfaceKind};

    let evidence = SurfaceEvidence {
        source: source.map(str::to_string),
        terminal_app: None,
        dsh_nav_socket: None,
        task_id: String::new(),
        session_id: String::new(),
        external_session_id: external_session_id.map(str::to_string),
        workspace_path: workspace_path.map(str::to_string),
        terminal_tty: terminal_tty.map(str::to_string),
        terminal_id: terminal_id.map(str::to_string),
        terminal_id_captured_cwd: None,
        herdr_pane_id: None,
        herdr_socket_path: None,
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
    let resolved = resolve_surface(&evidence);
    log_resolution("surface", "focus_target", &evidence, &resolved);

    match resolved.kind {
        SurfaceKind::TerminalTty => FocusTarget::Terminal {
            tty: terminal_tty.unwrap_or_default().to_string(),
        },
        SurfaceKind::GhosttyTerminal => FocusTarget::Ghostty {
            terminal_id: terminal_id.unwrap_or_default().to_string(),
        },
        SurfaceKind::DshSession => FocusTarget::DshSession {
            session_id: external_session_id.unwrap_or_default().to_string(),
        },
        SurfaceKind::CodexDesktop => FocusTarget::CodexDesktop {
            thread_id: external_session_id.unwrap_or_default().to_string(),
        },
        SurfaceKind::CursorWorkspace => FocusTarget::CursorGui {
            path: workspace_path.unwrap_or_default().to_string(),
        },
        SurfaceKind::FinderWorkspace => FocusTarget::Finder {
            path: workspace_path.unwrap_or_default().to_string(),
        },
        SurfaceKind::OttyTerminal
        | SurfaceKind::OrcaTerminal
        | SurfaceKind::HerdrPane
        | SurfaceKind::Unknown => FocusTarget::Nothing,
    }
}

/// Terminal tty 定位失败后的降级：Cursor 回 GUI，其它 source 回 Finder。
/// 仅当 workspace_path 非空时有意义，否则什么都不做。
pub(super) fn log_path_hint(path: Option<&str>) -> &'static str {
    if path.is_some_and(|p| !p.is_empty()) {
        "present"
    } else {
        "absent"
    }
}

pub(super) fn terminal_id_hint(terminal_id: Option<&str>) -> String {
    let Some(id) = terminal_id.filter(|id| !id.is_empty()) else {
        return "absent".to_string();
    };
    let preview: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        format!("{}…", preview)
    } else {
        preview
    }
}

fn fallback_after_terminal_miss(source: Option<&str>, workspace_path: Option<&str>) {
    let Some(path) = workspace_path.filter(|p| !p.is_empty()) else {
        return;
    };
    match source {
        Some("cursor") => {
            crate::log_debug!(
                "focus",
                "terminal tty miss -> fallback CursorGui workspace_path_present=true"
            );
            focus_cursor_workspace(path);
        }
        _ => {
            // 兜底动作已下线:CC/Codex 等 source 定位不到终端时,不再 `open <path>`
            // 打开 Finder —— 用户点跳转的意图是"回到那个 session",落到 Finder
            // 是错答案而不是有用的兜底。什么都不做,让上层的 popup/panel 自然
            // 关闭即可。这里只留一行日志,方便复盘"为什么没跳"。
            crate::log_debug!(
                "focus",
                "terminal tty miss -> no fallback (Finder disabled) workspace_path_present=true source={:?}",
                source
            );
        }
    }
}

pub(super) fn clear_stale_terminal_id(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    terminal_id: &str,
) {
    let cleared = {
        let mut s = store.lock().unwrap_or_else(|p| p.into_inner());
        s.clear_terminal_id_if_matches(session_id, terminal_id)
    };
    crate::log_info!(
        "focus",
        "cleared stale terminal_id session_id={} terminal_id_hint={} cleared={}",
        session_id,
        terminal_id_hint(Some(terminal_id)),
        cleared
    );
}

/// Focus the terminal that matches the session's tty.
/// Does NOT close the popup — the frontend handles that separately
/// to avoid destroying the calling webview mid-command.
///
/// 跳转成功后两手准备:
/// - mark `RecentlyVisited`:30s 短窗口,挡 hook 几秒级被动重传
/// - mark `AckedStages` (id, current_status):用户认领该 session 的当前 stage,
///   同 stage 重弹一直被抑制,直到该 session 至少经过一次 Running 才放行
///
/// 2026-07-01(主线程挂死事故):本命令改 `async` + `spawn_blocking`。同步
/// `#[tauri::command]` 在**主线程**跑,内部一连串 osascript(枚举 iTerm2/Terminal/
/// Ghostty 全部窗口 + 重试)只要目标 App 卡住就把主线程钉死,popup 建/销全部饿死
/// → 整个 UI 卡死。改 async 后命令体在 tokio 上跑,阻塞的 osascript 再丢进
/// blocking 池(对齐 http_server 的 spawn_ghostty_capture 范式)。State 是借用参数,
/// async 命令按 Tauri 文档必须返回 `Result` 并先 clone 出内部 Arc 再 move 进闭包。
#[tauri::command]
pub async fn focus_session_terminal(
    id: String,
    store: State<'_, Arc<Mutex<SessionStore>>>,
    memory: State<'_, Arc<crate::http_server::SessionMemory>>,
) -> Result<(), ()> {
    let store = store.inner().clone();
    let memory = memory.inner().clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        focus_session_terminal_blocking(id, store, memory);
    })
    .await;
    Ok(())
}

fn focus_session_terminal_blocking(
    id: String,
    store: Arc<Mutex<SessionStore>>,
    memory: Arc<crate::http_server::SessionMemory>,
) {
    let (
        terminal_tty,
        workspace_path,
        source,
        external_session_id,
        current_status,
        terminal_id,
        terminal_id_captured_cwd,
        herdr_pane_id,
        herdr_socket_path,
        herdr_workspace_id,
        cmux_surface_id,
        cmux_workspace_id,
        wezterm_pane_id,
        wezterm_unix_socket,
        wezterm_executable,
        orca_pane_key,
        orca_worktree_id,
        otty_pane_id,
        otty_tab_id,
        tmux_pane_id,
        tmux_socket_path,
        terminal_app,
        dsh_nav_socket,
    ) = {
        let s = store.lock().unwrap_or_else(|p| p.into_inner());
        let session = s.get_all().iter().find(|s| s.id == id).cloned();
        match session {
            Some(s) => (
                s.terminal_tty.clone(),
                s.workspace_path.clone(),
                s.source.clone(),
                s.external_session_id.clone(),
                s.status.clone(),
                s.terminal_id.clone(),
                s.terminal_id_captured_cwd.clone(),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .map(|(pane, _, _)| pane.to_string()),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .and_then(|(_, _, sock)| sock.map(str::to_string)),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .and_then(|(_, ws, _)| ws.map(str::to_string)),
                match &s.surface {
                    crate::surface::SurfaceBinding::Cmux { surface_id, .. } => {
                        Some(surface_id.clone())
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Cmux { workspace_id, .. } => {
                        workspace_id.clone()
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { pane_id, .. } => {
                        Some(pane_id.clone())
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { unix_socket, .. } => {
                        unix_socket.clone()
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { executable, .. } => {
                        executable.clone()
                    }
                    _ => None,
                },
                crate::surface::SurfaceBinding::orca_fields(&s).map(|(pk, _, _)| pk.to_string()),
                crate::surface::SurfaceBinding::orca_fields(&s)
                    .and_then(|(_, wt, _)| wt.map(str::to_string)),
                match &s.surface {
                    crate::surface::SurfaceBinding::Otty { pane_id, .. } => Some(pane_id.clone()),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Otty { tab_id, .. } => tab_id.clone(),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Tmux { pane_id, .. } => Some(pane_id.clone()),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Tmux { socket_path, .. } => socket_path.clone(),
                    _ => None,
                },
                s.terminal_app.clone(),
                s.dsh_nav_socket.clone(),
            ),
            None => {
                crate::log_debug!(
                    "focus",
                    "focus_session_terminal: session not found id={}",
                    id
                );
                return;
            }
        }
    };

    // Herdr → cmux → wezterm → orca → otty short-circuit 链(共用 helper)。
    let sc = run_terminal_short_circuits(
        &store,
        &id,
        &SessionFocusBindings {
            workspace_path: workspace_path.clone(),
            terminal_tty: terminal_tty.clone(),
            herdr_pane_id: herdr_pane_id.clone(),
            herdr_socket_path: herdr_socket_path.clone(),
            is_herdr_remote: crate::surface::is_herdr_remote_source(source.as_deref()),
            cmux_surface_id: cmux_surface_id.clone(),
            cmux_workspace_id: cmux_workspace_id.clone(),
            wezterm_pane_id: wezterm_pane_id.clone(),
            wezterm_unix_socket: wezterm_unix_socket.clone(),
            wezterm_executable: wezterm_executable.clone(),
            orca_pane_key: orca_pane_key.clone(),
            orca_worktree_id: orca_worktree_id.clone(),
            otty_pane_id: otty_pane_id.clone(),
            otty_tab_id: otty_tab_id.clone(),
            tmux_pane_id: tmux_pane_id.clone(),
            tmux_socket_path: tmux_socket_path.clone(),
        },
    );
    let did_herdr_short_circuit = sc.did_herdr;
    let did_cmux_short_circuit = sc.did_cmux;
    let did_wezterm_short_circuit = sc.did_wezterm;
    let did_orca_short_circuit = sc.did_orca;
    let did_otty_short_circuit = sc.did_otty;

    // Warp 回跳(brief 20260810-warp-support):session 宿主是 Warp(terminal_app
    // 持久化)就走 warp 分支。Warp 无 AppleScript 可枚举,ladder 在 Warp 会话上
    // 必然落空,反而可能误吸同 cwd 的其它终端窗口(iTerm2 / Ghostty),所以
    // 存在即 gate ladder。
    let is_warp_host = terminal_app.as_deref() == Some("warpterminal");
    if is_warp_host {
        focus_warp_session();
    }

    // herdr / orca binding 存在时禁止落 target ladder —— hook tty 是 agent 内层 pty,
    // ladder 会误吸 Ghostty cwd 匹配(如 /Users/edy)到 default herdr tab,或把 Orca
    // 里的 Cursor CLI 错跳到 Ghostty / Cursor GUI。
    if herdr_pane_id.is_none()
        && orca_pane_key.is_none()
        && !is_warp_host
        && !did_cmux_short_circuit
        && !did_herdr_short_circuit
        && !did_wezterm_short_circuit
        && !did_otty_short_circuit
        && !did_orca_short_circuit
    {
        let target = decide_focus_target(
            source.as_deref(),
            terminal_tty.as_deref(),
            workspace_path.as_deref(),
            terminal_id.as_deref(),
            external_session_id.as_deref(),
        );
        crate::log_debug!("focus", "focus_session_terminal id={} -> {:?}", id, target);

        match target {
            FocusTarget::Terminal { tty } => {
                let r = focus_terminal(
                    &tty,
                    terminal_id.as_deref(),
                    terminal_id_captured_cwd.as_deref(),
                    workspace_path.as_deref(),
                );
                // stale id 自愈:focus_ghostty L1 验 cwd 不通过 → 反向清掉脏 id,
                // 下次 click 自然走 L2,不再稳定跳错。
                if r.invalidate_id {
                    if let Some(old_id) = terminal_id.as_deref() {
                        clear_stale_terminal_id(&store, &id, old_id);
                    }
                }
                if !r.focused {
                    fallback_after_terminal_miss(source.as_deref(), workspace_path.as_deref());
                }
            }
            FocusTarget::Ghostty { terminal_id } => {
                let r = focus_ghostty_for_session(
                    &store,
                    &id,
                    &terminal_id,
                    terminal_id_captured_cwd.as_deref(),
                    workspace_path.as_deref(),
                    "focus_session_terminal",
                );
                if !r.focused {
                    fallback_after_terminal_miss(source.as_deref(), workspace_path.as_deref());
                }
            }
            FocusTarget::CodexDesktop { thread_id } => {
                focus_codex_desktop(&thread_id);
            }
            FocusTarget::DshSession { session_id } => {
                let outcome = focus_dsh_session(&session_id, dsh_nav_socket.as_deref());
                crate::log_info!(
                    "focus",
                    "focus_session_terminal dsh session_id={} outcome={:?}",
                    terminal_id_hint(Some(&session_id)),
                    outcome
                );
            }
            FocusTarget::CursorGui { path } => focus_cursor_workspace(&path),
            FocusTarget::Finder { path: _ } => {
                // 兜底动作已下线,见 fallback_after_terminal_miss 的说明。
                crate::log_debug!(
                    "focus",
                    "focus_session_terminal: FocusTarget::Finder no-op (fallback disabled)"
                );
            }
            FocusTarget::Nothing => {}
        }
    }

    // 段 C(herdr-remote):段 B 之前先激活承载本地 herdr client 的终端窗口
    // (段 B 只改远程 server 状态,本地窗口不激活用户看不到任何动静)。outcome
    // 分型(Precise / AppActivatedOnly / Failed)带 session_id 落日志,排查
    // "停错 tab"时不被 activate-only 误报成精确命中。失败只记日志,继续段 B。
    if crate::surface::is_herdr_remote_source(source.as_deref()) {
        let outcome = focus_herdr_remote_client_window(herdr_socket_path.as_deref());
        crate::log_info!(
            "focus",
            "focus_session_terminal herdr-remote 段C session_id={} outcome={:?}",
            id,
            outcome
        );
    }

    // 段 B:段 A 完成后如果 session 绑了 herdr pane,让 herdr 内部切到目标 pane。
    // 不管段 A 落到 Terminal / Ghostty / 兜底,只要 binding 有值就试 —— 三件套就是
    // "CC 跑在 herdr pane"的判据。段 B 失败按 source 分型处理(见 focus_herdr_if_bound)。
    if let (Some(pane), Some(sock)) = (herdr_pane_id.as_deref(), herdr_socket_path.as_deref()) {
        let _ = focus_herdr_if_bound(
            &store,
            &id,
            pane,
            sock,
            herdr_workspace_id.as_deref(),
            crate::surface::is_herdr_remote_source(source.as_deref()),
        );
    }

    // mark 不分跳转结果:即便 AppleScript 失败 / fallback 到 Finder,用户的
    // 意图就是"我去看这个 session 了"。两道 mark:
    // - recently_visited(30s 短窗):Running/Pending 才 mark,挡片刻后的 hook 重传。
    //   Idle 不 mark:轮次正常结束,用户跳回看结果后可能立刻开始下一轮;
    //   若 mark 了 Idle,30s 内下一轮结束的 popup 会被 recently_visited 误拦。
    // - acked_stages:认领当前 stage,直到下次 Running 才放行
    if current_status == SessionStatus::Running || current_status == SessionStatus::Pending {
        memory.mark_visited(&id);
    }
    memory.mark_acked(&id, current_status);
}

/// Focus the terminal window that spawned the session (used by panel clicks).
///
/// 命令尾部统一调 `auto_hide_panel`：osascript 跨进程激活别的 app 时，
/// macOS 不一定会给 Accessory + always-on-top 的 panel 派发 `resignKey`，
/// Tauri 的 `WindowEvent::Focused(false)` 也就不触发，前端 `window.blur`
/// 同样收不到 → 失焦自动关闭那条链整条断掉，panel 卡死在屏幕上。
/// 用户点 session 行的语义就是"跳走"，与其等 OS 焦点机制，不如服务端
/// 直接把 panel 收掉。
#[tauri::command]
pub async fn open_session_source(
    app: AppHandle,
    id: String,
    store: State<'_, Arc<Mutex<SessionStore>>>,
) -> Result<(), ()> {
    let store = store.inner().clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        open_session_source_blocking(app, id, store);
    })
    .await;
    Ok(())
}

fn open_session_source_blocking(app: AppHandle, id: String, store: Arc<Mutex<SessionStore>>) {
    let (
        terminal_tty,
        workspace_path,
        source,
        external_session_id,
        terminal_id,
        terminal_id_captured_cwd,
        herdr_pane_id,
        herdr_socket_path,
        herdr_workspace_id,
        cmux_surface_id,
        cmux_workspace_id,
        wezterm_pane_id,
        wezterm_unix_socket,
        wezterm_executable,
        orca_pane_key,
        orca_worktree_id,
        otty_pane_id,
        otty_tab_id,
        tmux_pane_id,
        tmux_socket_path,
        terminal_app,
        dsh_nav_socket,
    ) = {
        let s = store.lock().unwrap_or_else(|p| p.into_inner());
        let session = s.get_all().iter().find(|s| s.id == id).cloned();
        match session {
            Some(s) => (
                s.terminal_tty.clone(),
                s.workspace_path.clone(),
                s.source.clone(),
                s.external_session_id.clone(),
                s.terminal_id.clone(),
                s.terminal_id_captured_cwd.clone(),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .map(|(pane, _, _)| pane.to_string()),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .and_then(|(_, _, sock)| sock.map(str::to_string)),
                crate::surface::SurfaceBinding::herdr_fields(&s)
                    .and_then(|(_, ws, _)| ws.map(str::to_string)),
                match &s.surface {
                    crate::surface::SurfaceBinding::Cmux { surface_id, .. } => {
                        Some(surface_id.clone())
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Cmux { workspace_id, .. } => {
                        workspace_id.clone()
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { pane_id, .. } => {
                        Some(pane_id.clone())
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { unix_socket, .. } => {
                        unix_socket.clone()
                    }
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Wezterm { executable, .. } => {
                        executable.clone()
                    }
                    _ => None,
                },
                crate::surface::SurfaceBinding::orca_fields(&s).map(|(pk, _, _)| pk.to_string()),
                crate::surface::SurfaceBinding::orca_fields(&s)
                    .and_then(|(_, wt, _)| wt.map(str::to_string)),
                match &s.surface {
                    crate::surface::SurfaceBinding::Otty { pane_id, .. } => Some(pane_id.clone()),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Otty { tab_id, .. } => tab_id.clone(),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Tmux { pane_id, .. } => Some(pane_id.clone()),
                    _ => None,
                },
                match &s.surface {
                    crate::surface::SurfaceBinding::Tmux { socket_path, .. } => socket_path.clone(),
                    _ => None,
                },
                s.terminal_app.clone(),
                s.dsh_nav_socket.clone(),
            ),
            None => {
                crate::log_debug!("focus", "open_session_source: session not found id={}", id);
                return;
            }
        }
    };

    // Herdr → cmux → wezterm → orca → otty short-circuit 链(镜像 focus_session_terminal)。
    let sc = run_terminal_short_circuits(
        &store,
        &id,
        &SessionFocusBindings {
            workspace_path: workspace_path.clone(),
            terminal_tty: terminal_tty.clone(),
            herdr_pane_id: herdr_pane_id.clone(),
            herdr_socket_path: herdr_socket_path.clone(),
            is_herdr_remote: crate::surface::is_herdr_remote_source(source.as_deref()),
            cmux_surface_id: cmux_surface_id.clone(),
            cmux_workspace_id: cmux_workspace_id.clone(),
            wezterm_pane_id: wezterm_pane_id.clone(),
            wezterm_unix_socket: wezterm_unix_socket.clone(),
            wezterm_executable: wezterm_executable.clone(),
            orca_pane_key: orca_pane_key.clone(),
            orca_worktree_id: orca_worktree_id.clone(),
            otty_pane_id: otty_pane_id.clone(),
            otty_tab_id: otty_tab_id.clone(),
            tmux_pane_id: tmux_pane_id.clone(),
            tmux_socket_path: tmux_socket_path.clone(),
        },
    );
    let did_herdr_short_circuit = sc.did_herdr;
    let did_cmux_short_circuit = sc.did_cmux;
    let did_wezterm_short_circuit = sc.did_wezterm;
    let did_orca_short_circuit = sc.did_orca;
    let did_otty_short_circuit = sc.did_otty;

    let reason = if did_herdr_short_circuit {
        "open_session_source/herdr_anchor"
    } else if did_cmux_short_circuit {
        "open_session_source/cmux_focus_panel"
    } else if did_wezterm_short_circuit {
        "open_session_source/wezterm_activate_pane"
    } else if did_orca_short_circuit {
        "open_session_source/orca_focus_pane"
    } else if did_otty_short_circuit {
        "open_session_source/otty_focus_pane"
    } else if herdr_pane_id.is_some() {
        "open_session_source/herdr_anchor_miss"
    } else if orca_pane_key.is_some() {
        "open_session_source/orca_focus_miss"
    } else if terminal_app.as_deref() == Some("warpterminal") {
        // brief 20260810-warp-support:Warp 会话走 warp 分支(activate-only,
        // local_control 不可用降 AppleScript);reason 记 ok/miss 供日志排查。
        let ok = focus_warp_session();
        if ok {
            "open_session_source/warp_activate"
        } else {
            "open_session_source/warp_miss"
        }
    } else {
        let target = decide_focus_target(
            source.as_deref(),
            terminal_tty.as_deref(),
            workspace_path.as_deref(),
            terminal_id.as_deref(),
            external_session_id.as_deref(),
        );
        crate::log_debug!(
            "focus",
            "open_session_source id={} source={:?} tty={:?} workspace_path={} terminal_id_hint={} captured_cwd={} -> {:?}",
            id,
            source,
            terminal_tty,
            log_path_hint(workspace_path.as_deref()),
            terminal_id_hint(terminal_id.as_deref()),
            log_path_hint(terminal_id_captured_cwd.as_deref()),
            target
        );

        match &target {
            FocusTarget::Terminal { tty } => {
                let r = focus_terminal(
                    tty,
                    terminal_id.as_deref(),
                    terminal_id_captured_cwd.as_deref(),
                    workspace_path.as_deref(),
                );
                crate::log_debug!(
                    "focus",
                    "focus_terminal({}) -> focused={} invalidate_id={}",
                    tty,
                    r.focused,
                    r.invalidate_id
                );
                // 镜像 focus_session_terminal 的 stale id 自愈
                if r.invalidate_id {
                    if let Some(old_id) = terminal_id.as_deref() {
                        clear_stale_terminal_id(&store, &id, old_id);
                    }
                }
                if r.focused {
                    "open_session_source/focus_terminal"
                } else {
                    fallback_after_terminal_miss(source.as_deref(), workspace_path.as_deref());
                    "open_session_source/terminal_miss_fallback"
                }
            }
            FocusTarget::Ghostty { terminal_id } => {
                let r = focus_ghostty_for_session(
                    &store,
                    &id,
                    terminal_id,
                    terminal_id_captured_cwd.as_deref(),
                    workspace_path.as_deref(),
                    "open_session_source",
                );
                if r.focused {
                    "open_session_source/focus_ghostty"
                } else {
                    fallback_after_terminal_miss(source.as_deref(), workspace_path.as_deref());
                    "open_session_source/ghostty_miss_fallback"
                }
            }
            FocusTarget::CodexDesktop { thread_id } => {
                focus_codex_desktop(thread_id);
                "open_session_source/codex_desktop"
            }
            FocusTarget::CursorGui { path } => {
                focus_cursor_workspace(path);
                "open_session_source/focus_cursor"
            }
            FocusTarget::DshSession { session_id } => {
                match focus_dsh_session(session_id, dsh_nav_socket.as_deref()) {
                    DshFocusOutcome::Precise => "open_session_source/dsh_open",
                    DshFocusOutcome::AppActivatedOnly => "open_session_source/dsh_activate_only",
                    DshFocusOutcome::Failed => "open_session_source/dsh_failed",
                }
            }
            FocusTarget::Finder { path: _ } => {
                // 兜底动作已下线,见 fallback_after_terminal_miss 的说明。
                // popup/panel 上层调 auto_hide_panel 关掉窗口即可,不再打开 Finder。
                crate::log_debug!(
                    "focus",
                    "open_session_source: FocusTarget::Finder no-op (fallback disabled)"
                );
                "open_session_source/finder_noop"
            }
            FocusTarget::Nothing => "open_session_source/nothing",
        }
    };

    // 段 C(herdr-remote):段 B 之前先激活承载本地 herdr client 的终端窗口,
    // 否则用户看不到回跳动静(见 focus_herdr_remote_client_window 注释)。outcome
    // 分型带 session_id 落日志,与 focus_session_terminal 侧对齐。
    if crate::surface::is_herdr_remote_source(source.as_deref()) {
        let outcome = focus_herdr_remote_client_window(herdr_socket_path.as_deref());
        crate::log_info!(
            "focus",
            "open_session_source herdr-remote 段C session_id={} outcome={:?}",
            id,
            outcome
        );
    }

    // 段 B:段 A 完成后如果 session 绑了 herdr pane,让 herdr 内部切到目标 pane。
    // 不管段 A 落到 Terminal / Ghostty / 兜底,只要 binding 有值就试 —— 三件套就是
    // "CC 跑在 herdr pane"的判据。段 B 失败按 source 分型处理(见 focus_herdr_if_bound)。
    if let (Some(pane), Some(sock)) = (herdr_pane_id.as_deref(), herdr_socket_path.as_deref()) {
        let _ = focus_herdr_if_bound(
            &store,
            &id,
            pane,
            sock,
            herdr_workspace_id.as_deref(),
            crate::surface::is_herdr_remote_source(source.as_deref()),
        );
    }

    auto_hide_panel(&app, reason);
}

/// 命令副作用结束后主动隐藏 panel 窗口。
/// 仅在 panel 真正存在时才记日志，避免那些"用户根本没开 panel
/// 就触发 command"的路径（理论上不存在，但 cheap insurance）刷屏。
/// 走 tray::hide_session_panel：macOS 上是 hide()，其他平台是 destroy()。
///
/// 2026-07-01:focus 命令改 async + spawn_blocking 后,本函数可能在 tokio blocking
/// 线程被调用。`window.hide()` 是 AppKit 操作,必须回主线程,否则非主线程动 NSWindow
/// 是 UB。`run_on_main_thread` 在主线程时同步执行、非主线程时排队到下一 tick,两种
/// 调用来源都安全。
pub(crate) fn auto_hide_panel(app: &AppHandle, reason: &str) {
    if app.get_webview_window("panel").is_some() {
        crate::log_debug!("panel", "auto-hide panel after {}", reason);
        let app_clone = app.clone();
        let _ = app.run_on_main_thread(move || {
            crate::tray::hide_session_panel(&app_clone);
        });
    }
}

/// Focus the terminal matching the given tty (or, for Ghostty, by stable terminal id
/// captured at SessionStart;失败时退 cwd first-match)。
/// Only tries terminal apps that are already running to avoid launching unwanted apps.
/// 探测链:**iTerm2(tty) → Terminal.app(tty) → Ghostty(id/cwd)**;前两者命中直接返回。
/// 跟 `focus_ghostty` 一样,把"stored terminal_id 看起来已 stale"信号冒上来,
/// caller 拿到后调 `clear_terminal_id_if_matches` 清掉脏 id。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FocusTerminalResult {
    pub focused: bool,
    pub invalidate_id: bool,
}

fn focus_terminal(
    tty: &str,
    terminal_id: Option<&str>,
    terminal_id_captured_cwd: Option<&str>,
    workspace_path: Option<&str>,
) -> FocusTerminalResult {
    let mut result = FocusTerminalResult::default();
    let iterm_running = is_app_running("iTerm2");
    let term_running = is_app_running("Terminal");
    crate::log_debug!(
        "focus",
        "focus_terminal probing: iTerm2_running={} Terminal_running={}",
        iterm_running,
        term_running
    );
    if iterm_running {
        let ok = focus_iterm2(tty);
        crate::log_info!("focus", "focus_iterm2({}) -> {}", tty, ok);
        if ok {
            result.focused = true;
            return result;
        }
    }
    if term_running {
        let ok = focus_terminal_app(tty);
        crate::log_info!("focus", "focus_terminal_app({}) -> {}", tty, ok);
        if ok {
            result.focused = true;
            return result;
        }
    }
    // Ghostty(brief 20260520-ghostty-terminal-focus):前两个 enumerate 找不到这个 tty
    // 说明不是 iTerm2 / Terminal.app 的 session;尝试 Ghostty。
    //
    // 2026-10-29(brief 20261029-ghostty-tty-focus):Ghostty 1.4 的 sdef 原生暴露
    // `tty`,先用 tty 直连(与 iTerm2 同构):命中即精准落位,split pane / 同 cwd
    // 多 tab 都准。miss(1.3 无该属性 / tty 属于 herdr·tmux 内层 pty)则继续走
    // 下面的 terminal_id / cwd / activate 兜底 —— 任一分支失败继续下一个的语义不变。
    if is_app_running("Ghostty") {
        if focus_ghostty_by_tty(tty) {
            crate::log_info!("focus", "focus_ghostty_by_tty({}) -> true", tty);
            result.focused = true;
            return result;
        }
        crate::log_debug!("focus", "focus_ghostty_by_tty({}) miss, fall L1/L2/L3", tty);
    }
    // 兜底梯:id Some 走精准 L1,否则 cwd L2,都不行 L3 activate-only。
    // 任何一层 true 即视为已 focus。
    if terminal_id.is_some() || workspace_path.is_some() {
        crate::log_info!(
            "focus",
            "focus_terminal trying ghostty (tty={} miss iTerm2/Terminal); terminal_id_hint={} captured_cwd={} workspace_path={}",
            tty,
            terminal_id_hint(terminal_id),
            log_path_hint(terminal_id_captured_cwd),
            log_path_hint(workspace_path)
        );
        let g = focus_ghostty(terminal_id, terminal_id_captured_cwd, workspace_path);
        result.invalidate_id = g.invalidate_id;
        result.focused = g.focused;
    }
    result
}

#[cfg(test)]
mod tests;
