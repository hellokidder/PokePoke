use super::applescript::{is_app_running, run_applescript_bool};
use super::ghostty_herdr::{focus_ghostty_herdr_anchor, focus_ghostty_herdr_anchor_for_socket};
use super::iterm2::focus_iterm2;
use super::terminal_app::focus_terminal_app;
use crate::sessions::SessionStore;
use std::sync::{Arc, Mutex};

/// popup 用:client_tty 是否归属 WezTerm。
pub(crate) fn client_tty_belongs_to_wezterm(client_tty: &str) -> bool {
    tty_belongs_to_app(client_tty, "WezTerm")
}

/// popup 用:client_tty 是否归属 Ghostty(避免把 cmux herdr 误判成 Ghostty tab 命中)。
pub(crate) fn client_tty_belongs_to_ghostty(client_tty: &str) -> bool {
    tty_belongs_to_app(client_tty, "Ghostty")
}

/// 段 B:段 A(Ghostty tab 层聚焦)完成后,如果 session 绑了 herdr pane,
/// 通过 herdr socket 让 herdr 内部把焦点切到目标 pane。
///
/// 失败时(socket down / pane_id 过时)反向清 binding,让下次 click 走既有
/// Ghostty 路径 + 下轮 reconcile 时重建 —— 用户至少能到 Ghostty tab,而不是
/// 卡在一个死绑定上。
///
/// 返回:段 B 是否命中(仅供日志)。段 A 已经把用户带到了正确 tab,段 B 命中
/// 与否不再影响 UX 主线。
pub(super) fn focus_herdr_if_bound(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    herdr_pane_id: &str,
    herdr_socket_path: &str,
    herdr_workspace_id: Option<&str>,
    is_herdr_remote: bool,
) -> bool {
    let sock = std::path::PathBuf::from(herdr_socket_path);
    let mut outcome = crate::herdr::focus_agent_pane(&sock, herdr_pane_id, herdr_workspace_id);
    // 瞬态失败(传输层超时/断连)对 agent.focus 重试一次,不重发 workspace.focus
    // (workspace 已切好,重发只会再次触发远程慢渲染)。
    if outcome == crate::herdr::HerdrFocusOutcome::TransportFailure {
        crate::log_info!(
            "focus",
            "focus_herdr_if_bound transport failure, retry agent.focus once pane_id={}",
            herdr_pane_id,
        );
        outcome = crate::herdr::focus_pane(&sock, herdr_pane_id);
    }
    crate::log_info!(
        "focus",
        "focus_herdr_if_bound pane_id={} workspace_id={} sock={} -> {:?} remote={}",
        herdr_pane_id,
        herdr_workspace_id.unwrap_or("<none>"),
        herdr_socket_path,
        outcome,
        is_herdr_remote,
    );
    let ok = outcome == crate::herdr::HerdrFocusOutcome::Focused;
    if !ok {
        // 失败分型 + 分域(brief 20260807-herdr-remote-jumpback-fix):
        // - 本地 herdr:保留既有"失败即清"恢复机制(2026-07-14 root cause B ——
        //   清掉后下次点击走 Ghostty 锚定 + 下轮 reconcile 重建,不清会卡死
        //   stale pane「点击无反应」并阻塞 cmux binding 迁移)。
        // - herdr-remote:瞬态失败(超时/断连)保留 binding —— 一次超时不该
        //   永久失去回跳能力,断连→点击快速失败→重连→点击恢复;
        //   仅 herdr 显式错误(pane 确定不存在)才清,monitor 3 轮消失后
        //   session_end 会整体清理 session(含 binding),窗口有界。
        if !is_herdr_remote || outcome == crate::herdr::HerdrFocusOutcome::ExplicitError {
            let cleared = store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear_surface_binding_if_matches(
                    session_id,
                    &crate::surface::SurfaceBinding::Herdr {
                        pane_id: herdr_pane_id.to_string(),
                        workspace_id: None,
                        socket_path: None,
                    },
                );
            crate::log_info!(
                "focus",
                "cleared stale herdr_binding session_id={} pane_id={} cleared={} remote={}",
                session_id,
                herdr_pane_id,
                cleared,
                is_herdr_remote,
            );
        }
    }
    ok
}

/// 段 B 之前 herdr 段 A 的宿主定位 hint(来自 session binding)。
pub(super) struct HerdrHostAnchorHint<'a> {
    pub(super) wezterm_unix_socket: Option<&'a str>,
    pub(super) wezterm_executable: Option<&'a str>,
    /// herdr-in-Orca:client_tty 歧义或 host-by-tty miss 时按 pane_key 切 Orca tab。
    pub(super) orca_pane_key: Option<&'a str>,
    pub(super) orca_worktree_id: Option<&'a str>,
    /// agent cwd,用于 `orca terminal list` 消歧 herdr 外层 tab。
    pub(super) workspace_path: Option<&'a str>,
}

pub(crate) fn resolve_herdr_client_tty(
    path: &std::path::Path,
    wezterm_unix_socket: Option<&str>,
    wezterm_executable: Option<&str>,
    orca_pane_key: Option<&str>,
    orca_worktree_id: Option<&str>,
) -> Option<String> {
    let hint = HerdrHostAnchorHint {
        wezterm_unix_socket,
        wezterm_executable,
        orca_pane_key,
        orca_worktree_id,
        workspace_path: None,
    };
    resolve_herdr_client_tty_with_hint(path, &hint)
}

fn resolve_herdr_client_tty_with_hint(
    path: &std::path::Path,
    hint: &HerdrHostAnchorHint<'_>,
) -> Option<String> {
    let ttys = crate::herdr::client_ttys_for_socket(path);
    match ttys.len() {
        0 => None,
        1 => Some(ttys[0].clone()),
        _ => {
            if let Some(sock) = hint.wezterm_unix_socket.filter(|s| !s.is_empty()) {
                if let Some(tty) = ttys.iter().find(|t| {
                    crate::wezterm::pane_exists_for_tty(t, Some(sock), hint.wezterm_executable)
                }) {
                    crate::log_info!(
                        "focus",
                        "resolve_herdr_client_tty picked tty={} via wezterm socket={}",
                        tty,
                        sock
                    );
                    return Some(tty.clone());
                }
            }
            let wezterm_ttys: Vec<_> = ttys
                .iter()
                .filter(|t| tty_belongs_to_app(t, "WezTerm"))
                .collect();
            if wezterm_ttys.len() == 1 {
                crate::log_info!(
                    "focus",
                    "resolve_herdr_client_tty picked tty={} as sole WezTerm client",
                    wezterm_ttys[0]
                );
                return Some(wezterm_ttys[0].clone());
            }
            let ghostty_ttys: Vec<_> = ttys
                .iter()
                .filter(|t| tty_belongs_to_app(t, "Ghostty"))
                .collect();
            if ghostty_ttys.len() == 1 {
                crate::log_info!(
                    "focus",
                    "resolve_herdr_client_tty picked tty={} as sole Ghostty client",
                    ghostty_ttys[0]
                );
                return Some(ghostty_ttys[0].clone());
            }
            let orca_ttys: Vec<_> = ttys
                .iter()
                .filter(|t| tty_belongs_to_app(t, "Orca"))
                .collect();
            if orca_ttys.len() == 1 {
                crate::log_info!(
                    "focus",
                    "resolve_herdr_client_tty picked tty={} as sole Orca client",
                    orca_ttys[0]
                );
                return Some(orca_ttys[0].clone());
            }
            let otty_ttys: Vec<_> = ttys
                .iter()
                .filter(|t| tty_belongs_to_app(t, "Otty"))
                .collect();
            if otty_ttys.len() == 1 {
                crate::log_info!(
                    "focus",
                    "resolve_herdr_client_tty picked tty={} as sole Otty client",
                    otty_ttys[0]
                );
                return Some(otty_ttys[0].clone());
            }
            // Warp:herdr UI client 跑在 Warp 里(brief 20260817-herdr-warp-jumpback)。
            // 放 ladder 末位 -- 只有 WezTerm/Ghostty/Orca/Otty 都不在场时才由 Warp
            // 胜出,与既有"唯一 X client"语义一致;副产品是 cmux+Warp 双 client
            // 歧义场景会解到 Warp(cmux 本就不在 ladder,走后续 host anchor)。
            let warp_ttys: Vec<_> = ttys
                .iter()
                .filter(|t| tty_belongs_to_app(t, "Warp"))
                .collect();
            if warp_ttys.len() == 1 {
                crate::log_info!(
                    "focus",
                    "resolve_herdr_client_tty picked tty={} as sole Warp client",
                    warp_ttys[0]
                );
                return Some(warp_ttys[0].clone());
            }
            crate::log_info!(
                "focus",
                "resolve_herdr_client_tty skip: {} ui client ttys for sock={} (ambiguous)",
                ttys.len(),
                path.display()
            );
            None
        }
    }
}

/// 段 C 结果分型(供日志/排查;caller 不依赖它改变跳转行为):
///
/// - [`Precise`]:精确命中 —— Warp title-key `tab.activate`,或非 Warp 宿主的精确 host select。
/// - [`AppActivatedOnly`]:仅 app 级激活(activate-only 兜底)—— Warp local_control
///   未开 / marker 失败时只能把 Warp 拉到前台,显示的是上一个活跃 tab,双 herdr
///   tab 并存时可能停错(2026-08-18 用户实测)。
/// - [`Failed`]:什么都没做成(解析失败 / client tty 找不到 / 激活失败)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HerdrRemoteClientWindowOutcome {
    Precise,
    AppActivatedOnly,
    Failed,
}

/// 段 C(herdr-remote):激活承载 `herdr --remote <target>` UI client 的本地终端窗口。
///
/// 段 B(`focus_herdr_if_bound`)只把远程 herdr server 的 focus 切到目标 pane,用户
/// 眼睛在公司机屏幕 —— 本地终端窗口不激活,点击 popup 后“完全没动静”(2026-08-07
/// 用户实测)。段 C 用 `herdr_remote_client_tty` 找到本地 herdr client 所在 tty,再
/// 走既有 `focus_herdr_client_host_by_tty`(WezTerm / Orca / iTerm2 / Terminal /
/// Otty / cmux 全套宿主激活)把承载它的终端窗口拉到前台。
///
/// 仅 herdr-remote 调用(本地 herdr 由段 A 的 host anchor 处理)。失败只记日志,
/// 不影响段 B。
///
/// 返回 [`HerdrRemoteClientWindowOutcome`] 让"精确命中"与"仅激活 app"在日志/返回
/// 值上诚实可辨 —— 2026-08-19 排查双 herdr tab 时发现原 bool 返回把 marker skip
/// 后的 activate-only 也报 `true`,误报精确命中。
pub(crate) fn focus_herdr_remote_client_window(
    herdr_socket_path: Option<&str>,
) -> HerdrRemoteClientWindowOutcome {
    let Some(sock) = herdr_socket_path.filter(|s| !s.is_empty()) else {
        return HerdrRemoteClientWindowOutcome::Failed;
    };
    let Some(target) = herdr_remote_target_from_socket(sock) else {
        crate::log_info!(
            "focus",
            "focus_herdr_remote_client_window sock={} target parse failed",
            sock
        );
        return HerdrRemoteClientWindowOutcome::Failed;
    };
    let Some(tty) = crate::herdr::herdr_remote_client_tty(&target) else {
        crate::log_info!(
            "focus",
            "focus_herdr_remote_client_window target={} client tty not found",
            target
        );
        return HerdrRemoteClientWindowOutcome::Failed;
    };
    // Warp 宿主:marker 标题 + tab.activate 精确切 tab(brief
    // 20260818-herdr-remote-warp-tab-focus)。activate-only 只把 Warp 拉前台,
    // 显示的是上一个活跃 tab —— 本机 + remote 双 herdr tab 时点 remote popup
    // 会停在本地 herdr tab(2026-08-18 用户实测)。
    if tty_belongs_to_app(&tty, "Warp") {
        if focus_warp_tab_with_marker(sock, &target) {
            crate::log_info!(
                "focus",
                "focus_herdr_remote_client_window target={} tty={} -> warp tab focused (title-key, precise)",
                target,
                tty
            );
            return HerdrRemoteClientWindowOutcome::Precise;
        }
        // marker 不可用(Warp Settings > Scripting 未开)/ 失败 → 显式 activate-only
        // 兜底,返回 AppActivatedOnly —— 不再让 "只激活了 app" 误报成精确命中。
        let ok = super::warp::focus_warp_session();
        crate::log_info!(
            "focus",
            "focus_herdr_remote_client_window target={} tty={} warp marker skipped -> app-activate-only ok={} (tab may be wrong)",
            target,
            tty,
            ok
        );
        return if ok {
            HerdrRemoteClientWindowOutcome::AppActivatedOnly
        } else {
            HerdrRemoteClientWindowOutcome::Failed
        };
    }
    let hint = HerdrHostAnchorHint {
        wezterm_unix_socket: None,
        wezterm_executable: None,
        orca_pane_key: None,
        orca_worktree_id: None,
        workspace_path: None,
    };
    let ok = focus_herdr_client_host_by_tty(&tty, &hint);
    crate::log_info!(
        "focus",
        "focus_herdr_remote_client_window target={} tty={} -> host-by-tty {}",
        target,
        tty,
        ok
    );
    if ok {
        HerdrRemoteClientWindowOutcome::Precise
    } else {
        HerdrRemoteClientWindowOutcome::Failed
    }
}

/// Warp tab 精确聚焦:先经 herdr server socket 把 remote client 所在 tab 标题
/// 改成唯一 marker(`herdr · <target>`),等 OSC 2 传播到 Warp UI 后按标题全等
/// `tab.activate`。local_control 不可用(Warp Settings > Scripting 未开)时
/// 不设 marker 直接返回 false —— 不白改用户 tab 标题,由调用方退 activate-only。
fn focus_warp_tab_with_marker(sock: &str, target: &str) -> bool {
    if !crate::warp::is_available() {
        crate::log_info!(
            "focus",
            "focus_warp_tab_with_marker skip: warp local-control unavailable (Scripting off)"
        );
        return false;
    }
    let marker = format!("herdr · {target}");
    if !crate::herdr::set_client_window_title(std::path::Path::new(sock), &marker) {
        crate::log_info!(
            "focus",
            "focus_warp_tab_with_marker marker set failed sock={}",
            sock
        );
        return false;
    }
    // OSC 2 经 remote bridge → client → Warp UI 传播需要时间,等 300ms 再激活。
    std::thread::sleep(std::time::Duration::from_millis(300));
    // tab.activate 自带窗口聚焦,app.focus 失败也不影响 tab 切换。
    let _ = crate::warp::focus_app();
    match crate::warp::focus_tab_by_title(&marker) {
        Ok(()) => {
            crate::log_info!(
                "focus",
                "focus_warp_tab_with_marker tab.activate ok marker={:?}",
                marker
            );
            true
        }
        Err(e) => {
            crate::log_info!(
                "focus",
                "focus_warp_tab_with_marker tab.activate failed marker={:?}: {}",
                marker,
                e
            );
            false
        }
    }
}

/// 从 PokePoke 转发 socket 路径解析远程 target:
/// `~/.pokepoke/herdr-remote-<target>.sock` → `<target>`。
fn herdr_remote_target_from_socket(sock: &str) -> Option<String> {
    let name = std::path::Path::new(sock).file_name()?.to_str()?;
    let stripped = name.strip_prefix("herdr-remote-")?.strip_suffix(".sock")?;
    (!stripped.is_empty()).then(|| stripped.to_string())
}

/// 段 A：优先用 herdr client tty 精确聚焦外层宿主（iTerm2 / Terminal / Ghostty
/// herdr tab / Otty / Orca），失败后依次尝试：
/// 1. Orca CLI 发现 herdr 外层 tab（client_tty 解歧失败时）
/// 2. Ghostty title 前缀匹配 herdr 自动写的 `<ws-label> · cc-<hex8>`
/// 3. 仍无命中：cmux title=="herdr" 全局唯一（legacy，仅无 socket 时用）
///
/// 有 socket 且已解析出 client_tty 时**禁止**掉到步骤 3 —— 否则 iTerm2 上的
/// `herdr --session work` 会被机器里另一个 cmux herdr pane 误吸走。
pub(super) fn focus_herdr_host_anchor(
    herdr_socket_path: Option<&str>,
    hint: &HerdrHostAnchorHint<'_>,
) -> bool {
    if let Some(sock) = herdr_socket_path.filter(|s| !s.is_empty()) {
        let path = std::path::Path::new(sock);
        if let Some(client_tty) = resolve_herdr_client_tty_with_hint(path, hint) {
            if focus_herdr_client_host_by_tty(client_tty.as_str(), hint) {
                return true;
            }
            // client_tty 已 session-scoped 解出;只有归属 Ghostty 时才走 Ghostty tab
            // 聚焦,否则会把 cmux/iTerm2/WezTerm 上的 herdr 误吸到 Ghostty herdr tab。
            if tty_belongs_to_app(&client_tty, "Ghostty")
                && focus_ghostty_herdr_anchor_for_socket(path)
            {
                return true;
            }
            // 2026-07-14(根因 B,herdr-in-iTerm2 回跳致死):host-by-tty 精确 select
            // 全 miss 后,**不能**直接 return false -- 那会让 caller 落 target
            // ladder,而 ladder 用的是 session.terminal_tty(存的是 CC inner pane
            // 的 pty,如 /dev/ttys015),根本不是 herdr client 所在 iTerm2 session
            // 的 tty(ttys023)-> ladder 永远 miss -> 用户"点了没反应"。
            //
            // 兜底:已解出 client_tty 说明 herdr client 真在某个终端 session 上跑,
            // 精确 select 失败(瞬时竞态 / Apple Event 撞车)时,至少 activate 把
            // 候选宿主 app 拉到前台。iTerm2/Terminal 在跑就 activate 它,cmux 在跑
            // 就 activate cmux app。不选 session,只拉前台 -- herdr 窗口就在那个
            // app 里,段 B(workspace.focus + agent.focus)仍会把 pane 切对。
            // 这比"落 ladder 用错 tty 100% miss"强得多。
            if activate_terminal_host_fallback(&client_tty, hint.workspace_path) {
                crate::log_info!(
                    "focus",
                    "focus_herdr_host_anchor: client_tty={} host-by-tty miss, activated host app as fallback",
                    client_tty
                );
                return true;
            }
            crate::log_info!(
                "focus",
                "focus_herdr_host_anchor: client_tty={} all hosts miss + no app to activate; skip global cmux title fallback (sock={})",
                client_tty,
                sock
            );
            return false;
        }
        // client_tty 解歧失败:尝试 Orca CLI 发现 herdr 外层 tab,再试显式 pane_key。
        if super::orca::focus_orca_herdr_anchor_by_tty("", hint.workspace_path) {
            return true;
        }
        if let Some(pane_key) = hint.orca_pane_key.filter(|s| !s.is_empty()) {
            if super::orca::focus_orca_pane(pane_key, hint.orca_worktree_id) {
                crate::log_info!(
                    "focus",
                    "focus_herdr_host_anchor: orca pane_key={} (client_tty ambiguous/miss)",
                    pane_key
                );
                return true;
            }
        }
        if focus_ghostty_herdr_anchor_for_socket(path) {
            return true;
        }
    }
    if focus_ghostty_herdr_anchor() {
        return true;
    }
    crate::cmux::focus_herdr_client_anchor()
}

/// herdr host anchor 的最终兜底:精确 select 失败时,把 herdr UI client 所在的
/// 终端宿主 app 拉前台(不选 session,只 activate)。
///
/// 2026-07-14(ghostty+herdr 回跳失败):首版只 activate iTerm2/Terminal/cmux,
/// **漏了 Ghostty** -> ghostty+herdr 场景误激活 cmux(cmux 也在跑)。改为**反查
/// client_tty 属于哪个终端 app**,只 activate 那一个,避免误激活无关的终端 app
/// 盖住 herdr 窗口。
///
/// 反查用 AppleScript 逐个候选 app 问"你的 session tty 里有没有这个 tty":
/// iTerm2 / Terminal 支持 `tty of session`;Ghostty 不直接暴露 tty,但 herdr
/// 在 Ghostty 时该 tab name 以 "herdr" 开头(focus_ghostty_herdr_anchor 已处理精确
/// tab 聚焦),所以 Ghostty 兜底只做 activate(不查 tty)。cmux 走 cmux tree 的
/// find_surfaces_by_tty(已在 host_by_tty 里试过,这里不重复)。
///
/// 返回 true 表示激活了某个在跑的终端 app。
fn activate_terminal_host_fallback(client_tty: &str, workspace_path: Option<&str>) -> bool {
    let tty_dev = normalize_client_tty_for_host(client_tty);
    // iTerm2 / Terminal:查 session tty 是否含目标 tty,命中才 activate(避免误激活
    // 不相关的在跑终端 app)。
    if is_app_running("iTerm2") && host_app_has_tty("iTerm2", &tty_dev) {
        let ok = run_applescript_bool(
            r#"tell application "iTerm2" to activate
return true"#,
        );
        crate::log_info!(
            "focus",
            "activate_host_fallback iterm2 tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    if is_app_running("Terminal") && host_app_has_tty("Terminal", &tty_dev) {
        let ok = run_applescript_bool(
            r#"tell application "Terminal" to activate
return true"#,
        );
        crate::log_info!(
            "focus",
            "activate_host_fallback terminal_app tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // Ghostty:不查 tty(Ghostty AppleScript 不直接暴露 per-terminal tty),但走到
    // 兜底说明 focus_ghostty_herdr_anchor 精确 tab 聚焦已失败。herdr UI client 的
    // tty 属于 Ghostty 时,至少 activate Ghostty 把它拉前台(用户能看到 herdr tab)。
    if is_app_running("Ghostty") && tty_belongs_to_app(client_tty, "Ghostty") {
        let ok = run_applescript_bool(
            r#"tell application "Ghostty" to activate
return true"#,
        );
        crate::log_info!(
            "focus",
            "activate_host_fallback ghostty tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // WezTerm:按 client_tty 定位 pane 并 activate(herdr-in-WezTerm 主路径兜底)。
    if tty_belongs_to_app(client_tty, "WezTerm") {
        let ok = crate::wezterm::focus_tty(client_tty, None, None);
        crate::log_info!(
            "focus",
            "activate_host_fallback wezterm tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // Orca:herdr UI client 在 Orca tab 里时,按 tty 选中 tab 并 activate。
    if is_app_running("Orca") && tty_belongs_to_app(client_tty, "Orca") {
        let ok = super::orca::focus_orca_herdr_anchor_by_tty(&tty_dev, workspace_path);
        crate::log_info!(
            "focus",
            "activate_host_fallback orca tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // Warp:host-by-tty 的 Warp activate 自查失败(瞬时竞态 / Apple Event 撞车)
    // 时重试一次,与其它终端的兜底对齐(brief 20260817-herdr-warp-jumpback)。
    if tty_belongs_to_app(client_tty, "Warp") {
        let ok = super::warp::focus_warp_session();
        crate::log_info!(
            "focus",
            "activate_host_fallback warp tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // cmux:client_tty 归属 cmux 时 activate cmux app(brief
    // 20260818-cmux-herdr-jumpback-fallback)。精确 focus-panel 依赖 cmux tree;
    // tree 挂死/失败时(2026-08-18 实测 cmux CLI 无限 hang)至少把 herdr 所在
    // 的 cmux 窗口拉前台,段 B 仍会把 pane 切对。门控用 tty 归属反查(ps ppid
    // 链命中 /Applications/cmux.app/.../cmux),不查 tree,与 Ghostty/WezTerm/
    // Warp 分支语义一致。
    if tty_belongs_to_app(client_tty, "cmux") {
        let ok = crate::cmux::activate_cmux_app();
        crate::log_info!(
            "focus",
            "activate_host_fallback cmux tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    false
}

/// 查 iTerm2 / Terminal 是否有 session 的 tty 等于 `tty_dev`。纯查询,不 activate。
/// 用于 `activate_terminal_host_fallback` 判定 client_tty 是否属于该 app,避免
/// 误激活无关的在跑终端 app。
fn host_app_has_tty(app: &str, tty_dev: &str) -> bool {
    let script = format!(
        r#"tell application "{app}"
    try
        repeat with w in windows
            repeat with t in tabs of w
                repeat with s in sessions of t
                    if tty of s is "{tty}" then return true
                end repeat
            end repeat
        end repeat
        return false
    on error
        return false
    end try
end tell"#,
        app = app,
        tty = tty_dev
    );
    run_applescript_bool(&script)
}

/// 反查 `tty` 所属进程树顶层是否为 `app`。用 ps 拿 tty 上的进程,沿 ppid 往上找,
/// 看顶层进程的 comm/路径是否匹配 app 名。用于 Ghostty(Ghostty AppleScript 不暴露
/// per-terminal tty,但 herdr 进程跑在 Ghostty 的 tab 里时,ps 进程树顶层是 Ghostty)。
fn tty_belongs_to_app(tty: &str, app: &str) -> bool {
    use std::process::Command;
    // ps 拿该 tty 上所有进程的 pid + ppid + comm
    let bare = tty.trim().strip_prefix("/dev/").unwrap_or(tty.trim());
    let out = match Command::new("ps")
        .args(["-t", bare, "-o", "pid=,ppid=,comm="])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    // 收集该 tty 上所有 pid -> comm,以及 ppid 链。
    // 简化:只要有任一进程的 comm 含 app 名(iTerm2/Ghostty/Terminal 的子进程
    // 如 login/zsh 通常不含,但 herdr 本身不含 -- 需要往上爬 ppid)。
    // 这里直接查:该 tty 的进程链顶层 bundle 是否目标 app。
    let lines = String::from_utf8_lossy(&out.stdout);
    // 收集 pid->ppid, comm
    let mut procs: Vec<(i32, i32, String)> = Vec::new();
    for line in lines.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid_s), Some(ppid_s)) = (it.next(), it.next()) else {
            continue;
        };
        let comm = it.collect::<Vec<_>>().join(" ");
        if let (Ok(pid), Ok(ppid)) = (pid_s.parse::<i32>(), ppid_s.parse::<i32>()) {
            procs.push((pid, ppid, comm));
        }
    }
    if procs.is_empty() {
        return false;
    }
    // 对每个进程沿 ppid 往上爬,看顶层 comm 是否含 app 名。
    // ppid 不在当前 tty 进程集里时,用 ps 查全局。
    for (pid, _ppid, _) in &procs {
        if pid_chain_contains_app(*pid, app) {
            return true;
        }
    }
    false
}

/// 沿 pid 的 ppid 链往上爬(最多 20 层防环),看顶层进程 comm 是否含 `app` 名。
fn pid_chain_contains_app(start_pid: i32, app: &str) -> bool {
    use std::process::Command;
    let mut pid = start_pid;
    for _ in 0..20 {
        let out = match Command::new("ps")
            .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
            .output()
        {
            Ok(o) => o,
            Err(_) => return false,
        };
        if !out.status.success() {
            return false;
        }
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let mut it = line.split_whitespace();
        let ppid_s = it.next();
        let comm = it.collect::<Vec<_>>().join(" ");
        // comm 含 app 名(如 "Ghostty", "iTerm2")?Ghostty 进程 comm 通常是
        // "Ghostty" 或路径含 Ghostty。
        if comm.to_lowercase().contains(&app.to_lowercase()) {
            return true;
        }
        match ppid_s.and_then(|s| s.parse::<i32>().ok()) {
            Some(p) if p > 0 && p != pid => pid = p,
            _ => return false,
        }
    }
    false
}

fn normalize_client_tty_for_host(client_tty: &str) -> String {
    let bare = client_tty.trim();
    if bare.starts_with("/dev/") {
        bare.to_string()
    } else {
        format!("/dev/{bare}")
    }
}

/// 把 herdr UI client 的 tty 聚焦到正确的外层宿主(iTerm2 / Terminal / cmux / WezTerm)。
fn focus_herdr_client_host_by_tty(client_tty: &str, hint: &HerdrHostAnchorHint<'_>) -> bool {
    let tty_dev = normalize_client_tty_for_host(client_tty);
    // WezTerm:仅当 client tty 确属 WezTerm pane 才扫 socket 列表 + `wezterm cli list`。
    // 否则每个 iTerm2 / Ghostty 上的 herdr 跳转都会 discover_wezterm_gui_sockets 遍历
    // 全部实例,纯开销。门控对齐 activate_host_fallback / popup 侧 frontmost gate。
    if tty_belongs_to_app(client_tty, "WezTerm")
        && crate::wezterm::focus_tty(&tty_dev, hint.wezterm_unix_socket, hint.wezterm_executable)
    {
        crate::log_info!("focus", "focus_herdr_host wezterm tty={} -> true", tty_dev);
        return true;
    }
    // Orca:herdr 跑在 Orca worktree pane 里时,`terminal list` 找 herdr tab 后 switch。
    if is_app_running("Orca") {
        let ok = super::orca::focus_orca_herdr_anchor_by_tty(&tty_dev, hint.workspace_path);
        crate::log_info!("focus", "focus_herdr_host orca tty={} -> {}", tty_dev, ok);
        if ok {
            return true;
        }
    }
    if let Some(pane_key) = hint.orca_pane_key.filter(|s| !s.is_empty()) {
        let ok = super::orca::focus_orca_pane(pane_key, hint.orca_worktree_id);
        crate::log_info!(
            "focus",
            "focus_herdr_host orca pane_key={} -> {}",
            pane_key,
            ok
        );
        if ok {
            return true;
        }
    }
    if is_app_running("iTerm2") {
        let ok = focus_iterm2(&tty_dev);
        crate::log_info!("focus", "focus_herdr_host iterm2 tty={} -> {}", tty_dev, ok);
        if ok {
            return true;
        }
    }
    if is_app_running("Terminal") {
        let ok = focus_terminal_app(&tty_dev);
        crate::log_info!(
            "focus",
            "focus_herdr_host terminal_app tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // Ghostty:herdr --remote UI client 跑在 Ghostty tab 里时,按 client tty 定位
    // tab(段 C,brief 20260810-herdr-remote-ghostty-jumpback)。tty 归属门控与上方
    // WezTerm / Otty 对齐;Ghostty AppleScript 无 tty 属性,内部走
    // cwd/title/单 tab 三级定位 + activate-only 兜底。
    if is_app_running("Ghostty") && tty_belongs_to_app(client_tty, "Ghostty") {
        let ok = super::ghostty_herdr::focus_ghostty_herdr_client_by_tty(&tty_dev);
        crate::log_info!(
            "focus",
            "focus_herdr_host ghostty tty={} -> {}",
            tty_dev,
            ok
        );
        if ok {
            return true;
        }
    }
    // Otty:herdr 跑在 Otty pane 里时,按 client tty 定位 Otty tab 并选中+抬窗。
    // 放在 cmux 之前,避免 herdr-in-Otty 时白跑一遍 cmux tree load(实测 cmux
    // 未运行也耗时)。tty 全局唯一,herdr 在别的终端时 `tty of tab` 不命中、不激活。
    //
    // 2026-08-03 事故:仅 gate `is_app_running("Otty")` 会让 herdr-in-Ghostty
    // (client tty 属 Ghostty)也白白 probe Otty;若 Otty AppleEvent 挂死,裸 osascript
    // 会等满 ~2 分钟拖死整条回跳链。补 `tty_belongs_to_app(client_tty, "Otty")` 门控,
    // 与上方 WezTerm / Orca 分支对齐——只有 herdr client 真在 Otty 里才碰 Otty。
    if is_app_running("Otty") && tty_belongs_to_app(client_tty, "Otty") {
        let ok = super::otty::focus_otty_herdr_anchor_by_tty(&tty_dev);
        crate::log_info!("focus", "focus_herdr_host otty tty={} -> {}", tty_dev, ok);
        if ok {
            return true;
        }
    }
    // Warp:herdr UI client 跑在 Warp 里时激活 Warp(activate-only)。Warp 无法
    // 按 tty 精确选 tab,与 popup 侧 app 级处理对齐;`focus_warp_session`
    // 内部三层降级(local_control -> open -a -> AppleScript)+ frontmost 自查。
    // 放 cmux anchor 之前,与其它具名终端分支顺序一致。
    if tty_belongs_to_app(client_tty, "Warp") {
        let ok = super::warp::focus_warp_session();
        crate::log_info!("focus", "focus_herdr_host warp tty={} -> {}", tty_dev, ok);
        if ok {
            return true;
        }
    }
    // cmux anchor 加 tty 归属门控(brief 20260818-cmux-herdr-jumpback-fallback):
    // 非 cmux tty 时跳过 cmux tree load —— tree 挂死会白付 3s 超时 + 外部 SIGKILL;
    // 行为等价(非 cmux tty 本来就 matched 0 surfaces)。
    if tty_belongs_to_app(client_tty, "cmux")
        && crate::cmux::focus_herdr_client_anchor_for_tty(client_tty)
    {
        return true;
    }
    false
}

#[cfg(test)]
mod herdr_host_anchor_tests {
    use super::normalize_client_tty_for_host;

    #[test]
    fn herdr_remote_target_from_socket_parses_basename() {
        assert_eq!(
            super::herdr_remote_target_from_socket("/Users/edy/.pokepoke/herdr-remote-hello.sock")
                .as_deref(),
            Some("hello")
        );
        assert_eq!(
            super::herdr_remote_target_from_socket(
                "/Users/edy/.pokepoke/herdr-remote-my-host-42.sock"
            )
            .as_deref(),
            Some("my-host-42")
        );
    }

    #[test]
    fn herdr_remote_target_from_socket_rejects_foreign_paths() {
        assert_eq!(
            super::herdr_remote_target_from_socket("/Users/edy/.config/herdr/herdr.sock"),
            None
        );
        assert_eq!(
            super::herdr_remote_target_from_socket("herdr-remote-.sock"),
            None
        );
    }

    #[test]
    fn normalize_client_tty_adds_dev_prefix() {
        assert_eq!(normalize_client_tty_for_host("ttys024"), "/dev/ttys024");
        assert_eq!(
            normalize_client_tty_for_host("/dev/ttys024"),
            "/dev/ttys024"
        );
    }

    #[test]
    fn focus_herdr_remote_client_window_none_socket_failed() {
        assert_eq!(
            super::focus_herdr_remote_client_window(None),
            super::HerdrRemoteClientWindowOutcome::Failed
        );
    }

    #[test]
    fn focus_herdr_remote_client_window_foreign_socket_failed() {
        // 本地 herdr socket / 任意非 `herdr-remote-<target>.sock` 路径 → target
        // 解析失败,纯 Failed,不依赖真实 socket。
        assert_eq!(
            super::focus_herdr_remote_client_window(Some("/Users/edy/.config/herdr/herdr.sock")),
            super::HerdrRemoteClientWindowOutcome::Failed
        );
    }
}

#[cfg(test)]
mod herdr_bind_tests {
    #[test]
    fn focus_herdr_if_bound_returns_false_and_no_op_when_socket_absent() {
        use crate::sessions::SessionStore;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(SessionStore::load(
            dir.path().join("s.json"),
        )));
        let ok = super::focus_herdr_if_bound(
            &store,
            "sess-x",
            "w3:p1",
            "/tmp/definitely-not-a-socket-9999",
            Some("w3"),
            false, // 本地 herdr:失败即清
        );
        assert!(!ok);
    }

    #[test]
    fn focus_herdr_if_bound_remote_transport_failure_keeps_binding() {
        // herdr-remote + 瞬态失败(无 socket → TransportFailure):保留 binding ——
        // 一次超时不该永久失去回跳能力,断连→点击失败→重连→点击恢复。
        use crate::sessions::{Priority, SessionStatus, SessionStore, UpsertRequest};
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(SessionStore::load(
            dir.path().join("s.json"),
        )));
        store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .upsert_session(UpsertRequest {
                task_id: "hr-hello-w1:p1".into(),
                title: "t".into(),
                message: "m".into(),
                source: Some("herdr-remote".into()),
                priority: Priority::Normal,
                status: SessionStatus::Idle,
                ..Default::default()
            });
        let session_id = store.lock().unwrap_or_else(|p| p.into_inner()).get_all()[0]
            .id
            .clone();
        store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_surface_binding(
                "hr-hello-w1:p1",
                crate::surface::SurfaceBinding::Herdr {
                    pane_id: "w1:p1".into(),
                    workspace_id: Some("w1".into()),
                    socket_path: Some("/tmp/x.sock".into()),
                },
            );
        let ok = super::focus_herdr_if_bound(
            &store,
            &session_id,
            "w1:p1",
            "/tmp/definitely-not-a-socket-9999",
            Some("w1"),
            true, // herdr-remote:瞬态失败保留 binding
        );
        assert!(!ok);
        let guard = store.lock().unwrap_or_else(|p| p.into_inner());
        let s = &guard.get_all()[0];
        assert_eq!(
            s.herdr_pane_id.as_deref(),
            Some("w1:p1"),
            "remote 瞬态失败不清 binding"
        );
    }

    #[test]
    fn focus_herdr_if_bound_local_failure_clears_binding() {
        // 本地 herdr:保留既有"失败即清"恢复机制(2026-07-14 root cause B)。
        use crate::sessions::{Priority, SessionStatus, SessionStore, UpsertRequest};
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(SessionStore::load(
            dir.path().join("s.json"),
        )));
        store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .upsert_session(UpsertRequest {
                task_id: "cc-abc".into(),
                title: "t".into(),
                message: "m".into(),
                source: Some("claude-code".into()),
                priority: Priority::Normal,
                status: SessionStatus::Running,
                ..Default::default()
            });
        let session_id = store.lock().unwrap_or_else(|p| p.into_inner()).get_all()[0]
            .id
            .clone();
        store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_surface_binding(
                "cc-abc",
                crate::surface::SurfaceBinding::Herdr {
                    pane_id: "w3:p1".into(),
                    workspace_id: Some("w3".into()),
                    socket_path: Some("/tmp/x.sock".into()),
                },
            );
        let ok = super::focus_herdr_if_bound(
            &store,
            &session_id,
            "w3:p1",
            "/tmp/definitely-not-a-socket-9999",
            Some("w3"),
            false, // 本地 herdr:失败即清
        );
        assert!(!ok);
        let guard = store.lock().unwrap_or_else(|p| p.into_inner());
        let s = &guard.get_all()[0];
        assert_eq!(s.herdr_pane_id, None, "本地 herdr 失败仍清 binding");
    }
}
