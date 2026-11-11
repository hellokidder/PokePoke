use crate::surface::SurfaceBinding;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
// Instant / 下列两个常量 / spawn_flush_worker 的 store 参数都只在
// `#[cfg(not(test))]` 分支里用,test 构建下 clippy 会报 unused。这是条件编译的
// false positive,不能用删 import / 改 _store 来消(非 test 构建会编译失败),
// 统一在模块顶部 allow。
#[allow(unused_imports)]
use std::time::Instant;

/// upsert 等热路径写盘 debounce 窗口。测试构建仍同步写盘,见 `schedule_save`。
#[allow(dead_code)]
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
#[allow(dead_code)]
const FLUSH_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    #[default]
    Normal,
    High,
}

// Session 状态机（Task C 重构后）：
// 这四种都是"agent 活着"时的一种状态，没有"终态"概念。
// session 的生命周期唯一由宿主进程存活决定，见 lib.rs 的探活线程。
//
// - Running    : agent 正在处理某一轮（UserPromptSubmit / SessionStart）
// - Pending    : agent 停下等用户操作（CC Notification 权限询问等）
// - Idle       : 上一轮正常结束，agent 空闲等下一轮（Stop hook）
// - LastFailed : 上一轮因 API 错误结束，agent 仍活着（StopFailure hook）
//
// serde alias 保持对老 sessions.json 的向下兼容：
// - "success" -> Idle
// - "failure" -> LastFailed
// 新版本写入时使用新名字，老版本读新文件会反序列化失败（单向升级）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
// 用 snake_case 是为了让 `LastFailed` 正确序列化成 "last_failed"。
// 原来用 lowercase 会合并成 "lastfailed"，和前端 types.ts / i18n key 的
// "last_failed" 对不上，导致 StatusDot 的 Record 索引为 undefined，
// 整个 panel 白屏。Running/Pending/Idle 都是单个单词，snake_case 与
// lowercase 行为一致，没有兼容成本。
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Pending,
    Running,
    #[serde(alias = "success")]
    Idle,
    #[serde(alias = "failure")]
    LastFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub task_id: String,
    #[serde(default)]
    pub external_session_id: Option<String>,
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub terminal_tty: Option<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    // LastFailed 状态下的 reason code（如 CC StopFailure 的 matcher 值）。
    // 只在 status=LastFailed 时有意义；前端负责根据 reason 做本地化展示。
    // 老 sessions.json 里没有这个字段，serde_default 保证兼容。
    #[serde(default)]
    pub failure_reason: Option<String>,
    // T1(brief 20260805-t1-cursor-aborted):cursor Ctrl+C 取消任务的时刻戳。
    // pokehook 在 stop|idle(aborted) 的 payload 带 `aborted: true`,upsert 时写入。
    // 用途:1) ABORTED_ECHO_WINDOW 内识别同轮双发的 last_failed echo(保持 idle);
    // 2) 音效抑制(取消不播"完成"音)。新一轮 Running 到来时清空。
    // 老 sessions.json 无该字段,serde_default 保证兼容。
    #[serde(default)]
    pub aborted_at: Option<DateTime<Utc>>,
    // Ghostty 1.3+ terminal surface stable UUID,SessionStart 到达 server 时
    // 通过 osascript `id of focused terminal of front window` 抓到并存入。
    // 后续 focus_ghostty 优先用 id 精准跳 tab,避开同 cwd 多 session 的
    // first-match 偏差(see brief 20260520-ghostty-terminal-focus.md)。
    // iTerm2 / Terminal.app / Cursor session 永远 None(只 Ghostty 走这个路径)。
    // 老 sessions.json 反序列化无该字段时 serde_default 保证为 None。
    #[serde(default)]
    pub terminal_id: Option<String>,
    // Ghostty terminal_id 写入时同步记录 terminal object 当时的 working
    // directory。focus L1 用它验证绑定,避免后续 workspace_path upsert 漂移
    // 把原本合法的 terminal_id 误判为 stale。
    #[serde(default)]
    pub terminal_id_captured_cwd: Option<String>,
    // cc/codex hook payload 携带的 transcript jsonl 路径;Claude transcript watcher 等使用。
    // upsert 时 Some(_) 才覆盖,允许后续事件 refresh,空 None 保留旧值(同 source
    // 同 session_id 路径稳定,不会变)。cursor 当前 schema 不带 → 永远 None。
    // 老 sessions.json 反序列化无该字段 serde_default 保证为 None。
    #[serde(default)]
    pub transcript_path: Option<String>,
    // brief 20260709-herdr-jump-support:CC 跑在 herdr pane 里时,pokehook 采集
    // 的 HERDR_PANE_ID / HERDR_WORKSPACE_ID / HERDR_SOCKET_PATH 三件套。
    // 跳回时用于二段动作:段 A 走既有 Ghostty terminal_id / title 前缀(herdr
    // 自动把 `<ws> · cc-<hex8>` 写进外层 Ghostty tab title),段 B 调 herdr
    // socket `agent.focus <pane_id>` 让 herdr 内部切到 CC pane。
    // 直接在 Ghostty 里跑的 CC 永远 None;老 sessions.json 无该字段 serde_default
    // 保证兼容。
    //
    // W5(brief 20260813-surface-binding)起,以下 6 组内层绑定字段是
    // **legacy mirror**:写侧一律经 `surface` 字段 + `set_surface_binding*`
    // 双写,本组字段保持同步;W5 期间仍是运行时真源(读侧 W6 切换后降级为
    // 迁移兼容镜像)。herdr 与 orca 可共存(herdr-in-Orca 外层锚点)。
    #[serde(default)]
    pub herdr_pane_id: Option<String>,
    #[serde(default)]
    pub herdr_workspace_id: Option<String>,
    #[serde(default)]
    pub herdr_socket_path: Option<String>,
    // brief 20260709-cmux-jump-support:CC 跑在 cmux pane 里时,pokehook 从环境
    // 变量采集的两件套。cmux 用 libghostty 渲染,pane 内 `TERM_PROGRAM=ghostty`,
    // 会误让 server 触发真 Ghostty capture(枚举不到 cmux surface,退 cwd
    // first-match 撞无关窗口)。这两字段一存在就走 cmux short-circuit
    // `cmux focus-panel --panel <UUID>`,完全绕开 target ladder。
    // surface_id 是 UUID(pane 生命周期恒定,重启即换),`set` 允许覆盖 + 幂等;
    // workspace_id 仅供诊断日志,不参与 focus 决策。
    // 直接跑 Ghostty / iTerm2 / Terminal.app 的 session 永远 None;老 sessions.json
    // 无该字段 serde_default 保证兼容。
    #[serde(default)]
    pub cmux_surface_id: Option<String>,
    #[serde(default)]
    pub cmux_workspace_id: Option<String>,
    // brief 20260724-wezterm-jump-support:CC 直接跑在 WezTerm pane 里时,pokehook
    // 从 `WEZTERM_PANE` + `WEZTERM_UNIX_SOCKET` 采集。pane_id 是可复用整数(非 UUID),
    // focus 后必须 cwd 校验;herdr / cmux env 存在时不采集 wezterm(优先级更低)。
    #[serde(default)]
    pub wezterm_pane_id: Option<String>,
    #[serde(default)]
    pub wezterm_unix_socket: Option<String>,
    #[serde(default)]
    pub wezterm_executable: Option<String>,
    // brief 20260727-otty-support:CC 直接跑在 Otty pane 里时,pokehook 从
    // `OTTY_PANE_ID` 采集。pane_id 格式 `p_<hex>`(pane 生命周期恒定),
    // focus 通过 otty CLI / AppleScript 精准跳转。herdr/cmux/wezterm env
    // 存在时不采集 otty(优先级最低)。
    #[serde(default)]
    pub otty_pane_id: Option<String>,
    #[serde(default)]
    pub otty_tab_id: Option<String>,
    #[serde(default)]
    pub otty_window_id: Option<String>,
    // brief 20260804-tmux-binding:agent 跑在原生 tmux pane 里时,pokehook 从
    // `TMUX_PANE` + `TMUX` 采集。tmux 无 pane 级 GUI 定位,focus 只做
    // best-effort select-pane 后仍走外层终端 ladder。herdr/cmux/wezterm/otty
    // env 存在时不采集(优先级最低)。
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
    #[serde(default)]
    pub tmux_socket_path: Option<String>,
    // brief 20260810-warp-support:终端宿主(app 名小写归一化,如 warpterminal /
    // ghostty),notify 时从 req.terminal_app 写入。focus 时用于判定 Warp 会话
    // 必须走 warp 分支,避免落 iTerm2/Ghostty ladder 误吸。老 sessions.json
    // 无该字段 serde_default 保证兼容。
    #[serde(default)]
    pub terminal_app: Option<String>,
    // brief 20260728-orca-support:agent 跑在 Orca worktree pane 里时,pokehook 从
    // `ORCA_PANE_KEY` + `ORCA_WORKTREE_ID` 采集。pane_key 是 `tabId:leafUuid`
    // 稳定身份;handle(`term_*`) 会 remint,不持久化。herdr/cmux/wezterm/otty
    // env 存在时不采集 orca(优先级最低)。
    // brief 20261104-dsh-integration:DSH host 半插件自建的本地控制通道
    // (`~/.pokepoke/dsh-<pid>.sock`)。点击 session 时 PokePoke 通过它请求渲染
    // 进程 `uiWorkspace.openSession` —— DSH 没有 per-session deep link,这是
    // 唯一精确回跳入口。socket 随 DSH 进程走,进程重启后下一次事件刷新。
    #[serde(default)]
    pub dsh_nav_socket: Option<String>,
    #[serde(default)]
    pub orca_pane_key: Option<String>,
    #[serde(default)]
    pub orca_worktree_id: Option<String>,
    #[serde(default)]
    pub orca_tab_id: Option<String>,
    // W5(brief 20260813-surface-binding):内层绑定单一真源(见上方 legacy mirror
    // 注释)。老文件无该字段 → serde default None,由 `SessionStore::load` 迁移。
    #[serde(default)]
    pub surface: SurfaceBinding,
}

impl Session {
    /// `aborted_at` 是否仍在 ABORTED_ECHO_WINDOW 窗口内。
    /// 供 http_server(SuppressWithSound 播音抑制)与 popup.rs(show_popup
    /// 播音抑制)复用,见 brief 20260805-t1-cursor-aborted。
    pub fn is_recently_aborted(&self) -> bool {
        match self.aborted_at {
            Some(t) => {
                (Utc::now() - t).num_milliseconds() <= ABORTED_ECHO_WINDOW.as_millis() as i64
            }
            None => false,
        }
    }
}

pub struct SessionStore {
    pub sessions: Vec<Session>,
    file_path: PathBuf,
    /// 非测试构建:有未落盘变更时为 true。
    #[cfg(not(test))]
    dirty: bool,
    #[cfg(not(test))]
    dirty_since: Option<Instant>,
}

/// cursor Ctrl+C 取消后,识别同轮双发的 `stop|last_failed` artifact echo
/// 与抑制"完成"音的时间窗口(brief 20260805-t1-cursor-aborted)。
/// 实测双发间隔 ~40ms,5s 余量充足;窗口内且无 Running 介入的真错误实际
/// 不可达(agent 要出错必须先跑一轮,而 Running 会清 aborted_at)。
pub const ABORTED_ECHO_WINDOW: Duration = Duration::from_secs(5);

/// Return value from upsert: the session, whether it's new, and the previous status / message if updated.
/// `prev_message` 2026-05-25 加:popup gate `should_show_popup` Pending → Pending 的
/// 抑制规则需要看 message 是否变化(同 Pending 但新 message → 是新需求,应解锁弹)。
/// `upsert_session` 的入参。命名字段替代原先 11 个位置参数--加字段时不再需要
/// 改函数签名，调用点按需用 `..Default::default()` 省略 None 字段。
/// 注意：`task_id` / `title` / `message` 语义上必填，`Default` 仅作构造便利；
/// 生产路径（http_server）应显式赋值，勿依赖 `default()` 的空串。
#[derive(Default)]
pub struct UpsertRequest {
    pub task_id: String,
    pub external_session_id: Option<String>,
    pub title: String,
    pub message: String,
    pub source: Option<String>,
    pub priority: Priority,
    pub status: SessionStatus,
    pub terminal_tty: Option<String>,
    pub workspace_path: Option<String>,
    pub failure_reason: Option<String>,
    pub transcript_path: Option<String>,
    // brief 20260810-warp-support:终端宿主(小写归一化);Some 才覆盖既有值,
    // None(旧 hook 不带)保留原样。
    pub terminal_app: Option<String>,
    // T1(brief 20260805-t1-cursor-aborted):cursor stop|idle(aborted) 事件标记。
    // Some(true) 时 upsert 写 aborted_at=now;None(其它事件)不改变既有值。
    pub aborted: Option<bool>,
    /// brief 20261104-dsh-integration:DSH 本地控制通道;Some 才覆盖。
    pub dsh_nav_socket: Option<String>,
}

pub struct UpsertResult {
    pub session: Session,
    pub is_new: bool,
    pub prev_status: Option<SessionStatus>,
    pub prev_message: Option<String>,
}

impl SessionStore {
    pub fn load(file_path: PathBuf) -> Self {
        let mut sessions: Vec<Session> = if file_path.exists() {
            // 升级迁移:老 sessions.json 可能因 umask 残留 group/other 可读位。
            // 读之前顺手收紧一次,跟 save() 对齐。失败不阻断 load。
            let _ = tighten_file_perms_0600(&file_path);
            fs::read_to_string(&file_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // W5(brief 20260813-surface-binding):老文件无 surface 字段 → 从平铺
        // 镜像字段派生(primary = focus ladder 顺序,见 surface.rs::from_session)。
        // 不触发保存:下一次 upsert/set 写盘时 surface 随双写落盘。
        for s in sessions.iter_mut() {
            if s.surface == SurfaceBinding::None {
                let derived = SurfaceBinding::from_session(s);
                if derived != SurfaceBinding::None {
                    s.surface = derived;
                }
            }
        }
        Self {
            sessions,
            file_path,
            #[cfg(not(test))]
            dirty: false,
            #[cfg(not(test))]
            dirty_since: None,
        }
    }

    /// 后台线程:每 100ms 看 dirty 是否已超过 debounce 窗口,是则 flush。
    /// `store` 参数只在非 test 构建的 spawn 闭包里用,加 allow 避免 test 构建报 unused。
    #[allow(unused_variables)]
    pub fn spawn_flush_worker(store: Arc<Mutex<SessionStore>>) {
        #[cfg(not(test))]
        std::thread::spawn(move || loop {
            std::thread::sleep(FLUSH_POLL_INTERVAL);
            if let Ok(mut guard) = store.lock() {
                guard.maybe_flush_debounced();
            }
        });
    }

    /// 立即把 dirty 状态写入磁盘。退出前必须调一次。
    pub fn flush(&mut self) {
        #[cfg(not(test))]
        {
            if self.dirty {
                self.save();
                self.dirty = false;
                self.dirty_since = None;
            }
        }
    }

    #[cfg(not(test))]
    fn maybe_flush_debounced(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        let due = self
            .dirty_since
            .is_some_and(|since| since.elapsed() >= SAVE_DEBOUNCE);
        if due {
            self.flush();
            true
        } else {
            false
        }
    }

    fn schedule_save(&mut self) {
        #[cfg(test)]
        self.save();
        #[cfg(not(test))]
        {
            self.dirty = true;
            if self.dirty_since.is_none() {
                self.dirty_since = Some(Instant::now());
            }
        }
    }

    fn save(&self) {
        if let Some(parent) = self.file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            &self.file_path,
            serde_json::to_string_pretty(&self.sessions).unwrap_or_default(),
        );
        // 0600:sessions.json 含 workspace_path / transcript_path / message 等用户
        // 敏感信息(diagnostics 模块费很大劲 redact,落盘文件本身也该只属主可读)。
        // 对齐 license/state.rs + settings.rs。失败不致命,仅静默。
        let _ = tighten_file_perms_0600(&self.file_path);
    }

    // 入参用 UpsertRequest struct(见上方),命名字段 + ..Default::default(),
    // 加 session 字段时不再需要改函数签名。
    pub fn upsert_session(&mut self, req: UpsertRequest) -> UpsertResult {
        let UpsertRequest {
            task_id,
            external_session_id,
            title,
            message,
            source,
            priority,
            status,
            terminal_tty,
            workspace_path,
            failure_reason,
            transcript_path,
            aborted,
            terminal_app,
            dsh_nav_socket,
        } = req;
        // Try to find existing session by task_id
        if let Some(existing) = self.sessions.iter_mut().find(|s| s.task_id == task_id) {
            let prev_status = existing.status.clone();
            let prev_message = existing.message.clone();

            // T1(brief 20260805-t1-cursor-aborted):cursor Ctrl+C 双发 stop
            // (~40ms),第二条 hookStatus 缺失/error → last_failed,是同一轮取消的
            // artifact echo:不覆盖 idle、不写 failure_reason、不触发 save 变更。
            // 返回的 session 仍是 idle(prev=idle → popup gate 自然 Skip,不弹不播)。
            let aborted_echo = status == SessionStatus::LastFailed
                && aborted != Some(true)
                && existing.is_recently_aborted();
            if aborted_echo {
                crate::log_debug!(
                    "notify",
                    "last_failed suppressed: aborted turn echo task_id={}",
                    task_id
                );
                let session = existing.clone();
                return UpsertResult {
                    session,
                    is_new: false,
                    prev_status: Some(prev_status),
                    prev_message: Some(prev_message),
                };
            }
            if external_session_id.is_some() {
                existing.external_session_id = external_session_id;
            }
            existing.title = title;
            existing.message = message;
            if source.is_some() {
                existing.source = source;
            }
            existing.priority = priority;
            existing.status = status.clone();
            // T1:aborted 事件钉住 aborted_at(取消窗口);新一轮 Running 清掉。
            if aborted == Some(true) {
                existing.aborted_at = Some(Utc::now());
            } else if status == SessionStatus::Running {
                existing.aborted_at = None;
            }
            existing.updated_at = Utc::now();
            if terminal_tty.is_some() {
                existing.terminal_tty = terminal_tty;
            }
            if workspace_path.is_some() {
                existing.workspace_path = workspace_path;
            }
            // failure_reason：只在进入 LastFailed 时才覆盖；其他状态下清空，避免
            // 一条 session 从 LastFailed 回到 Running/Idle 后仍显示旧的失败原因。
            existing.failure_reason = if status == SessionStatus::LastFailed {
                failure_reason
            } else {
                None
            };
            // transcript_path:只在 Some(_) 时覆盖;空字符串当作 None 处理(防御边界)。
            // session_id 稳定 → 路径稳定,允许新事件 refresh,旧 hook 不带时保留旧值。
            if let Some(tp) = transcript_path {
                if !tp.is_empty() {
                    existing.transcript_path = Some(tp);
                }
            }
            // terminal_app:只在 Some 时覆盖(同一 hook 进程 env 稳定,防旧事件冲刷)。
            if terminal_app.is_some() {
                existing.terminal_app = terminal_app;
            }
            // dsh_nav_socket:只在 Some 时覆盖;DSH 重启换 pid 后由下一次事件刷新。
            if dsh_nav_socket.is_some() {
                existing.dsh_nav_socket = dsh_nav_socket;
            }
            let session = existing.clone();
            self.schedule_save();
            UpsertResult {
                session,
                is_new: false,
                prev_status: Some(prev_status),
                prev_message: Some(prev_message),
            }
        } else {
            let now = Utc::now();
            // 同理：非 LastFailed 新 session 不该携带 failure_reason
            let failure_reason = if status == SessionStatus::LastFailed {
                failure_reason
            } else {
                None
            };
            // 空 transcript_path 同样归一化到 None
            let transcript_path = transcript_path.filter(|s| !s.is_empty());
            let session = Session {
                id: uuid::Uuid::new_v4().to_string(),
                task_id,
                external_session_id,
                title,
                message,
                source,
                priority,
                status,
                created_at: now,
                updated_at: now,
                terminal_tty,
                workspace_path,
                failure_reason,
                aborted_at: if aborted == Some(true) {
                    Some(now)
                } else {
                    None
                },
                // 新建 session 时永远 None;Ghostty 走 server 端 SessionStart
                // 触发 osascript 异步 capture 后通过 set_terminal_id 写入。
                terminal_id: None,
                terminal_id_captured_cwd: None,
                transcript_path,
                herdr_pane_id: None,
                herdr_workspace_id: None,
                herdr_socket_path: None,
                // 新建 session 时永远 None;cmux binding 由 process_notify 里
                // set_cmux_binding 在 /notify 到达后立即写入(pokehook 已把 env
                // 两件套塞进 payload)。
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
                // 终端宿主从 notify 的 terminal_app 写入(Some 才覆盖)。
                terminal_app,
                // DSH 本地控制通道(仅 source=dsh 的事件带)。
                dsh_nav_socket,
                // 新建 session 时无内层绑定;binding 由 process_notify / capture 经
                // set_surface_binding* 写入(W5,brief 20260813-surface-binding)。
                surface: SurfaceBinding::None,
            };
            self.sessions.insert(0, session.clone());
            self.schedule_save();
            UpsertResult {
                session,
                is_new: true,
                prev_status: None,
                prev_message: None,
            }
        }
    }

    pub fn get_all(&self) -> &[Session] {
        &self.sessions
    }

    /// 按 PokePoke 内部 session.id 查是否存在。
    ///
    /// 用于 popup show 前的 race guard:`handle_session_end` 按 task_id 删 session,
    /// 若并发的 idle 事件在 `handle_session_end` 之前 upsert 出新 session(新 id,
    /// 新 binding=None),再用 stale 的 `UpsertResult.session` clone 去 show 就会
    /// 给"已被删的旧 session.id"弹幽灵 popup。show 前重查 store,确认该 id 还在。
    pub fn session_exists(&self, id: &str) -> bool {
        self.sessions.iter().any(|s| s.id == id)
    }

    pub fn remove_session(&mut self, id: &str) -> bool {
        let len = self.sessions.len();
        self.sessions.retain(|s| s.id != id);
        if self.sessions.len() < len {
            self.schedule_save();
            true
        } else {
            false
        }
    }

    pub fn remove_session_by_task_id(&mut self, task_id: &str) -> Option<Session> {
        let index = self.sessions.iter().position(|s| s.task_id == task_id)?;
        let removed = self.sessions.remove(index);
        self.schedule_save();
        Some(removed)
    }

    /// 给指定 task_id 的 session 写入 Ghostty terminal_id + capture cwd snapshot。
    /// 只在该字段当前为 None 时写,不覆盖已有值(幂等)。
    ///
    /// b20530 加固:若同一 terminal_id 已被其它 task 且不同 tty 持有,拒写。
    /// 同 tty 允许复用,覆盖同一个 tab 内连续开 agent 的合法场景。
    pub fn set_terminal_id_if_unset(
        &mut self,
        task_id: &str,
        terminal_id: String,
        captured_cwd: String,
    ) -> bool {
        let Some(index) = self.sessions.iter().position(|s| s.task_id == task_id) else {
            return false;
        };
        if self.sessions[index].terminal_id.is_some() {
            return false;
        }
        let target_tty = normalized_tty(self.sessions[index].terminal_tty.as_deref());
        let duplicate_different_tty = self.sessions.iter().any(|s| {
            s.task_id != task_id
                && s.terminal_id.as_deref() == Some(terminal_id.as_str())
                && normalized_tty(s.terminal_tty.as_deref()) != target_tty
        });
        if duplicate_different_tty {
            crate::log_info!(
                "ghostty",
                "refusing duplicate terminal_id task_id={} id={} tty={:?} (held by different tty)",
                task_id,
                terminal_id,
                target_tty
            );
            return false;
        }
        let session = &mut self.sessions[index];
        session.terminal_id = Some(terminal_id);
        session.terminal_id_captured_cwd = Some(captured_cwd);
        self.schedule_save();
        true
    }

    /// 周期对账写回 terminal_id(brief 20260629-ghostty-jump-reconcile)。
    ///
    /// 与 `set_terminal_id_if_unset` 的区别在**覆盖语义**:
    /// - `by_title=true`(title 主键命中):强证据,**允许覆盖**已有 terminal_id
    ///   —— 这是"纠正错绑"的核心。历史上 cwd first-match 可能把 session 绑到同 cwd
    ///   的邻居 tab,title 命中后必须能改正。
    /// - `by_title=false`(cwd 兜底命中):弱证据,**仅在当前为空时写**,不覆盖
    ///   —— cwd 本就可能撞车,不能让它推翻已有绑定。
    ///
    /// 防撞保护保留:若目标 terminal_id 已被**另一个不同 tty** 的 task 持有,拒写
    /// (跟 `set_terminal_id_if_unset` 一致,防一个 UUID 绑到多个 session)。
    ///
    /// 返回是否真的改了值(false = 没找到 task / 值未变 / 被防撞拒写 / 弱证据不覆盖)。
    pub fn reconcile_terminal_id(
        &mut self,
        task_id: &str,
        terminal_id: String,
        captured_cwd: String,
        by_title: bool,
    ) -> bool {
        let Some(index) = self.sessions.iter().position(|s| s.task_id == task_id) else {
            return false;
        };

        // 值没变:幂等省写。
        if self.sessions[index].terminal_id.as_deref() == Some(terminal_id.as_str()) {
            return false;
        }

        // 已有值且非 title 强证据:不覆盖。
        if self.sessions[index].terminal_id.is_some() && !by_title {
            return false;
        }

        // 防撞:同一 terminal_id 被别的、且不同 tty 的 task 持有 → 拒写。
        let target_tty = normalized_tty(self.sessions[index].terminal_tty.as_deref());
        let duplicate_different_tty = self.sessions.iter().any(|s| {
            s.task_id != task_id
                && s.terminal_id.as_deref() == Some(terminal_id.as_str())
                && normalized_tty(s.terminal_tty.as_deref()) != target_tty
        });
        if duplicate_different_tty {
            crate::log_info!(
                "ghostty",
                "reconcile refusing duplicate terminal_id task_id={} id={} tty={:?} (held by different tty)",
                task_id,
                terminal_id,
                target_tty
            );
            return false;
        }

        let session = &mut self.sessions[index];
        session.terminal_id = Some(terminal_id);
        session.terminal_id_captured_cwd = Some(captured_cwd);
        self.schedule_save();
        true
    }

    /// 2026-05-25 加固:focus_ghostty 发现 stored id 已 stale 时反向通知
    /// caller 调本方法,后续 click 直接走 L2 cwd 兜底,避开稳定跳错。
    pub fn clear_terminal_id_if_matches(&mut self, session_id: &str, expected: &str) -> bool {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return false;
        };
        if session.terminal_id.as_deref() != Some(expected) {
            return false;
        }
        session.terminal_id = None;
        session.terminal_id_captured_cwd = None;
        self.schedule_save();
        true
    }

    // ==================== surface binding ops(W5,brief 20260813-surface-binding) ====================
    // 内层绑定写侧唯一入口:`surface` 为真源,平铺字段保留为 legacy mirror
    // (W5 期间运行时真源,读侧 W6 切换后降级为迁移兼容)。
    // 替代原 set_herdr_binding / set_cmux_binding / set_wezterm_binding /
    // set_otty_binding / set_tmux_binding / set_orca_binding + 对应 clear_*_if_matches
    // (terminal_id 三件套属宿主层,不参与坍缩,见上)。

    /// 覆盖式写入内层绑定(替代原 6 个 set_*_binding)。按 **task_id** 定位
    /// (与原 set* 一致)。
    ///
    /// 行为逐条复刻原 set_*:
    /// - 幂等:surface 与现值相等 → false 不写(原实现按平铺字段同值 skip);
    /// - `Orca{worktree_id}` 为空 → 拒写(原 set_orca_binding 守卫,防幂等被
    ///   空串打破);
    /// - 镜像同步 [`sync_surface_mirror`]:variant 字段反写平铺镜像 + 原互斥
    ///   规则(Herdr → 清 cmux;Orca → 清 otty);**herdr 与 orca 共存保留**
    ///   (orca 是 herdr 宿主的外层锚点,互不清理对方镜像)。
    pub fn set_surface_binding(&mut self, task_id: &str, binding: SurfaceBinding) -> bool {
        // 原 set_orca_binding 守卫:worktree_id 空拒写(位置在原实现里先于定位)。
        if let SurfaceBinding::Orca { worktree_id, .. } = &binding {
            if worktree_id.as_deref().is_none_or(|w| w.is_empty()) {
                return false;
            }
        }
        let Some(idx) = self.sessions.iter().position(|s| s.task_id == task_id) else {
            return false;
        };
        // 幂等:与原 set* 的同值 skip 等价(surface 与镜像同步,真源一致)。
        if self.sessions[idx].surface == binding {
            return false;
        }
        let s = &mut self.sessions[idx];
        s.surface = binding;
        sync_surface_mirror(s);
        self.schedule_save();
        true
    }

    /// 仅当无内层绑定才写(语义 = 原"不覆盖已有"的调用点;本波暂无调用点,
    /// 供后续 wave / 未来宿主使用)。按 **task_id** 定位。
    /// 与 [`set_surface_binding`] 的区别只在已有绑定时的早退。
    pub fn set_surface_binding_if_missing(
        &mut self,
        task_id: &str,
        binding: SurfaceBinding,
    ) -> bool {
        let Some(idx) = self.sessions.iter().position(|s| s.task_id == task_id) else {
            return false;
        };
        if self.sessions[idx].surface != SurfaceBinding::None {
            return false;
        }
        let s = &mut self.sessions[idx];
        s.surface = binding;
        sync_surface_mirror(s);
        self.schedule_save();
        true
    }

    /// 主键匹配即清(替代原 clear_herdr_binding_if_matches / clear_cmux_binding_if_matches /
    /// clear_wezterm_binding_if_matches / clear_otty_binding_if_matches /
    /// clear_orca_binding_if_matches)。按 **session_id** 定位(与原 clear* 一致)。
    ///
    /// 匹配规则 = 原 clear_*_if_matches 的只比主键行为:surface 同 variant 且
    /// **主键字段**(Herdr/Wezterm/Otty/Tmux → pane_id,Cmux → surface_id,
    /// Orca → pane_key)与 expected 相等(调用点只持有主键,构造不了完整
    /// variant)。清空后按 variant 清对应平铺镜像字段(与原实现一致,
    /// 见 [`clear_surface_mirror`]——terminal_id / 宿主字段不动)。
    pub fn clear_surface_binding_if_matches(
        &mut self,
        session_id: &str,
        expected: &SurfaceBinding,
    ) -> bool {
        let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) else {
            return false;
        };
        if !s.surface.same_variant(expected) || s.surface.primary_key() != expected.primary_key() {
            return false;
        }
        let cleared = std::mem::replace(&mut s.surface, SurfaceBinding::None);
        clear_surface_mirror(s, &cleared);
        self.schedule_save();
        true
    }
}

/// 把 surface 反写到平铺镜像字段(W5 期间读侧仍读平铺)。
/// 逐条复刻原 set_* 的镜像写入:只写 variant 自己的字段 + 原互斥规则
/// (Herdr → 清 cmux 镜像;Orca → 清 otty 镜像),其余平铺字段**不动**
/// —— 尤其 herdr 与 orca 的共存必须保留(orca 是 herdr 宿主的外层锚点,
/// 见 commands/focus/orca_herdr.rs::apply_orca_binding_for_herdr_host_if_missing)。
fn sync_surface_mirror(s: &mut Session) {
    match &s.surface {
        SurfaceBinding::None => {}
        SurfaceBinding::Herdr {
            pane_id,
            workspace_id,
            socket_path,
        } => {
            s.herdr_pane_id = Some(pane_id.clone());
            s.herdr_workspace_id = workspace_id.clone();
            s.herdr_socket_path = socket_path.clone();
            // 互斥:herdr 与 cmux 不共存(agent 在 herdr 内层,外层由 herdr client 定位)。
            s.cmux_surface_id = None;
            s.cmux_workspace_id = None;
        }
        SurfaceBinding::Cmux {
            surface_id,
            workspace_id,
        } => {
            s.cmux_surface_id = Some(surface_id.clone());
            s.cmux_workspace_id = workspace_id.clone();
        }
        SurfaceBinding::Wezterm {
            pane_id,
            unix_socket,
            executable,
        } => {
            s.wezterm_pane_id = Some(pane_id.clone());
            s.wezterm_unix_socket = unix_socket.clone();
            s.wezterm_executable = executable.clone();
        }
        SurfaceBinding::Otty {
            pane_id,
            tab_id,
            window_id,
        } => {
            s.otty_pane_id = Some(pane_id.clone());
            // 原 set_otty_binding 的 tab_id 是必填 String:apply_otty_binding 传
            // 空串表示"缺 tab_id,待 capture 补全"。None → 空串,镜像形状不变。
            s.otty_tab_id = Some(tab_id.clone().unwrap_or_default());
            s.otty_window_id = window_id.clone();
        }
        SurfaceBinding::Tmux {
            pane_id,
            socket_path,
        } => {
            s.tmux_pane_id = Some(pane_id.clone());
            s.tmux_socket_path = socket_path.clone();
        }
        SurfaceBinding::Orca {
            pane_key,
            worktree_id,
            tab_id,
        } => {
            s.orca_pane_key = Some(pane_key.clone());
            s.orca_worktree_id = worktree_id.clone();
            s.orca_tab_id = tab_id.clone();
            // 互斥:Orca 宿主优先,清 Otty cwd-capture 误绑(原 set_orca_binding 行为)。
            s.otty_pane_id = None;
            s.otty_tab_id = None;
            s.otty_window_id = None;
        }
    }
}

/// 清 binding 时按被清 variant 清对应平铺镜像字段(逐条复刻原 clear_*_if_matches)。
fn clear_surface_mirror(s: &mut Session, cleared: &SurfaceBinding) {
    match cleared {
        SurfaceBinding::None => {}
        SurfaceBinding::Herdr { .. } => {
            s.herdr_pane_id = None;
            s.herdr_workspace_id = None;
            s.herdr_socket_path = None;
        }
        SurfaceBinding::Cmux { .. } => {
            s.cmux_surface_id = None;
            s.cmux_workspace_id = None;
        }
        SurfaceBinding::Wezterm { .. } => {
            s.wezterm_pane_id = None;
            s.wezterm_unix_socket = None;
            s.wezterm_executable = None;
        }
        SurfaceBinding::Otty { .. } => {
            s.otty_pane_id = None;
            s.otty_tab_id = None;
            s.otty_window_id = None;
        }
        SurfaceBinding::Tmux { .. } => {
            s.tmux_pane_id = None;
            s.tmux_socket_path = None;
        }
        SurfaceBinding::Orca { .. } => {
            s.orca_pane_key = None;
            s.orca_worktree_id = None;
            s.orca_tab_id = None;
        }
    }
}

fn normalized_tty(tty: Option<&str>) -> Option<&str> {
    tty.filter(|t| !t.is_empty())
}

/// 把已存在文件权限收紧到 0600(仅 unix;非 unix 无权限位概念,no-op)。
/// 复用思路跟 `license::state::tighten_file_perms_0600` / `settings::tighten_file_perms_0600`
/// 一致;为避免 sessions 反向依赖 license/settings 模块,各自独立维护一份。
/// 失败上抛,调用方 best-effort 静默。
#[cfg(unix)]
fn tighten_file_perms_0600(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn tighten_file_perms_0600(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "sessions/binding_tests.rs"]
mod binding_tests;
#[cfg(test)]
#[path = "sessions/proptest_tests.rs"]
mod proptest_tests;
#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
