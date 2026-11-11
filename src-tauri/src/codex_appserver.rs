//! Codex app-server 订阅：用 `thread/resume` + `thread/status/changed` 区分
//! 自动审批回环 Idle 与轮次真结束。
//!
//! Layer A (`codex_rollout`) 判「是不是自动审批轮」;本模块判「这轮 Idle 时 thread
//! 是否仍在 active」。两者正交:
//! - auto_review + thread `active`(实时) → 抑制 popup(自驱回环)
//! - auto_review + thread `idle`(仅 `status/changed` 推送) → 放行 popup(轮次真结束)
//! - auto_review + resume 快照里的 `idle`(陈旧) → 抑制,不误当结束
//! - auto_review + 状态未知(Idle) → 放行(见 rollout brief 保守降级,不误杀真结束)
//! - auto_review + resume 快照 idle → 抑制(陈旧,不可信)
//!
//! 关键:必须先对 thread 调 `thread/resume` 才会收到 push(见
//! `docs/briefs/20260715-codex-appserver-thread-status.md`)。

use crate::http_server::AppState;
use crate::sessions::{Session, SessionStatus};
use crate::{log_info, log_warn};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WATCH_INTERVAL: Duration = Duration::from_secs(5);
/// thread status 缓存 TTL;比轮询间隔长,避免短暂断连误判未知。
const CACHE_TTL: Duration = Duration::from_secs(30);
const RESTART_BACKOFF: Duration = Duration::from_secs(30);
/// 同一 thread 重复 `thread/resume` 的最小间隔(订阅掉了可重试)。
const RESUME_INTERVAL: Duration = Duration::from_secs(10);

/// app-server `ThreadStatus` 的运行时子集(只关心 popup 门控)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexThreadRuntimeStatus {
    Idle,
    Active {
        waiting_on_approval: bool,
        waiting_on_user_input: bool,
    },
}

/// thread 状态来源:resume 响应里的快照 vs `thread/status/changed` 推送。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatusProvenance {
    /// `thread/resume` 响应携带的瞬时快照,可能是上轮残留 idle,不可信为结束信号。
    ResumeSnapshot,
    /// `thread/status/changed` 实时推送。
    LiveNotification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexThreadStatusEntry {
    pub status: CodexThreadRuntimeStatus,
    pub provenance: ThreadStatusProvenance,
}

/// thread_id → 最新 thread status + 来源 + 更新时间。
pub struct CodexThreadStatusCache {
    entries: Mutex<
        std::collections::HashMap<
            String,
            (CodexThreadRuntimeStatus, ThreadStatusProvenance, Instant),
        >,
    >,
}

impl CodexThreadStatusCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn provenance_rank(provenance: ThreadStatusProvenance) -> u8 {
        match provenance {
            ThreadStatusProvenance::ResumeSnapshot => 0,
            ThreadStatusProvenance::LiveNotification => 1,
        }
    }

    /// 写入 thread status;状态或来源变化时打 INFO 便于排障。
    /// `LiveNotification` 不可被 `ResumeSnapshot` 降级覆盖。
    pub fn set(
        &self,
        thread_id: &str,
        status: CodexThreadRuntimeStatus,
        provenance: ThreadStatusProvenance,
    ) {
        let mut map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some((_, prev_prov, _)) = map.get(thread_id) {
            if Self::provenance_rank(provenance) < Self::provenance_rank(*prev_prov) {
                return;
            }
        }
        let changed = map
            .get(thread_id)
            .is_none_or(|(prev, prev_prov, _)| *prev != status || *prev_prov != provenance);
        map.insert(thread_id.to_string(), (status, provenance, Instant::now()));
        if changed {
            log_info!(
                "codex_appserver",
                "thread status thread_id={} status={:?} provenance={:?}",
                thread_id,
                status,
                provenance
            );
        }
    }

    pub fn clear(&self, thread_id: &str) {
        let mut map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.remove(thread_id);
    }

    /// 命中且未过期返回条目;否则 None(调用侧当未知)。
    pub fn get_entry(&self, thread_id: &str) -> Option<CodexThreadStatusEntry> {
        let now = Instant::now();
        let map = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        map.get(thread_id).and_then(|(status, provenance, ts)| {
            (now.duration_since(*ts) < CACHE_TTL).then_some(CodexThreadStatusEntry {
                status: *status,
                provenance: *provenance,
            })
        })
    }
}

impl Default for CodexThreadStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 向后台 app-server 桥发送 `thread/resume` 请求。测试默认构造为 no-op。
#[derive(Clone)]
pub struct CodexAppServerHandle {
    resume_tx: Sender<String>,
}

impl CodexAppServerHandle {
    pub fn spawn(cache: Arc<CodexThreadStatusCache>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || run_appserver_bridge(cache, rx));
        Self { resume_tx: tx }
    }

    pub fn request_resume(&self, thread_id: &str) {
        let _ = self.resume_tx.send(thread_id.to_string());
    }
}

impl Default for CodexAppServerHandle {
    fn default() -> Self {
        let (tx, _rx) = mpsc::channel();
        Self { resume_tx: tx }
    }
}

/// ChatGPT 桌面端 codex 会话:source=codex 且无 tty(与 surface / title 解析一致)。
pub fn is_codex_desktop_session(session: &Session) -> bool {
    session.source.as_deref() == Some("codex")
        && session
            .terminal_tty
            .as_deref()
            .map(|t| t.is_empty())
            .unwrap_or(true)
}

/// Codex 自动审批 popup 抑制判据。返回 Some(reason) = 应抑制;None = 放行。
pub fn codex_auto_popup_suppress_reason(
    session: &Session,
    auto_approved: bool,
    thread: Option<CodexThreadStatusEntry>,
) -> Option<&'static str> {
    if !is_codex_desktop_session(session) {
        return None;
    }
    if !auto_approved {
        return None;
    }

    // Pending:guardian 自审 PermissionRequest;resume 快照 idle 不可信。
    if session.status == SessionStatus::Pending {
        return match thread {
            Some(CodexThreadStatusEntry {
                status:
                    CodexThreadRuntimeStatus::Active {
                        waiting_on_user_input: true,
                        ..
                    },
                ..
            }) => None,
            Some(CodexThreadStatusEntry {
                status: CodexThreadRuntimeStatus::Active { .. },
                ..
            }) => Some("codex_thread_active"),
            _ => Some("codex_auto_approval"),
        };
    }

    match thread {
        Some(CodexThreadStatusEntry {
            status: CodexThreadRuntimeStatus::Active { .. },
            ..
        }) => Some("codex_thread_active"),
        Some(CodexThreadStatusEntry {
            status: CodexThreadRuntimeStatus::Idle,
            provenance: ThreadStatusProvenance::LiveNotification,
        }) => None,
        Some(CodexThreadStatusEntry {
            status: CodexThreadRuntimeStatus::Idle,
            provenance: ThreadStatusProvenance::ResumeSnapshot,
        }) => Some("codex_stale_thread_idle"),
        // thread 缓存未命中/过期:无正面证据证明仍在回环,不误杀真结束 popup。
        None => None,
    }
}

fn parse_thread_status(val: &Value) -> Option<CodexThreadRuntimeStatus> {
    let t = val.get("type")?.as_str()?;
    match t {
        "idle" => Some(CodexThreadRuntimeStatus::Idle),
        "active" => {
            let flags = val.get("activeFlags")?.as_array()?;
            let mut waiting_on_approval = false;
            let mut waiting_on_user_input = false;
            for f in flags {
                match f.as_str()? {
                    "waitingOnApproval" => waiting_on_approval = true,
                    "waitingOnUserInput" => waiting_on_user_input = true,
                    _ => {}
                }
            }
            Some(CodexThreadRuntimeStatus::Active {
                waiting_on_approval,
                waiting_on_user_input,
            })
        }
        _ => None,
    }
}

fn apply_status_notification(params: &Value, cache: &CodexThreadStatusCache) {
    let Some(thread_id) = params.get("threadId").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(status_val) = params.get("status") else {
        return;
    };
    let Some(parsed) = parse_thread_status(status_val) else {
        return;
    };
    cache.set(thread_id, parsed, ThreadStatusProvenance::LiveNotification);
}

fn apply_thread_object(thread: &Value, cache: &CodexThreadStatusCache) {
    let thread_id = thread
        .get("id")
        .or_else(|| thread.get("threadId"))
        .and_then(|v| v.as_str());
    let Some(thread_id) = thread_id else {
        return;
    };
    let Some(status_val) = thread.get("status") else {
        return;
    };
    let Some(parsed) = parse_thread_status(status_val) else {
        return;
    };
    cache.set(thread_id, parsed, ThreadStatusProvenance::ResumeSnapshot);
}

fn json_rpc_id_matches(val: &Value, expected: u64) -> bool {
    val.get("id")
        .and_then(|id| id.as_u64().or_else(|| id.as_str()?.parse().ok()))
        == Some(expected)
}

fn spawn_stderr_drain(stderr: impl std::io::Read + Send + 'static) -> Arc<Mutex<Vec<String>>> {
    let buf = Arc::new(Mutex::new(Vec::<String>::new()));
    let buf2 = buf.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(mut g) = buf2.lock() {
                    g.push(trimmed.to_string());
                    if g.len() > 20 {
                        let drop = g.len() - 20;
                        g.drain(0..drop);
                    }
                }
            }
        }
    });
    buf
}

fn handle_appserver_line(line: &str, cache: &CodexThreadStatusCache) {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if let Some(method) = val.get("method").and_then(|m| m.as_str()) {
        if method == "thread/status/changed" {
            if let Some(params) = val.get("params") {
                apply_status_notification(params, cache);
            }
        }
        return;
    }
    if let Some(result) = val.get("result") {
        if let Some(thread) = result.get("thread") {
            apply_thread_object(thread, cache);
        }
    }
}

fn spawn_codex_appserver_child() -> Option<Child> {
    // 裸 Command::new("codex") 只吃 daemon 极简 PATH（/usr/bin:/bin:...），
    // codex 装在 ~/.local/bin 等非标准路径时永远 spawn 失败 → 每 30s "codex missing?"
    // 噪音 + 桌面 title 静默失效。复用 agent_presence 的解析链（common paths → which
    // → login shell），与 is_agent_present 保持一致（2026-08-02 e2e 跑出来的真 bug）。
    // E2E 沙箱下不 spawn：sandbox .local/bin 里的是空壳 fake，spawn 了也即退，
    // 只留 is_agent_present 的检出职责给 resolve_cli_binary。
    if crate::agent_presence::sandbox_root_active() {
        return None;
    }
    let codex = crate::agent_presence::resolve_cli_binary("codex")?;
    // codex app-server 内部会 spawn `node`（`env: node: No such file or directory` 是
    // 2026-08-02 实装验证时发现的第二个坑）——子进程继承 daemon 极简 PATH，这里把常见
    // node 目录补进 PATH（/opt/homebrew/bin、/usr/local/bin、~/.local/bin、fnm node）。
    let mut path = std::env::var("PATH").unwrap_or_default();
    if let Ok(home) = std::env::var("HOME") {
        let extra = [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            &format!("{home}/.local/bin"),
            &format!("{home}/.local/share/fnm/node-versions/v24.14.1/installation/bin"),
        ];
        for dir in extra {
            if !path.split(':').any(|p| p == dir) {
                if !path.is_empty() {
                    path.push(':');
                }
                path.push_str(dir);
            }
        }
    }
    Command::new(codex)
        .arg("app-server")
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()
}

fn write_rpc_line(stdin: &mut impl Write, payload: &Value) -> bool {
    let line = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(_) => return false,
    };
    stdin.write_all(line.as_bytes()).is_ok()
        && stdin.write_all(b"\n").is_ok()
        && stdin.flush().is_ok()
}

fn send_thread_resume(stdin: &mut impl Write, next_id: &AtomicU64, thread_id: &str) -> bool {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "thread/resume",
        "params": { "threadId": thread_id }
    });
    write_rpc_line(stdin, &req)
}

fn run_appserver_session(cache: Arc<CodexThreadStatusCache>, resume_rx: &Receiver<String>) -> bool {
    let mut child = match spawn_codex_appserver_child() {
        Some(c) => c,
        None => {
            log_warn!(
                "codex_appserver",
                "spawn codex app-server failed (codex missing?)"
            );
            return false;
        }
    };
    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => return false,
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return false,
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => return false,
    };
    let stderr_buf = spawn_stderr_drain(stderr);

    let (line_tx, line_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        let _ = line_tx.send(trimmed);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // initialize 走「写后从 line_rx 等 id=1 响应」。
    if !write_rpc_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1u64,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "pokepoke",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        }),
    ) {
        let _ = child.kill();
        return false;
    }
    let mut init_ok = false;
    for _ in 0..50 {
        match line_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(line) => {
                if let Ok(val) = serde_json::from_str::<Value>(&line) {
                    if json_rpc_id_matches(&val, 1) {
                        init_ok = val.get("error").is_none();
                        break;
                    }
                    handle_appserver_line(&line, &cache);
                }
            }
            Err(_) => break,
        }
    }
    if !init_ok {
        let tail = stderr_buf.lock().map(|g| g.join(" | ")).unwrap_or_default();
        log_warn!(
            "codex_appserver",
            "initialize handshake failed stderr={}",
            tail
        );
        let _ = child.kill();
        return false;
    }
    if !write_rpc_line(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized" }),
    ) {
        let _ = child.kill();
        return false;
    }
    log_info!("codex_appserver", "app-server connected");

    let next_id = AtomicU64::new(2);
    let mut last_resume: std::collections::HashMap<String, Instant> =
        std::collections::HashMap::new();

    loop {
        let now = Instant::now();
        // 优先处理 resume 请求;失败或未订阅时按间隔重试。
        while let Ok(thread_id) = resume_rx.try_recv() {
            let due = last_resume
                .get(&thread_id)
                .map(|t| now.duration_since(*t) >= RESUME_INTERVAL)
                .unwrap_or(true);
            if due && send_thread_resume(&mut stdin, &next_id, &thread_id) {
                last_resume.insert(thread_id, now);
            }
        }

        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => handle_appserver_line(&line, &cache),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if child.try_wait().ok().flatten().is_some() {
            let tail = stderr_buf.lock().map(|g| g.join(" | ")).unwrap_or_default();
            log_info!("codex_appserver", "app-server child exited stderr={}", tail);
            break;
        }
    }
    let _ = child.kill();
    false
}

fn run_appserver_bridge(cache: Arc<CodexThreadStatusCache>, resume_rx: Receiver<String>) {
    loop {
        if !run_appserver_session(cache.clone(), &resume_rx) {
            log_info!(
                "codex_appserver",
                "app-server disconnected; retry in {:?}",
                RESTART_BACKOFF
            );
            std::thread::sleep(RESTART_BACKOFF);
        }
    }
}

fn eligible_session(session: &Session) -> bool {
    session
        .source
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("codex"))
        && session
            .external_session_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
}

/// 5s 扫描 codex session,对新 thread 发 resume。照 codex_rollout 模式。
pub fn scan_and_resume_threads(state: &AppState) {
    let sessions: Vec<Session> = {
        let store = state.store.lock().unwrap_or_else(|p| p.into_inner());
        store.get_all().to_vec()
    };
    for session in sessions.iter().filter(|s| eligible_session(s)) {
        if let Some(thread_id) = session.external_session_id.as_deref() {
            state.codex_appserver.request_resume(thread_id);
        }
    }
}

/// W3(brief 20260813-watcher-supervisor):迁移到 supervisor::spawn_periodic
/// (命名线程 + catch_unwind + 心跳),body 单轮逻辑零变化。
pub fn spawn_codex_thread_scan_watcher(state: AppState) {
    crate::supervisor::spawn_periodic("codex-thread-scan", WATCH_INTERVAL, move || {
        scan_and_resume_threads(&state);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionStatus;
    use chrono::Utc;

    fn codex_desktop_session(status: SessionStatus) -> Session {
        Session {
            id: "s1".into(),
            task_id: "t1".into(),
            external_session_id: Some("thread-uuid".into()),
            title: "t".into(),
            message: "m".into(),
            source: Some("codex".into()),
            priority: Default::default(),
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            terminal_tty: None,
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

    #[test]
    fn parse_idle_status() {
        let v = json!({"type": "idle"});
        assert_eq!(
            parse_thread_status(&v),
            Some(CodexThreadRuntimeStatus::Idle)
        );
    }

    #[test]
    fn parse_active_waiting_on_approval() {
        let v = json!({
            "type": "active",
            "activeFlags": ["waitingOnApproval"]
        });
        assert_eq!(
            parse_thread_status(&v),
            Some(CodexThreadRuntimeStatus::Active {
                waiting_on_approval: true,
                waiting_on_user_input: false,
            })
        );
    }

    fn entry(
        status: CodexThreadRuntimeStatus,
        provenance: ThreadStatusProvenance,
    ) -> CodexThreadStatusEntry {
        CodexThreadStatusEntry { status, provenance }
    }

    #[test]
    fn parse_status_changed_notification() {
        let cache = CodexThreadStatusCache::new();
        let params = json!({
            "threadId": "019f65d1-ae14-7121-9651-09a252d8f967",
            "status": { "type": "idle" }
        });
        apply_status_notification(&params, &cache);
        assert_eq!(
            cache.get_entry("019f65d1-ae14-7121-9651-09a252d8f967"),
            Some(entry(
                CodexThreadRuntimeStatus::Idle,
                ThreadStatusProvenance::LiveNotification
            ))
        );
    }

    #[test]
    fn suppress_active_auto_review_loop() {
        let session = codex_desktop_session(SessionStatus::Idle);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Active {
                        waiting_on_approval: true,
                        waiting_on_user_input: false,
                    },
                    ThreadStatusProvenance::LiveNotification,
                )),
            ),
            Some("codex_thread_active")
        );
    }

    #[test]
    fn allow_idle_auto_review_true_end() {
        let session = codex_desktop_session(SessionStatus::Idle);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Idle,
                    ThreadStatusProvenance::LiveNotification,
                )),
            ),
            None
        );
    }

    #[test]
    fn suppress_stale_resume_snapshot_idle() {
        let session = codex_desktop_session(SessionStatus::Idle);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Idle,
                    ThreadStatusProvenance::ResumeSnapshot,
                )),
            ),
            Some("codex_stale_thread_idle")
        );
    }

    #[test]
    fn idle_unknown_thread_status_allows_popup() {
        let session = codex_desktop_session(SessionStatus::Idle);
        assert_eq!(codex_auto_popup_suppress_reason(&session, true, None), None);
    }

    #[test]
    fn pending_suppressed_when_auto_and_thread_unknown() {
        let session = codex_desktop_session(SessionStatus::Pending);
        assert_eq!(
            codex_auto_popup_suppress_reason(&session, true, None),
            Some("codex_auto_approval")
        );
    }

    #[test]
    fn pending_suppressed_when_auto_and_stale_idle_snapshot() {
        let session = codex_desktop_session(SessionStatus::Pending);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Idle,
                    ThreadStatusProvenance::ResumeSnapshot,
                )),
            ),
            Some("codex_auto_approval")
        );
    }

    #[test]
    fn pending_shown_when_waiting_on_user_input() {
        let session = codex_desktop_session(SessionStatus::Pending);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Active {
                        waiting_on_approval: false,
                        waiting_on_user_input: true,
                    },
                    ThreadStatusProvenance::LiveNotification,
                )),
            ),
            None
        );
    }

    #[test]
    fn pending_suppressed_when_auto_and_thread_active() {
        let session = codex_desktop_session(SessionStatus::Pending);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                true,
                Some(entry(
                    CodexThreadRuntimeStatus::Active {
                        waiting_on_approval: true,
                        waiting_on_user_input: false,
                    },
                    ThreadStatusProvenance::LiveNotification,
                )),
            ),
            Some("codex_thread_active")
        );
    }

    #[test]
    fn live_notification_not_downgraded_by_resume_snapshot() {
        let cache = CodexThreadStatusCache::new();
        cache.set(
            "t1",
            CodexThreadRuntimeStatus::Idle,
            ThreadStatusProvenance::LiveNotification,
        );
        cache.set(
            "t1",
            CodexThreadRuntimeStatus::Idle,
            ThreadStatusProvenance::ResumeSnapshot,
        );
        assert_eq!(
            cache.get_entry("t1"),
            Some(entry(
                CodexThreadRuntimeStatus::Idle,
                ThreadStatusProvenance::LiveNotification
            ))
        );
    }

    #[test]
    fn resume_snapshot_upgraded_by_live_notification() {
        let cache = CodexThreadStatusCache::new();
        cache.set(
            "t1",
            CodexThreadRuntimeStatus::Idle,
            ThreadStatusProvenance::ResumeSnapshot,
        );
        cache.set(
            "t1",
            CodexThreadRuntimeStatus::Active {
                waiting_on_approval: true,
                waiting_on_user_input: false,
            },
            ThreadStatusProvenance::LiveNotification,
        );
        assert_eq!(
            cache.get_entry("t1"),
            Some(entry(
                CodexThreadRuntimeStatus::Active {
                    waiting_on_approval: true,
                    waiting_on_user_input: false,
                },
                ThreadStatusProvenance::LiveNotification
            ))
        );
    }

    #[test]
    fn non_auto_approved_never_suppressed() {
        let session = codex_desktop_session(SessionStatus::Idle);
        assert_eq!(
            codex_auto_popup_suppress_reason(
                &session,
                false,
                Some(entry(
                    CodexThreadRuntimeStatus::Active {
                        waiting_on_approval: true,
                        waiting_on_user_input: false,
                    },
                    ThreadStatusProvenance::LiveNotification,
                )),
            ),
            None
        );
    }
}
