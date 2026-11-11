//! 终端表面抽象:SurfaceKind / SurfaceEvidence / 内层绑定枚举 SurfaceBinding(W5)。
//!
//! - [`SurfaceKind`] + [`ResolvedSurface`]:读侧(弹窗抑制 / 回跳分流)的终端身份分类;
//! - [`SurfaceEvidence`]:从 Session 平铺字段转换的只读视图;
//! - [`SurfaceBinding`]:存储层内层绑定枚举(W5 起写侧唯一真源,镜像平铺双写)。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceEvidence {
    pub source: Option<String>,
    /// 终端宿主归一化名(如 `warpterminal`;见 pokehook `terminal_app()`)。
    /// 用于无 AppleScript 枚举的宿主(Warp)的 app 级聚焦判定。
    pub terminal_app: Option<String>,
    pub task_id: String,
    pub session_id: String,
    // 各 source 的外部会话 id(codex hook payload 的 session_id,即 ChatGPT 桌面端
    // thread UUID)。codex 桌面端会话无 tty / terminal_id,靠它走 `codex://threads/<id>`
    // deep link 精准跳回;终端型 source 有 tty 优先,不会落到这条分支。
    pub external_session_id: Option<String>,
    pub workspace_path: Option<String>,
    /// DSH 本地控制通道(host 半插件自建)。聚焦判定用它现查渲染进程状态。
    pub dsh_nav_socket: Option<String>,
    pub terminal_tty: Option<String>,
    pub terminal_id: Option<String>,
    pub terminal_id_captured_cwd: Option<String>,
    pub herdr_pane_id: Option<String>,
    pub herdr_socket_path: Option<String>,
    pub cmux_surface_id: Option<String>,
    pub wezterm_pane_id: Option<String>,
    pub wezterm_unix_socket: Option<String>,
    pub wezterm_executable: Option<String>,
    pub otty_pane_id: Option<String>,
    pub otty_tab_id: Option<String>,
    pub otty_window_id: Option<String>,
    pub tmux_pane_id: Option<String>,
    pub tmux_socket_path: Option<String>,
    pub orca_pane_key: Option<String>,
    pub orca_worktree_id: Option<String>,
    pub orca_tab_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    TerminalTty,
    GhosttyTerminal,
    /// ChatGPT 桌面端会话(codex 无 tty,有 thread UUID):`codex://threads/<id>` deep link。
    CodexDesktop,
    /// DSH 会话(无 tty、无 deep link):由插件 socket 请求渲染进程 `openSession`。
    DshSession,
    CursorWorkspace,
    FinderWorkspace,
    /// Otty 直接 binding(`otty_pane_id`),无 tty 时仍能走聚焦抑制。
    OttyTerminal,
    /// Orca 直接 binding(`orca_pane_key`),无 tty 时仍能走聚焦抑制。
    OrcaTerminal,
    /// herdr binding(`herdr_pane_id + herdr_socket_path` 双在场,brief
    /// 20260810-surface-herdr-branch)。本地 herdr 可判聚焦;远程(herdr-remote)
    /// 聚焦不可判,capabilities 里 can_detect_focused=false。
    HerdrPane,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceConfidence {
    Exact,
    Probable,
    Fallback,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceCapabilities {
    pub can_focus: bool,
    pub can_detect_focused: bool,
    pub can_locate_screen: bool,
    pub can_check_liveness: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedSurface {
    pub kind: SurfaceKind,
    pub confidence: SurfaceConfidence,
    pub capabilities: SurfaceCapabilities,
    pub display_name: &'static str,
    pub reason: &'static str,
}

impl SurfaceEvidence {
    /// 从 Session 构造只读视图(W6,brief 20260813-surface-readers)。
    ///
    /// **内层绑定字段改从 `session.surface` 读**(W5 起写侧真源),宿主层字段
    /// (terminal_tty / terminal_id / terminal_id_captured_cwd / terminal_app /
    /// workspace_path / external_session_id)仍从平铺读。
    ///
    /// **herdr+orca 共存特例**(对称双向):herdr 跑在 Orca 外层时同 session 可
    /// 同时持有 herdr + orca(`orca_herdr.rs::apply_orca_binding_for_herdr_host_if_missing`),
    /// surface 单值只承载 primary —— surface=Herdr 时 orca 字段从 orca 镜像补,
    /// surface=Orca 时 herdr 字段从 herdr 镜像补(两个写序都可达:notify 先
    /// herdr 后 orca host hint → surface=Orca;ghostty_reconcile herdr 刷新后 →
    /// surface=Herdr)。
    pub fn from_session(session: &crate::sessions::Session) -> Self {
        use crate::surface::SurfaceBinding as B;
        let (herdr_pane_id, herdr_socket_path) = match &session.surface {
            B::Herdr {
                pane_id,
                socket_path,
                ..
            } => (Some(pane_id.clone()), socket_path.clone()),
            // surface=Orca(herdr 先写后 orca host hint):herdr 在镜像。
            B::Orca { .. } => (
                session.herdr_pane_id.clone(),
                session.herdr_socket_path.clone(),
            ),
            _ => (None, None),
        };
        let (orca_pane_key, orca_worktree_id, orca_tab_id) = match &session.surface {
            B::Orca {
                pane_key,
                worktree_id,
                tab_id,
            } => (Some(pane_key.clone()), worktree_id.clone(), tab_id.clone()),
            // surface=Herdr(herdr 刷新后):orca host hint 在镜像。
            B::Herdr { .. } => (
                session.orca_pane_key.clone(),
                session.orca_worktree_id.clone(),
                session.orca_tab_id.clone(),
            ),
            _ => (None, None, None),
        };
        // SurfaceEvidence 无 cmux_workspace_id 字段(仅诊断用,不进 evidence)。
        let cmux_surface_id = match &session.surface {
            B::Cmux { surface_id, .. } => Some(surface_id.clone()),
            _ => None,
        };
        let (wezterm_pane_id, wezterm_unix_socket, wezterm_executable) = match &session.surface {
            B::Wezterm {
                pane_id,
                unix_socket,
                executable,
            } => (
                Some(pane_id.clone()),
                unix_socket.clone(),
                executable.clone(),
            ),
            _ => (None, None, None),
        };
        let (otty_pane_id, otty_tab_id, otty_window_id) = match &session.surface {
            B::Otty {
                pane_id,
                tab_id,
                window_id,
            } => (Some(pane_id.clone()), tab_id.clone(), window_id.clone()),
            _ => (None, None, None),
        };
        let (tmux_pane_id, tmux_socket_path) = match &session.surface {
            B::Tmux {
                pane_id,
                socket_path,
            } => (Some(pane_id.clone()), socket_path.clone()),
            _ => (None, None),
        };
        Self {
            source: session.source.clone(),
            terminal_app: session.terminal_app.clone(),
            task_id: session.task_id.clone(),
            session_id: session.id.clone(),
            external_session_id: session.external_session_id.clone(),
            workspace_path: session.workspace_path.clone(),
            dsh_nav_socket: session.dsh_nav_socket.clone(),
            terminal_tty: session.terminal_tty.clone(),
            terminal_id: session.terminal_id.clone(),
            terminal_id_captured_cwd: session.terminal_id_captured_cwd.clone(),
            herdr_pane_id,
            herdr_socket_path,
            cmux_surface_id,
            wezterm_pane_id,
            wezterm_unix_socket,
            wezterm_executable,
            otty_pane_id,
            otty_tab_id,
            otty_window_id,
            tmux_pane_id,
            tmux_socket_path,
            orca_pane_key,
            orca_worktree_id,
            orca_tab_id,
        }
    }
}

/// Session 存储层的**内层绑定**单一真源(W5,brief 20260813-surface-binding,
/// 开工前修订版:只承载内层绑定,宿主层保持平铺)。
///
/// 每个终端宿主一个 variant;`None` = 无内层绑定。写侧一律经
/// `SessionStore::set_surface_binding*` 写入 surface **并同步平铺镜像字段**;
/// W5 期间镜像仍是运行时真源(读侧 W6 切),枚举只被写、不被运行时读。
///
/// 注意:herdr 跑在 Orca 外层时同一 session 可同时持有 herdr + orca 绑定
/// (`commands/focus/orca_herdr.rs::apply_orca_binding_for_herdr_host_if_missing`),
/// 枚举是单值,只能承载其中一个;共存由平铺镜像承载(W6 读侧再处理多值)。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceBinding {
    /// 无内层绑定(默认)。
    #[default]
    None,
    /// herdr pane 绑定(`herdr_pane_id` + 可选 workspace/socket)。
    Herdr {
        pane_id: String,
        workspace_id: Option<String>,
        socket_path: Option<String>,
    },
    /// cmux pane 绑定(`cmux_surface_id` + 可选 workspace)。
    Cmux {
        surface_id: String,
        workspace_id: Option<String>,
    },
    /// WezTerm pane 绑定(`wezterm_pane_id` + 可选 socket/executable)。
    Wezterm {
        pane_id: String,
        unix_socket: Option<String>,
        executable: Option<String>,
    },
    /// Otty pane 绑定(`otty_pane_id` + 可选 tab/window)。
    Otty {
        pane_id: String,
        tab_id: Option<String>,
        window_id: Option<String>,
    },
    /// 原生 tmux pane 绑定(`tmux_pane_id` + 可选 socket)。
    Tmux {
        pane_id: String,
        socket_path: Option<String>,
    },
    /// Orca worktree pane 绑定(`orca_pane_key` + 可选 worktree/tab)。
    Orca {
        pane_key: String,
        worktree_id: Option<String>,
        tab_id: Option<String>,
    },
}

/// herdr-remote 源判定单点(brief 20260813-focus-dispatch-table):`source ==
/// "herdr-remote"` 表示 session 由远程 herdr 监视器管理(agent 跑在远程机,
/// SSH 转发 socket,见 herdr_remote.rs 模块注释)。所有消费点经本函数,
/// 避免同一语义多处字符串比对。
pub fn is_herdr_remote_source(source: Option<&str>) -> bool {
    source == Some("herdr-remote")
}

impl SurfaceBinding {
    /// 从 Session 平铺字段推导内层绑定(加载迁移 / 测试用)。
    ///
    /// primary 选择顺序 = focus ladder 的 binding 检查顺序
    /// (`commands/focus/short_circuit.rs::run_terminal_short_circuits`:
    /// herdr → cmux → wezterm → orca → otty → tmux)。触发条件取各 binding 的
    /// 主键字段在场(pane_id / surface_id / pane_key;旧 set* 方法写入时保证
    /// 主键非空)。
    pub fn from_session(session: &crate::sessions::Session) -> Self {
        if is_present(&session.herdr_pane_id) {
            return Self::Herdr {
                pane_id: session.herdr_pane_id.clone().unwrap_or_default(),
                workspace_id: session.herdr_workspace_id.clone(),
                socket_path: session.herdr_socket_path.clone(),
            };
        }
        if is_present(&session.cmux_surface_id) {
            return Self::Cmux {
                surface_id: session.cmux_surface_id.clone().unwrap_or_default(),
                workspace_id: session.cmux_workspace_id.clone(),
            };
        }
        if is_present(&session.wezterm_pane_id) {
            return Self::Wezterm {
                pane_id: session.wezterm_pane_id.clone().unwrap_or_default(),
                unix_socket: session.wezterm_unix_socket.clone(),
                executable: session.wezterm_executable.clone(),
            };
        }
        if is_present(&session.orca_pane_key) {
            return Self::Orca {
                pane_key: session.orca_pane_key.clone().unwrap_or_default(),
                worktree_id: session.orca_worktree_id.clone(),
                tab_id: session.orca_tab_id.clone(),
            };
        }
        if is_present(&session.otty_pane_id) {
            return Self::Otty {
                pane_id: session.otty_pane_id.clone().unwrap_or_default(),
                tab_id: session.otty_tab_id.clone(),
                window_id: session.otty_window_id.clone(),
            };
        }
        if is_present(&session.tmux_pane_id) {
            return Self::Tmux {
                pane_id: session.tmux_pane_id.clone().unwrap_or_default(),
                socket_path: session.tmux_socket_path.clone(),
            };
        }
        Self::None
    }

    /// 主键字段(ladder 定位用的稳定身份):Herdr/Wezterm/Otty/Tmux → pane_id,
    /// Cmux → surface_id,Orca → pane_key。用于 `clear_surface_binding_if_matches`
    /// 的主键匹配(调用点只持有主键,与旧 clear_*_if_matches 只比主键一致)。
    pub fn primary_key(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Herdr { pane_id, .. }
            | Self::Wezterm { pane_id, .. }
            | Self::Otty { pane_id, .. }
            | Self::Tmux { pane_id, .. } => Some(pane_id),
            Self::Cmux { surface_id, .. } => Some(surface_id),
            Self::Orca { pane_key, .. } => Some(pane_key),
        }
    }

    /// herdr+orca 共存特例下的 orca 字段读取(W6 review fix):
    /// surface=Orca 取 surface;surface=Herdr 时 orca host hint 在镜像
    /// (herdr 刷新后的写序);其余无 orca。与 [`herdr_fields`] 对称,
    /// 供 SessionFocusBindings / 重发 wire 等非 evidence 读点复用
    /// (语义同 `SurfaceEvidence::from_session` 的 orca 分支)。
    pub fn orca_fields(
        session: &crate::sessions::Session,
    ) -> Option<(&str, Option<&str>, Option<&str>)> {
        match &session.surface {
            Self::Orca {
                pane_key,
                worktree_id,
                tab_id,
            } => Some((pane_key.as_str(), worktree_id.as_deref(), tab_id.as_deref())),
            Self::Herdr { .. } => session.orca_pane_key.as_deref().map(|pane_key| {
                (
                    pane_key,
                    session.orca_worktree_id.as_deref(),
                    session.orca_tab_id.as_deref(),
                )
            }),
            _ => None,
        }
    }

    /// 与另一 binding 是否同 variant(clear 主键匹配用)。
    pub fn same_variant(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// herdr+orca 共存特例下的 herdr 字段读取(W6,brief 20260813-surface-readers):
    /// surface=Herdr 取 surface;surface=Orca 时 herdr 在镜像(notify 先 herdr 后
    /// orca host hint 的写序);其余无 herdr。供 capture 资格 / herdr 刷新 /
    /// 探活等非 evidence 读点复用(与 evidence 的 herdr 分支同语义)。
    pub fn herdr_fields(
        session: &crate::sessions::Session,
    ) -> Option<(&str, Option<&str>, Option<&str>)> {
        match &session.surface {
            Self::Herdr {
                pane_id,
                workspace_id,
                socket_path,
            } => Some((
                pane_id.as_str(),
                workspace_id.as_deref(),
                socket_path.as_deref(),
            )),
            Self::Orca { .. } => session.herdr_pane_id.as_deref().map(|pane| {
                (
                    pane,
                    session.herdr_workspace_id.as_deref(),
                    session.herdr_socket_path.as_deref(),
                )
            }),
            _ => None,
        }
    }
}

pub fn log_resolution(
    channel: &str,
    action: &str,
    evidence: &SurfaceEvidence,
    resolved: &ResolvedSurface,
) {
    crate::log_debug!(
        channel,
        "surface action={} session={} source={:?} kind={:?} confidence={:?} reason={} tty_present={} terminal_id_present={}",
        action,
        evidence.session_id,
        evidence.source,
        resolved.kind,
        resolved.confidence,
        resolved.reason,
        evidence.terminal_tty.as_deref().is_some_and(|s| !s.is_empty()),
        evidence.terminal_id.as_deref().is_some_and(|s| !s.is_empty())
    );
}

/// popup ① 弹前抑制 gate 的 INFO 摘要:bindings + surface + already_focused,抑制与弹出都打。
pub fn binding_hints_for_log(evidence: &SurfaceEvidence) -> String {
    fn hint(value: &Option<String>) -> &'static str {
        if value.as_deref().is_some_and(|s| !s.is_empty()) {
            "present"
        } else {
            "absent"
        }
    }
    format!(
        "orca_pane={} otty_pane={} herdr_pane={} tmux_pane={} term_app={} tty={}",
        hint(&evidence.orca_pane_key),
        hint(&evidence.otty_pane_id),
        hint(&evidence.herdr_pane_id),
        hint(&evidence.tmux_pane_id),
        evidence.terminal_app.as_deref().unwrap_or("absent"),
        evidence.terminal_tty.as_deref().unwrap_or("absent"),
    )
}

pub fn log_focus_gate_decision(
    task_id: &str,
    session_id: &str,
    decision: &str,
    already_focused: bool,
    evidence: &SurfaceEvidence,
    resolved: &ResolvedSurface,
    suppress_reason: Option<&str>,
) {
    crate::log_info!(
        "notify",
        "popup focus_gate task_id={} session_id={} decision={} already_focused={} surface_kind={:?} surface_reason={} bindings={} suppress_reason={}",
        task_id,
        session_id,
        decision,
        already_focused,
        resolved.kind,
        resolved.reason,
        binding_hints_for_log(evidence),
        suppress_reason.unwrap_or("-"),
    );
}

fn terminal_focus_capabilities(evidence: &SurfaceEvidence) -> SurfaceCapabilities {
    SurfaceCapabilities {
        can_focus: true,
        can_detect_focused: is_present(&evidence.terminal_tty)
            || is_present(&evidence.terminal_id)
            || is_present(&evidence.cmux_surface_id)
            || is_present(&evidence.herdr_pane_id)
            || (is_present(&evidence.wezterm_pane_id) && is_present(&evidence.wezterm_unix_socket))
            || (is_present(&evidence.tmux_pane_id) && is_present(&evidence.tmux_socket_path)),
        can_locate_screen: is_present(&evidence.terminal_tty) || is_present(&evidence.terminal_id),
        can_check_liveness: true,
    }
}

fn binding_focus_capabilities() -> SurfaceCapabilities {
    SurfaceCapabilities {
        can_focus: true,
        can_detect_focused: true,
        can_locate_screen: false,
        can_check_liveness: true,
    }
}

pub fn resolve_surface(evidence: &SurfaceEvidence) -> ResolvedSurface {
    // Orca / Otty binding 优先于 tty:同一 session 常同时带 stale tty(旧 pane 残留)
    // 与精确 pane_key;若先判 tty 会误记为 TerminalTty,聚焦抑制与 watcher 走错分支。
    if is_present(&evidence.orca_pane_key) {
        return ResolvedSurface {
            kind: SurfaceKind::OrcaTerminal,
            confidence: SurfaceConfidence::Exact,
            capabilities: binding_focus_capabilities(),
            display_name: "Orca",
            reason: "exact_orca_pane",
        };
    }

    if is_present(&evidence.otty_pane_id) {
        return ResolvedSurface {
            kind: SurfaceKind::OttyTerminal,
            confidence: SurfaceConfidence::Exact,
            capabilities: binding_focus_capabilities(),
            display_name: "Otty",
            reason: "exact_otty_pane",
        };
    }

    if is_present(&evidence.terminal_tty) {
        return ResolvedSurface {
            kind: SurfaceKind::TerminalTty,
            confidence: SurfaceConfidence::Exact,
            capabilities: terminal_focus_capabilities(evidence),
            display_name: "Terminal",
            reason: "exact_tty",
        };
    }

    if is_present(&evidence.terminal_id) {
        return ResolvedSurface {
            kind: SurfaceKind::GhosttyTerminal,
            confidence: SurfaceConfidence::Exact,
            capabilities: all_capabilities(),
            display_name: "Ghostty",
            reason: "exact_ghostty_terminal_id",
        };
    }

    // ChatGPT 桌面端会话:codex 跑在 Electron 里,没有 tty / terminal_id,但 hook
    // payload 带 session_id(== thread UUID,见 pokehook::external_session_id)。靠
    // `codex://threads/<id>` deep link 精准跳回那条对话 -- 比 Finder no-op 强得多。
    // codex CLI 在终端里跑时有 tty,已在上面 TerminalTty 分支返回,不会误入这里。
    // DeepSeek Harness:GUI/Web 宿主,没有 tty 也没有 per-session deep link。身份是
    // hook 上报的 session id(`session-<uuid>`),回跳由 host 半插件的本地 socket
    // 请求渲染进程 `uiWorkspace.openSession`(brief 20261104-dsh-integration)。
    if evidence.source.as_deref() == Some("dsh") && is_present(&evidence.external_session_id) {
        return ResolvedSurface {
            kind: SurfaceKind::DshSession,
            confidence: SurfaceConfidence::Exact,
            capabilities: SurfaceCapabilities {
                can_focus: true,
                // 有 socket 就能现查渲染进程 `showing/focused/visible`(与终端
                // focus gate 同语义);没有 socket 时不启动 watcher,避免空转。
                can_detect_focused: is_present(&evidence.dsh_nav_socket),
                can_locate_screen: false,
                can_check_liveness: false,
            },
            display_name: "DeepSeek Harness",
            reason: "dsh_session",
        };
    }

    if evidence.source.as_deref() == Some("codex") && is_present(&evidence.external_session_id) {
        return ResolvedSurface {
            kind: SurfaceKind::CodexDesktop,
            confidence: SurfaceConfidence::Exact,
            capabilities: SurfaceCapabilities {
                can_focus: true,
                can_detect_focused: false,
                can_locate_screen: false,
                can_check_liveness: false,
            },
            display_name: "ChatGPT",
            reason: "codex_desktop_thread",
        };
    }

    if is_present(&evidence.wezterm_pane_id) && is_present(&evidence.wezterm_unix_socket) {
        return ResolvedSurface {
            kind: SurfaceKind::Unknown,
            confidence: SurfaceConfidence::Exact,
            capabilities: binding_focus_capabilities(),
            display_name: "WezTerm",
            reason: "exact_wezterm_pane",
        };
    }

    // herdr binding:本地 herdr 聚焦可判(outer+pane 两级,is_surface_focused 的
    // has_herdr_binding 分支优先于 surface kind),can_detect_focused=true 让
    // focus-watcher 启动、用户聚焦回 pane 时自动回收 popup(与 otty/orca 对齐);
    // 远程 herdr-remote 聚焦不可判(20260807 决策,is_herdr_bound_session_focused
    // 恒 false),can_detect_focused=false 避免 watcher 每 500ms osascript 空转。
    if is_present(&evidence.herdr_pane_id) && is_present(&evidence.herdr_socket_path) {
        let remote = crate::surface::is_herdr_remote_source(evidence.source.as_deref());
        let capabilities = if remote {
            SurfaceCapabilities {
                can_focus: true,
                can_detect_focused: false,
                can_locate_screen: false,
                can_check_liveness: true,
            }
        } else {
            binding_focus_capabilities()
        };
        return ResolvedSurface {
            kind: SurfaceKind::HerdrPane,
            confidence: SurfaceConfidence::Exact,
            capabilities,
            display_name: "herdr",
            reason: "exact_herdr_binding",
        };
    }

    // tmux binding:无 pane 级 GUI 定位,binding 在场按 binding 类解析(can_detect_focused
    // = true,聚焦抑制可判)。优先级低于 herdr/cmux/wezterm/otty/orca(它们先命中或带 tty)。
    if is_present(&evidence.tmux_pane_id) && is_present(&evidence.tmux_socket_path) {
        return ResolvedSurface {
            kind: SurfaceKind::Unknown,
            confidence: SurfaceConfidence::Exact,
            capabilities: binding_focus_capabilities(),
            display_name: "tmux",
            reason: "exact_tmux_pane",
        };
    }

    if is_present(&evidence.workspace_path) {
        if evidence.source.as_deref() == Some("cursor") {
            return ResolvedSurface {
                kind: SurfaceKind::CursorWorkspace,
                confidence: SurfaceConfidence::Probable,
                capabilities: SurfaceCapabilities {
                    can_focus: true,
                    can_detect_focused: true,
                    can_locate_screen: false,
                    can_check_liveness: true,
                },
                display_name: "Cursor",
                reason: "cursor_gui_fallback",
            };
        }

        return ResolvedSurface {
            kind: SurfaceKind::FinderWorkspace,
            confidence: SurfaceConfidence::Fallback,
            capabilities: SurfaceCapabilities {
                can_focus: true,
                can_detect_focused: false,
                can_locate_screen: false,
                can_check_liveness: false,
            },
            display_name: "Finder",
            reason: "finder_fallback",
        };
    }

    ResolvedSurface {
        kind: SurfaceKind::Unknown,
        confidence: SurfaceConfidence::Unknown,
        capabilities: SurfaceCapabilities {
            can_focus: false,
            can_detect_focused: false,
            can_locate_screen: false,
            can_check_liveness: false,
        },
        display_name: "Unknown",
        reason: "missing_surface_evidence",
    }
}

fn is_present(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|value| !value.is_empty())
}

fn all_capabilities() -> SurfaceCapabilities {
    SurfaceCapabilities {
        can_focus: true,
        can_detect_focused: true,
        can_locate_screen: true,
        can_check_liveness: true,
    }
}

#[cfg(test)]
#[path = "surface_binding_tests.rs"]
mod binding_tests;
#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
