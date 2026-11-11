//! Layer 1 — `/notify` handler 的端到端 integration 测试。
//!
//! 通过 `process_notify` 直接驱动 handler，完全绕开 axum / TCP；
//! popup / sound / focus / events 全部用 spy 实现注入，断言副作用
//! 调用次数与参数。覆盖：状态机、popup show/close 决策、focus 抑制、
//! 跨 source 的 task_id 隔离。
//!
//! 见 `docs/briefs/20260420-layer1-http-integration-tests.md`。

use poke_poke_lib::http_server::{
    process_notify, AppState, EventEmitter, NotificationGate, NotifyRequest, SessionMemory,
    StagingDispatcher,
};
use poke_poke_lib::popup::{FocusOracle, PopupController};
use poke_poke_lib::sessions::{Session, SessionStatus, SessionStore};
use poke_poke_lib::sound::SoundPlayer;
use poke_poke_lib::transcript_watch::{scan_once, TranscriptInterruptionState};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ==================== Spy 实现 ====================

#[derive(Default)]
struct SpyPopup {
    shown: Mutex<Vec<Session>>,
    /// 当前"活跃"popup 集合,模拟生产 popup_list — show 时插入、close 时移除。
    /// is_first 判定看这个,不动 shown 累计历史(后者用来断言 show 总次数)。
    active: Mutex<std::collections::HashSet<String>>,
    closed: Mutex<Vec<String>>,
    // 2026-08-05 kimi 评审线路(brief 20260805-pending-idle-completed-sound-swallowed):
    // 播音从 popup 创建解耦到 http_server::play_notify_sound — spy 不再模拟内部播音,
    // 声音断言全部走 state.sound(SpySound)由 helper 侧触发。
}

impl SpyPopup {
    fn show_count(&self) -> usize {
        self.shown.lock().unwrap().len()
    }
    fn close_count(&self) -> usize {
        self.closed.lock().unwrap().len()
    }
    fn last_shown_id(&self) -> Option<String> {
        self.shown.lock().unwrap().last().map(|s| s.id.clone())
    }
    fn last_closed_id(&self) -> Option<String> {
        self.closed.lock().unwrap().last().cloned()
    }
}

impl PopupController for SpyPopup {
    fn show(&self, session: &Session) -> bool {
        // is_first 看 active 集合(insert 返回 true 表示之前没有);
        // shown 累计历史不动,保留供 show_count 断言。
        let is_first = self.active.lock().unwrap().insert(session.id.clone());
        self.shown.lock().unwrap().push(session.clone());
        is_first
    }
    fn close(&self, session_id: &str, _reason: poke_poke_lib::popup::PopupCloseReason) {
        self.closed.lock().unwrap().push(session_id.to_string());
        self.active.lock().unwrap().remove(session_id);
    }
    fn has_session_popup(&self, session_id: &str) -> bool {
        self.active.lock().unwrap().contains(session_id)
    }
}

#[derive(Default)]
struct SpySound {
    played: Mutex<u32>,
}

impl SpySound {
    fn play_count(&self) -> u32 {
        *self.played.lock().unwrap()
    }
}

impl SoundPlayer for SpySound {
    fn play_alert(&self, _status: poke_poke_lib::sessions::SessionStatus) {
        *self.played.lock().unwrap() += 1;
    }
}

/// FocusOracle spy：默认所有 tty 都视为"未聚焦"；
/// 可通过 `set_focused_ttys` 强制把某些 tty 标记为聚焦。
#[derive(Default)]
struct SpyFocus {
    focused_ttys: Mutex<Vec<String>>,
    calls: Mutex<Vec<poke_poke_lib::surface::SurfaceEvidence>>,
}

impl SpyFocus {
    fn set_focused(&self, ttys: &[&str]) {
        let mut g = self.focused_ttys.lock().unwrap();
        g.clear();
        g.extend(ttys.iter().map(|s| s.to_string()));
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn last_call_tty(&self) -> Option<String> {
        self.calls
            .lock()
            .unwrap()
            .last()
            .and_then(|e| e.terminal_tty.clone())
    }
}

impl FocusOracle for SpyFocus {
    fn is_focused(&self, evidence: &poke_poke_lib::surface::SurfaceEvidence) -> bool {
        self.calls.lock().unwrap().push(evidence.clone());
        // 集成测试只覆盖 iTerm2/Terminal 路径(按 tty 匹配);Ghostty 路径
        // 走 terminal_id 由 build_focus_check_script 单测覆盖,这里不模拟。
        match evidence.terminal_tty.as_deref() {
            Some(t) if !t.is_empty() => self.focused_ttys.lock().unwrap().iter().any(|x| x == t),
            _ => false,
        }
    }
}

#[derive(Default)]
struct SpyEvents {
    emitted: Mutex<u32>,
}

impl SpyEvents {
    fn count(&self) -> u32 {
        *self.emitted.lock().unwrap()
    }
}

impl EventEmitter for SpyEvents {
    fn sessions_updated(&self) {
        *self.emitted.lock().unwrap() += 1;
    }
}

/// NotificationGate spy：默认"允许提示"，可通过 set_allow(false) 模拟
/// 用户关闭了全局提示开关，用来验证 popup / sound 的短路分支。
struct SpyGate {
    allow: Mutex<bool>,
}

impl Default for SpyGate {
    fn default() -> Self {
        Self {
            allow: Mutex::new(true),
        }
    }
}

impl SpyGate {
    fn set_allow(&self, allow: bool) {
        *self.allow.lock().unwrap() = allow;
    }
}

impl NotificationGate for SpyGate {
    fn allow_notifications(&self) -> bool {
        *self.allow.lock().unwrap()
    }
}

// ==================== Test fixture ====================

struct Fixture {
    state: AppState,
    popup: Arc<SpyPopup>,
    sound: Arc<SpySound>,
    focus: Arc<SpyFocus>,
    events: Arc<SpyEvents>,
    gate: Arc<SpyGate>,
    memory: Arc<SessionMemory>,
    _tmp: TempDir,
}

fn fixture() -> Fixture {
    fixture_with_cooldowns(std::time::Duration::ZERO, std::time::Duration::ZERO)
}

/// 带 popup/sound cooldown 参数的 fixture:默认 0(实质关闭,不挡快速连续
/// transition,让现有 transition 单测断言保持原意)。
/// 双发 stop 只播一次音的测试需要真实 sound 3s 窗口,
/// 用 `fixture_with_cooldowns(Duration::ZERO, Duration::from_secs(3))`。
/// Pending→Idle 完成音回归(本 brief)需要真实 popup 3s 窗口,
/// 用 `fixture_with_cooldowns(Duration::from_secs(3), Duration::ZERO)`。
fn fixture_with_cooldowns(
    popup_cooldown_window: std::time::Duration,
    sound_cooldown_window: std::time::Duration,
) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(Mutex::new(SessionStore::load(
        tmp.path().join("sessions.json"),
    )));
    let popup = Arc::new(SpyPopup::default());
    let sound = Arc::new(SpySound::default());
    let focus = Arc::new(SpyFocus::default());
    let events = Arc::new(SpyEvents::default());
    let gate = Arc::new(SpyGate::default());
    let state = AppState {
        store: store.clone(),
        popup: popup.clone(),
        sound: sound.clone(),
        focus: focus.clone(),
        events: events.clone(),
        gate: gate.clone(),
        // W8(brief 20260813-session-memory):五窗口合一 —— 测试默认 cooldown=0
        // (实质关闭,不挡快速连续 transition,让现有 transition 单测断言保持
        // 原意);窗口行为单测在 state.rs / tests_state.rs 独立覆盖,双发只播
        // 一次音 / Pending→Idle 完成音的集成断言用真实窗口。
        memory: Arc::new(SessionMemory::new(
            std::time::Duration::ZERO, // recently_visited 默认 0,不影响 transition 单测
            popup_cooldown_window,
            sound_cooldown_window,
        )),
        // AutoApprovalCache 默认空 -> 查询返回 false(放行),不影响现有 transition
        // 单测;自动审批抑制行为在 codex_rollout.rs 内单测覆盖。
        auto_approval: Arc::new(poke_poke_lib::codex_rollout::AutoApprovalCache::default()),
        codex_thread_status: Arc::new(
            poke_poke_lib::codex_appserver::CodexThreadStatusCache::default(),
        ),
        codex_appserver: poke_poke_lib::codex_appserver::CodexAppServerHandle::default(),
        settings: Arc::new(Mutex::new(poke_poke_lib::settings::SettingsStore::load(
            tmp.path().join("settings.json"),
        ))),
        quiet_mode: None,
        // 集成测试用同步 staging:断言可在 process_notify 返回后立即检查副作用。
        staging: StagingDispatcher::inline(),
    };
    let memory = state.memory.clone();
    Fixture {
        state,
        popup,
        sound,
        focus,
        events,
        gate,
        memory,
        _tmp: tmp,
    }
}

fn req(task_id: &str, status: &str, source: &str) -> NotifyRequest {
    NotifyRequest {
        protocol_version: None,
        title: format!("{}: project", source),
        message: "msg".into(),
        task_id: task_id.into(),
        external_session_id: Some(format!("ext-{}", task_id)),
        source: Some(source.into()),
        priority: None,
        event_type: None,
        status: Some(status.into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
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

fn req_with_tty(task_id: &str, status: &str, source: &str, tty: &str) -> NotifyRequest {
    let mut r = req(task_id, status, source);
    r.terminal_tty = Some(tty.into());
    r
}

fn req_session_end(task_id: &str) -> NotifyRequest {
    NotifyRequest {
        protocol_version: None,
        title: "Cursor: project".into(),
        message: "Session ended".into(),
        task_id: task_id.into(),
        external_session_id: None,
        source: Some("cursor".into()),
        priority: None,
        event_type: Some("session_end".into()),
        status: Some("idle".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
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

fn store_snapshot(state: &AppState) -> Vec<Session> {
    state.store.lock().unwrap().get_all().to_vec()
}

// ==================== Scenarios ====================

/// pokehook 上报的 transcript_path 经 /notify → Session 持久化,供 transcript watcher 使用。
/// 用此字段定位。同 session_id 多次事件:Some 覆盖、None 保留旧值(防止后续事件
/// 漏带把已知路径清掉)。
#[test]
fn notify_persists_transcript_path_on_session_and_preserves_when_omitted() {
    let f = fixture();
    let mut r = req("cc-tp-1", "running", "claude-code");
    r.transcript_path = Some("/tmp/cc-fake.jsonl".into());
    process_notify(&f.state, r);
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(s.transcript_path.as_deref(), Some("/tmp/cc-fake.jsonl"));

    // 后续 idle 事件 transcript_path=None → 保留前值
    process_notify(&f.state, req("cc-tp-1", "idle", "claude-code"));
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(
        s.transcript_path.as_deref(),
        Some("/tmp/cc-fake.jsonl"),
        "后续不带 transcript_path 的事件不该把旧值清空"
    );
}

/// brief 20260527-ghostty-capture-retry:retry 路径的回归保护 ——
/// 多个事件(SessionStart → UserPromptSubmit → Stop)挨个 fire 不应:
/// (a) crash
/// (b) 因为 retry 触发把已有 terminal_id 覆写(幂等)
/// (c) 因为 retry 触发把 None 改成"假"id(测试环境无 Ghostty,capture 会失败 → 保持 None)
///
/// 真实"capture 命中率从 72% → 95%" 验证靠真机 smoke,无法在 CI 复现
/// (依赖 macOS Ghostty + osascript)。本测仅守住"代码加了 retry 不破坏老 session
/// 状态"这层底线。
#[test]
fn ghostty_capture_retry_path_does_not_break_session_state() {
    let f = fixture();

    // SessionStart with ghostty hint — capture 会 spawn 但测试环境无 Ghostty → 不写
    let mut r1 = req("cc-ghostty-1", "running", "claude-code");
    r1.terminal_app = Some("ghostty".into());
    process_notify(&f.state, r1);
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(
        s.terminal_id, None,
        "测试环境无 Ghostty,SessionStart 不应写出假 id"
    );

    // 直接灌一个 terminal_id 模拟 capture 成功
    {
        let mut store = f.state.store.lock().unwrap();
        store.set_terminal_id_if_unset("cc-ghostty-1", "fake-term-uuid".into(), "/tmp/proj".into());
    }
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(s.terminal_id.as_deref(), Some("fake-term-uuid"));
    assert_eq!(s.terminal_id_captured_cwd.as_deref(), Some("/tmp/proj"));

    // 后续 UserPromptSubmit 等价事件(event_type=running,is_new=false)→
    // 触发 retry 路径,但 terminal_id 已是 Some → spawn 内部的 set_terminal_id_if_unset
    // 是幂等的,即便 capture 真跑了也不会覆盖。这里没真 Ghostty 也不会出问题。
    let mut r2 = req("cc-ghostty-1", "running", "claude-code");
    r2.terminal_app = Some("ghostty".into());
    r2.message = "Working...\n/tmp/proj".into(); // 让 signature 跟首次不同避免 dedupe
    process_notify(&f.state, r2);
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(
        s.terminal_id.as_deref(),
        Some("fake-term-uuid"),
        "retry 路径不应覆盖已捕获的 terminal_id"
    );

    // 后续 stop 同款
    let mut r3 = req("cc-ghostty-1", "idle", "claude-code");
    r3.terminal_app = Some("ghostty".into());
    process_notify(&f.state, r3);
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(s.terminal_id.as_deref(), Some("fake-term-uuid"));
}

/// CC 完整一轮：SessionStart → UserPromptSubmit → Stop。
/// Stop（Idle）应触发 popup 一次 + sound 一次；其它两步不弹。
///
/// 注:第二次 `running` 在 100ms 内会被 server 端去重(§3.2),所以只有 2 次
/// upsert 命中 store(running + idle),events.count() == 2。
#[test]
fn cc_full_round_pops_only_on_stop() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "running", "claude-code")); // 100ms 内同 signature → 被去重
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));

    assert_eq!(
        f.popup.show_count(),
        1,
        "popup should fire only on stage-end"
    );
    assert_eq!(f.sound.play_count(), 1, "sound should match popup");
    assert_eq!(f.popup.close_count(), 0, "no close in this flow");
    assert_eq!(
        f.events.count(),
        2,
        "dedupe: 第二次 running 被吃掉,只剩 running + idle 两次 upsert"
    );

    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].status, SessionStatus::Idle);
}

/// Pending(审批中) → Stop:状态被覆盖为 Idle,并弹"完成" popup。
///
/// 这是有意行为:CC 卡在权限审批时 turn 不会结束,Stop 只在审批解决后
/// (批准 → turn 完成 / 拒绝 → turn 结束)到达,此时 Pending→Idle 是正确
/// 的完成信号。payload 无法区分"审批未决时被 Esc 中断"的异常流,该场景
/// 记 followup(docs/briefs/20260801-kimi-review-round2.md)。
#[test]
fn stop_overwrites_pending_to_idle_and_pops() {
    let f = fixture();

    process_notify(&f.state, req("pend-1", "pending", "claude-code"));
    process_notify(&f.state, req("pend-1", "idle", "claude-code"));

    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].status, SessionStatus::Idle);
    assert_eq!(f.popup.show_count(), 2, "pending + idle 各弹一次");
}

/// 2026-08-05 用户反馈回归(brief 20260805-pending-idle-completed-sound-swallowed):
/// solutionhub4 的 CC 会话 Pending 弹窗播了 waiting 音,2s 后 Idle(完成)被 3s
/// popup cooldown 拦成 SuppressSilent → 完成音被连带吞掉,用户只听到等待音。
/// 修复后:cooldown 抑制的 stage-ending 事件降级"只播音不弹窗",completed 音仍要响。
#[test]
fn pending_then_idle_within_popup_cooldown_still_plays_completed_sound() {
    // popup cooldown 用真实 3s(模拟 Pending 弹窗消耗槽位后 2s 内 Idle 到达);
    // sound cooldown 用 0 隔离(本测试验 popup 窗口侧的降级播音,不验声音节流)。
    let f = fixture_with_cooldowns(std::time::Duration::from_secs(3), std::time::Duration::ZERO);

    process_notify(&f.state, req("pend-1", "running", "claude-code"));
    process_notify(&f.state, req("pend-1", "pending", "claude-code")); // 弹窗 + waiting 音,消耗 popup 槽位
    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.sound.play_count(), 1, "Pending 播 waiting 音");

    // 3s 内 Idle(完成):popup 已在屏 + popup 槽位被占 → SuppressSilent(cooldown)
    // 降级路径 —— 弹窗不重建,但 completed 音照播。
    process_notify(&f.state, req("pend-1", "idle", "claude-code"));

    assert_eq!(f.popup.show_count(), 1, "popup 不重建(refresh-only 语义)");
    assert_eq!(f.sound.play_count(), 2, "完成音不被 cooldown 吞掉");
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Idle);
}

/// 聚焦路径(SuppressWithSound)对称回归:Pending(waiting)音**不消耗** SoundCooldown
/// 槽位 → 3s 内到达的 Idle(完成)仍能播 completed 音(修复前 waiting 音把槽位占了,
/// 完成音被 sound_cooldown 吞掉——与失焦路径同构的用户反馈)。
#[test]
fn focused_pending_then_idle_plays_waiting_and_completed() {
    let f = fixture_with_cooldowns(std::time::Duration::ZERO, std::time::Duration::from_secs(3));
    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("pend-1", "running", "claude-code", "/dev/ttys001"),
    );
    process_notify(
        &f.state,
        req_with_tty("pend-1", "pending", "claude-code", "/dev/ttys001"),
    );
    assert_eq!(
        f.sound.play_count(),
        1,
        "Pending 聚焦播 waiting 音(不消耗槽位)"
    );

    process_notify(
        &f.state,
        req_with_tty("pend-1", "idle", "claude-code", "/dev/ttys001"),
    );
    assert_eq!(f.sound.play_count(), 2, "3s 内 Idle 仍播 completed 音");
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Idle);
}

/// §3.2: 同 task_id + 同 signature 在 100ms 内的重复 /notify 应被 server
/// 短路掉(返回 deduped: true);store / popup / sound / events 都不动。
#[test]
fn dedupe_short_circuits_duplicate_within_window() {
    let f = fixture();

    let (code1, body1) = process_notify(&f.state, req("dup-1", "idle", "claude-code"));
    let (code2, body2) = process_notify(&f.state, req("dup-1", "idle", "claude-code"));

    assert_eq!(code1.as_u16(), 201, "首次应 CREATED");
    assert!(body1.get("deduped").is_none(), "首次不带 deduped 字段");
    assert_eq!(code2.as_u16(), 200, "重复应 OK");
    assert_eq!(body2["deduped"], serde_json::json!(true));

    assert_eq!(f.popup.show_count(), 1, "popup 只 show 一次");
    assert_eq!(f.sound.play_count(), 1, "sound 只播一次");
    assert_eq!(f.events.count(), 1, "events 只 emit 一次");
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 1, "store 只 upsert 一次");
}

/// §6.2 主链路 e2e:dedupe + 状态机 + popup show/close 综合场景。
/// 模拟 Cursor 开 third-party hooks 后 CC + Cursor 双触发 + 用户多轮交互,
/// 验证所有副作用计数都正确。
#[test]
fn dedupe_plus_state_machine_full_cycle() {
    let f = fixture();

    // 第 1 轮:Running 开始 + 100ms 内重复(dedupe) + Stop(Idle 弹 popup)
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "running", "claude-code")); // dedup
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));

    assert_eq!(f.popup.show_count(), 1, "第 1 轮 Idle 弹 popup");
    assert_eq!(f.sound.play_count(), 1);
    assert_eq!(f.popup.close_count(), 0);

    // 第 2 轮:Running 重启关 popup + Pending 弹新 popup + 重复(dedup)
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code")); // dedup

    assert_eq!(f.popup.close_count(), 1, "Idle→Running 关 popup");
    assert_eq!(f.popup.show_count(), 2, "Running→Pending 弹新 popup");
    assert_eq!(f.sound.play_count(), 2);

    // 第 3 轮:Pending → Running(用户回应) → LastFailed(API 错误)
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "last_failed", "claude-code"));

    assert_eq!(f.popup.close_count(), 2, "Pending→Running 关 popup");
    assert_eq!(f.popup.show_count(), 3, "Running→LastFailed 弹 popup");
    assert_eq!(f.sound.play_count(), 3);

    // store 只有 1 个 session,最终状态 LastFailed
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].status, SessionStatus::LastFailed);
}

/// 用户开始新一轮:Stop(Idle) → UserPromptSubmit(Running)。
/// Running 应触发 popup close。
#[test]
fn restart_after_stop_closes_popup() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));
    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.popup.close_count(), 0);

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    assert_eq!(f.popup.close_count(), 1, "Idle→Running should close popup");
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Running);
}

/// CC Notification（Pending）：从 Running 切到 Pending 应弹 popup + 响。
#[test]
fn cc_notification_triggers_pending_popup() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));

    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.sound.play_count(), 1);
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Pending);
}

/// CC StopFailure：状态进入 LastFailed 并保留 failure_reason。
#[test]
fn cc_stop_failure_records_reason_and_pops() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    let mut failed = req("cc-1", "last_failed", "claude-code");
    failed.failure_reason = Some("rate_limit".into());
    process_notify(&f.state, failed);

    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.sound.play_count(), 1);
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::LastFailed);
    assert_eq!(sessions[0].failure_reason.as_deref(), Some("rate_limit"));
    assert_eq!(
        f.popup.last_shown_id().as_deref(),
        Some(sessions[0].id.as_str()),
    );
}

/// Cursor SessionEnd：把 session 从 store 移除，并关掉关联 popup。
#[test]
fn cursor_session_end_removes_session_and_closes_popup() {
    let f = fixture();

    process_notify(&f.state, req("cursor-1", "running", "cursor"));
    process_notify(&f.state, req("cursor-1", "idle", "cursor"));
    assert_eq!(store_snapshot(&f.state).len(), 1);
    let session_id = store_snapshot(&f.state)[0].id.clone();

    process_notify(&f.state, req_session_end("cursor-1"));

    assert!(
        store_snapshot(&f.state).is_empty(),
        "session should be gone"
    );
    assert_eq!(
        f.popup.last_closed_id().as_deref(),
        Some(session_id.as_str()),
        "the right popup should be closed",
    );
}

/// 聚焦抑制(focus_suppression_enabled 默认 true)：当 FocusOracle 报告
/// tty 已聚焦时,popup 被抑制但提示音仍然播放。
#[test]
fn focus_suppresses_popup_but_plays_sound() {
    let f = fixture();

    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cc-1", "running", "claude-code", "/dev/ttys001"),
    );
    process_notify(
        &f.state,
        req_with_tty("cc-1", "pending", "claude-code", "/dev/ttys001"),
    );

    assert_eq!(f.popup.show_count(), 0, "focus should suppress popup");
    assert_eq!(
        f.sound.play_count(),
        1,
        "focus should still play alert sound"
    );
    assert_eq!(
        f.focus.call_count(),
        1,
        "showable transition should consult focus oracle once"
    );
    assert_eq!(
        f.focus.last_call_tty().as_deref(),
        Some("/dev/ttys001"),
        "focus oracle should receive SurfaceEvidence carrying the request tty",
    );

    // store 仍然要正常更新——抑制的只是 popup 副作用
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Pending);
}

/// cursor CLI Ctrl+C 双发 stop|idle + stop|last_failed(同 task,~40ms 间隔,
/// 见 brief 20260805):聚焦终端时两条都走 SuppressWithSound,之前各播一次音
/// (completed + failed 同时响)。修复后同 task_id 3s 内只播第一次。
#[test]
fn double_stage_end_same_task_plays_sound_once_when_focused() {
    let f = fixture_with_cooldowns(std::time::Duration::ZERO, std::time::Duration::from_secs(3));
    f.focus.set_focused(&["/dev/ttys001"]);

    // 事件 1:aborted → stop|idle
    let mut r1 = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r1.event_type = Some("stop".into());
    process_notify(&f.state, r1);

    // 事件 2:hookStatus 缺失/error → stop|last_failed
    // (去重 signature=event_type|status,与 stop|idle 不同,不会被 100ms 去重吞)
    let mut r2 = req_with_tty("cursor-abc", "last_failed", "cursor", "/dev/ttys001");
    r2.event_type = Some("stop".into());
    process_notify(&f.state, r2);

    assert_eq!(f.sound.play_count(), 1, "同 turn-end 双发只播一次音");
}

/// 单次正常 stop(idle)聚焦时仍照常播音(非回归)。
#[test]
fn single_stage_end_plays_sound_when_focused() {
    let f = fixture_with_cooldowns(std::time::Duration::ZERO, std::time::Duration::from_secs(3));
    f.focus.set_focused(&["/dev/ttys001"]);

    let mut r = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r.event_type = Some("stop".into());
    process_notify(&f.state, r);

    assert_eq!(f.sound.play_count(), 1, "单次 stage-end 照常播音");
}

/// 不同 task 互不干扰:两个 task 各自的 stage-end 都播音(不把真任务完成音吞掉)。
#[test]
fn double_stage_end_different_task_plays_sound_twice() {
    let f = fixture_with_cooldowns(std::time::Duration::ZERO, std::time::Duration::from_secs(3));
    f.focus.set_focused(&["/dev/ttys001"]);

    let mut r1 = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r1.event_type = Some("stop".into());
    process_notify(&f.state, r1);
    let mut r2 = req_with_tty("cursor-def", "idle", "cursor", "/dev/ttys001");
    r2.event_type = Some("stop".into());
    process_notify(&f.state, r2);

    assert_eq!(f.sound.play_count(), 2, "不同 task 各自播音");
}

/// T1(brief 20260805-t1-cursor-aborted):cursor Ctrl+C 取消 —— stop|idle(aborted)
/// 聚焦终端时不播"完成"音(之前先到者播音,cancel 却响 completed)。
#[test]
fn cursor_aborted_focused_plays_no_sound() {
    let f = fixture();
    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cursor-abc", "running", "cursor", "/dev/ttys001"),
    );
    let mut r = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r.event_type = Some("stop".into());
    r.aborted = Some(true);
    process_notify(&f.state, r);

    assert_eq!(f.sound.play_count(), 0, "aborted 取消不播'完成'音");
    let s = &store_snapshot(&f.state)[0];
    assert_eq!(s.status, SessionStatus::Idle);
    assert!(s.aborted_at.is_some(), "aborted 事件应写 aborted_at");
}

/// T1:aborted 后 ~40ms 的 stop|last_failed(hookStatus 缺失/error)是同一轮取消的
/// artifact echo:状态保持 idle、不写 failure_reason、不播音。
/// 用默认 fixture(sound_cooldown=0)隔离验证——不是靠 SoundCooldown 兜住,
/// 而是 echo 在 upsert 层被抑制 → prev=idle → popup gate 自然 Skip。
#[test]
fn cursor_aborted_echo_last_failed_keeps_idle_and_silent() {
    let f = fixture();
    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cursor-abc", "running", "cursor", "/dev/ttys001"),
    );
    let mut r1 = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r1.event_type = Some("stop".into());
    r1.aborted = Some(true);
    process_notify(&f.state, r1);

    let mut r2 = req_with_tty("cursor-abc", "last_failed", "cursor", "/dev/ttys001");
    r2.event_type = Some("stop".into());
    r2.failure_reason = Some("api_error".into());
    process_notify(&f.state, r2);

    let s = &store_snapshot(&f.state)[0];
    assert_eq!(
        s.status,
        SessionStatus::Idle,
        "echo 不应把 idle 覆盖成 last_failed"
    );
    assert_eq!(s.failure_reason, None, "echo 不写 failure_reason");
    assert!(s.aborted_at.is_some());
    assert_eq!(f.sound.play_count(), 0, "aborted + echo 全程不播音");
}

/// T1:反向顺序 —— last_failed 先到(真错误,照常失败音),aborted 后到 → 最终 idle。
#[test]
fn cursor_last_failed_then_aborted_ends_idle() {
    let f = fixture();
    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cursor-abc", "running", "cursor", "/dev/ttys001"),
    );
    let mut r1 = req_with_tty("cursor-abc", "last_failed", "cursor", "/dev/ttys001");
    r1.event_type = Some("stop".into());
    r1.failure_reason = Some("api_error".into());
    process_notify(&f.state, r1);
    // 第一条是真失败 → 聚焦播音照常(非回归)
    assert_eq!(f.sound.play_count(), 1, "last_failed 先到应播失败音");

    let mut r2 = req_with_tty("cursor-abc", "idle", "cursor", "/dev/ttys001");
    r2.event_type = Some("stop".into());
    r2.aborted = Some(true);
    process_notify(&f.state, r2);

    let s = &store_snapshot(&f.state)[0];
    assert_eq!(s.status, SessionStatus::Idle, "反向顺序最终应 idle");
    assert_eq!(s.failure_reason, None);
    assert!(s.aborted_at.is_some());
    // aborted 事件本身也抑制播音 → 总数仍 1
    assert_eq!(f.sound.play_count(), 1);
}

/// T1 非回归:正常 error(无 aborted)仍 last_failed + 失败音(对照
/// focus_suppresses_popup_but_plays_sound,只是 transition 换成 Running→LastFailed)。
#[test]
fn cursor_error_without_aborted_still_last_failed_with_sound() {
    let f = fixture();
    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cursor-abc", "running", "cursor", "/dev/ttys001"),
    );
    let mut r = req_with_tty("cursor-abc", "last_failed", "cursor", "/dev/ttys001");
    r.event_type = Some("stop".into());
    r.failure_reason = Some("api_error".into());
    process_notify(&f.state, r);

    let s = &store_snapshot(&f.state)[0];
    assert_eq!(
        s.status,
        SessionStatus::LastFailed,
        "无 aborted 的真错误应落 last_failed"
    );
    assert_eq!(s.failure_reason.as_deref(), Some("api_error"));
    assert_eq!(f.sound.play_count(), 1, "真错误仍播失败音");
}

/// 焦点抑制只对匹配的 tty 生效。
#[test]
fn focus_does_not_suppress_other_ttys() {
    let f = fixture();

    f.focus.set_focused(&["/dev/ttys001"]);

    process_notify(
        &f.state,
        req_with_tty("cc-1", "running", "claude-code", "/dev/ttys999"),
    );
    process_notify(
        &f.state,
        req_with_tty("cc-1", "pending", "claude-code", "/dev/ttys999"),
    );

    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.sound.play_count(), 1);
}

/// 多 session 交错：CC + Cursor + Codex 三条 task_id，
/// 各自独立维护状态机，store 内有三条，互不影响。
#[test]
fn multi_session_interleaving_keeps_task_ids_isolated() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cursor-1", "running", "cursor"));
    process_notify(&f.state, req("codex-1", "running", "codex"));

    process_notify(&f.state, req("cursor-1", "idle", "cursor"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    process_notify(&f.state, req("codex-1", "idle", "codex"));

    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 3);
    let by_task: HashMap<_, _> = sessions
        .iter()
        .map(|s| (s.task_id.as_str(), s.status.clone()))
        .collect();
    assert_eq!(by_task["cc-1"], SessionStatus::Pending);
    assert_eq!(by_task["cursor-1"], SessionStatus::Idle);
    assert_eq!(by_task["codex-1"], SessionStatus::Idle);

    // 三次 stage-end 转移（cursor idle、cc pending、codex idle）都应弹
    assert_eq!(f.popup.show_count(), 3);
    assert_eq!(f.sound.play_count(), 3);
}

/// 全局提示开关关闭：阶段结束的转移完全不应触发 popup / sound，
/// 但 store 状态更新、sessions-updated 事件仍要照常发送。
#[test]
fn notifications_disabled_suppresses_popup_and_sound() {
    let f = fixture();
    f.gate.set_allow(false);

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));

    assert_eq!(f.popup.show_count(), 0, "gate off should suppress popup");
    assert_eq!(f.sound.play_count(), 0, "gate off should suppress sound");

    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].status, SessionStatus::Pending);
    assert_eq!(f.events.count(), 4, "store updates still emit events");
}

/// 全局提示开关关闭时，残留 popup 的 close 逻辑不受影响——
/// 先在开关打开状态弹出 popup，再关闭开关，触发 Idle→Running 转移应该正常 close。
#[test]
fn notifications_disabled_still_closes_existing_popup() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));
    assert_eq!(f.popup.show_count(), 1);

    f.gate.set_allow(false);
    process_notify(&f.state, req("cc-1", "running", "claude-code"));

    assert_eq!(
        f.popup.close_count(),
        1,
        "gate off must not block close for state consistency",
    );
}

/// AckedStages: 用户点过 popup 之后,同 session 同 status 重弹被抑制,
/// 直到 session 至少经过一次 Running 才允许该 status 再弹。
///
/// **Pending 例外(2026-06-25 修订,见 b3fffe4):** Pending 不受 AckedStages 抑制
/// —— 用户点过 permission popup 只是"去看了一眼",授权未必给、agent 仍卡在 Pending
/// 等输入。若 mark acked 后永久抑制直到 Running,用户错过首次后 agent 会无声卡死。
/// 所以 Pending 的频率仅由 PopupCooldown(3s) 兜底,ack(Pending) 不挡后续 Pending。
/// Idle/LastFailed 仍走原 acked 语义(认领过就别烦)。
///
/// 真实链路:popup 点击 → focus_session_terminal → acked_stages.mark(id, status)。
/// 测试侧直接调 mark 模拟,因为 Tauri command 的 State 注入需要 AppHandle。
///
/// 还原用户实际现场:Pending(用户点) → Idle → Pending(没有 Running 介入)。
/// 中间的 Idle 是"agent 完成"这条新 stage,应该正常弹;**第二次 Pending 也弹**
/// (Pending 不被 ack 抑制)。
#[test]
fn acked_stage_suppresses_same_status_until_running() {
    let f = fixture();

    // Round 1: Running → Pending,弹一次。用户点了 → 认领 Pending。
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    assert_eq!(f.popup.show_count(), 1, "首次 Pending 应弹");
    let session_id = store_snapshot(&f.state)[0].id.clone();
    f.memory.mark_acked(&session_id, SessionStatus::Pending);

    // agent 自己走 Pending → Idle → Pending(没有 Running 介入):
    //   - Idle 是"agent 完成"这条新 stage(ack 还是 Pending,不挡)→ 弹 popup #2
    //   - 紧接着的 Pending **不被 ack(Pending) 抑制**(Pending 例外)→ 弹 popup #3
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));
    assert_eq!(f.popup.show_count(), 2, "Idle 是新 stage,允许弹");

    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    assert_eq!(
        f.popup.show_count(),
        3,
        "Pending 不受 ack 抑制,Idle→Pending 第二次仍弹(旧 ack 语义下会是 2)",
    );
    // sound 与 popup 创建解耦(brief 20260805-pending-idle-completed-sound-swallowed):
    // 新 popup **或 refresh-only 的 stage 变化**(prev != status)都响音——
    // popup #1(Pending 新建)响 waiting;popup #2(Idle,Pending→Idle stage 变化,
    // 屏上已有 popup → refresh-only)响 completed——这正是"完成音不被在屏 popup
    // 吞掉"的修复本体;popup #3(Pending,Idle→Pending stage 变化)响 waiting
    // (Pending 是硬提醒,见 is_stage_popup_eligible 注释)。
    // 纯内容刷新(同 status)不响。同 status 的重复仍由 SoundCooldown(3s)兜底。
    assert_eq!(
        f.sound.play_count(),
        3,
        "新 popup + 每次 stage 变化各响一声"
    );
}

#[test]
fn acked_stage_reset_after_running_then_repops() {
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    assert_eq!(f.popup.show_count(), 1);

    let session_id = store_snapshot(&f.state)[0].id.clone();
    f.memory.mark_acked(&session_id, SessionStatus::Pending);

    // Pending → Running 进入"用户开始干活",ack 应被自动清掉。
    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    // 又一轮 Running → Pending:ack 已被 Running 清掉,应该再弹。
    process_notify(&f.state, req("cc-1", "pending", "claude-code"));
    assert_eq!(
        f.popup.show_count(),
        2,
        "Running 已清 ack,新一轮 Pending 应该再弹",
    );
    assert_eq!(f.sound.play_count(), 2);
}

#[test]
fn acked_stage_cleared_on_session_end() {
    // SessionEnd 移除 session 后 ack 也要清,免得偶发场景下同 id 残留(
    // 实际上 session_id 是 uuid 不会复用,但 SessionEnd clear 是显式正确路径)。
    let f = fixture();

    process_notify(&f.state, req("cc-1", "running", "claude-code"));
    process_notify(&f.state, req("cc-1", "idle", "claude-code"));
    let session_id = store_snapshot(&f.state)[0].id.clone();
    f.memory.mark_acked(&session_id, SessionStatus::Idle);
    assert!(f.memory.is_acked(&session_id, &SessionStatus::Idle));

    process_notify(&f.state, req_session_end("cc-1"));
    assert!(
        !f.memory.is_acked(&session_id, &SessionStatus::Idle),
        "SessionEnd 应清掉该 session 的 ack",
    );
}

#[test]
fn transcript_interruption_scanner_converts_running_session_to_pending_popup() {
    let f = fixture();
    let mut transcript = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        transcript,
        r#"{{"type":"user","uuid":"u-interrupt","message":{{"role":"user","content":[{{"type":"text","text":"[Request interrupted by user]"}}]}}}}"#
    )
    .unwrap();

    let mut running = req_with_tty("cc-interrupt-1", "running", "claude-code", "/dev/ttys001");
    running.transcript_path = Some(transcript.path().display().to_string());
    process_notify(&f.state, running);

    assert_eq!(store_snapshot(&f.state)[0].status, SessionStatus::Running);
    assert_eq!(f.popup.show_count(), 0);
    assert_eq!(f.sound.play_count(), 0);

    let seen = TranscriptInterruptionState::default();
    let detected = scan_once(&f.state, &seen);

    assert_eq!(detected, 1);
    let sessions = store_snapshot(&f.state);
    assert_eq!(sessions[0].status, SessionStatus::Pending);
    assert_eq!(
        sessions[0].message,
        "Claude Code conversation may have been interrupted. Click to inspect or continue."
    );
    assert_eq!(f.popup.show_count(), 1);
    assert_eq!(f.sound.play_count(), 1);
    assert_eq!(f.events.count(), 2);

    let detected_again = scan_once(&f.state, &seen);
    assert_eq!(detected_again, 0, "same marker should not re-trigger");
    assert_eq!(f.popup.show_count(), 1);
}

#[test]
fn notifications_are_never_quota_limited() {
    // 2026-11 移除 Pro 付费墙:不再有每日配额,连续触发都必须弹窗 + 放音。
    // 原用例是 `free_quota_exhausted_suppresses_popup_and_sound_but_upserts_session`,
    // 随 license 子系统一并删除。
    let f = fixture();
    for i in 0..25 {
        let req = NotifyRequest {
            title: "Test: project".into(),
            message: "Task completed".into(),
            task_id: format!("no-quota-{i}"),
            protocol_version: None,
            external_session_id: None,
            source: Some("claude-code".into()),
            priority: None,
            event_type: Some("stop".into()),
            status: Some("idle".into()),
            terminal_tty: None,
            workspace_path: Some("/tmp/proj".into()),
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
        };
        let _ = process_notify(&f.state, req);
    }
    assert_eq!(f.popup.show_count(), 25, "no quota: every notify must show");
}
#[test]
fn pro_plan_allows_popup_and_sound() {
    let tmp = tempfile::tempdir().unwrap();
    // Create a custom fixture with Pro license
    let store = Arc::new(Mutex::new(SessionStore::load(
        tmp.path().join("sessions.json"),
    )));
    let popup = Arc::new(SpyPopup::default());
    let sound = Arc::new(SpySound::default());
    let focus = Arc::new(SpyFocus::default());
    let events = Arc::new(SpyEvents::default());
    let gate = Arc::new(SpyGate::default());
    // Create Pro license

    let state = AppState {
        store: store.clone(),
        popup: popup.clone(),
        sound: sound.clone(),
        focus: focus.clone(),
        events: events.clone(),
        gate: gate.clone(),
        memory: Arc::new(SessionMemory::new(
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(15),
            std::time::Duration::ZERO,
        )),
        auto_approval: Arc::new(poke_poke_lib::codex_rollout::AutoApprovalCache::default()),
        codex_thread_status: Arc::new(
            poke_poke_lib::codex_appserver::CodexThreadStatusCache::default(),
        ),
        codex_appserver: poke_poke_lib::codex_appserver::CodexAppServerHandle::default(),
        settings: Arc::new(Mutex::new(poke_poke_lib::settings::SettingsStore::load(
            tmp.path().join("settings.json"),
        ))),
        quiet_mode: None,
        // 集成测试用同步 staging:断言可在 process_notify 返回后立即检查副作用。
        staging: StagingDispatcher::inline(),
    };

    // Send a notification that should trigger AttemptShow
    let req = NotifyRequest {
        protocol_version: None,
        title: "Test: project".into(),
        message: "Task completed".into(),
        task_id: "test-task-2".into(),
        external_session_id: Some("ext-test-2".into()),
        source: Some("claude-code".into()),
        priority: None,
        event_type: Some("stop".into()),
        status: Some("idle".into()),
        terminal_tty: None,
        workspace_path: Some("/tmp/proj".into()),
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
    };

    // Process the notification - should show popup and play sound
    let (status_code, _response) = process_notify(&state, req);
    assert_eq!(status_code, 201);
    assert_eq!(popup.shown.lock().unwrap().len(), 1);
    assert_eq!(sound.played.lock().unwrap().clone(), 1);

    // Session should still be upserted
    let store_guard = store.lock().unwrap();
    let sessions = store_guard.get_all();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].task_id, "test-task-2");
    assert_eq!(sessions[0].status, SessionStatus::Idle);
}

// ==================== Codex auto-approval popup gate ====================

fn session_id_for_task(state: &AppState, task_id: &str) -> String {
    store_snapshot(state)
        .into_iter()
        .find(|s| s.task_id == task_id)
        .expect("session")
        .id
}

fn thread_id_for_task(task_id: &str) -> String {
    format!("ext-{task_id}")
}

/// Layer A+B:codex 桌面端自动审批 + thread active → Idle 不弹 popup。
#[test]
fn codex_auto_approval_suppresses_idle_popup_when_thread_active() {
    let f = fixture();
    let task_id = "codex-auto-loop";

    process_notify(&f.state, req(task_id, "running", "codex"));
    let session_id = session_id_for_task(&f.state, task_id);
    let thread_id = thread_id_for_task(task_id);

    f.state.auto_approval.set(
        &session_id,
        poke_poke_lib::codex_rollout::AutoApprovalVerdict::AutoApproved,
    );
    f.state.codex_thread_status.set(
        &thread_id,
        poke_poke_lib::codex_appserver::CodexThreadRuntimeStatus::Active {
            waiting_on_approval: true,
            waiting_on_user_input: false,
        },
        poke_poke_lib::codex_appserver::ThreadStatusProvenance::LiveNotification,
    );

    process_notify(&f.state, req(task_id, "idle", "codex"));

    assert_eq!(
        f.popup.show_count(),
        0,
        "auto-review loop idle should not popup"
    );
    assert_eq!(
        store_snapshot(&f.state)[0].status,
        SessionStatus::Idle,
        "session still updates"
    );
}

/// Layer A+B:自动审批轮 thread 真结束(idle live) → 弹 popup。
#[test]
fn codex_auto_approval_shows_idle_popup_when_thread_idle_live() {
    let f = fixture();
    let task_id = "codex-auto-end";

    process_notify(&f.state, req(task_id, "running", "codex"));
    let session_id = session_id_for_task(&f.state, task_id);
    let thread_id = thread_id_for_task(task_id);

    f.state.auto_approval.set(
        &session_id,
        poke_poke_lib::codex_rollout::AutoApprovalVerdict::AutoApproved,
    );
    f.state.codex_thread_status.set(
        &thread_id,
        poke_poke_lib::codex_appserver::CodexThreadRuntimeStatus::Idle,
        poke_poke_lib::codex_appserver::ThreadStatusProvenance::LiveNotification,
    );

    process_notify(&f.state, req(task_id, "idle", "codex"));

    assert_eq!(f.popup.show_count(), 1, "true end should popup");
    assert_eq!(f.sound.play_count(), 1);
}

/// Layer B 未知:自动审批但 thread 缓存未命中 → Idle 放行(不误杀真结束)。
#[test]
fn codex_auto_approval_allows_idle_popup_when_thread_unknown() {
    let f = fixture();
    let task_id = "codex-auto-unknown";

    process_notify(&f.state, req(task_id, "running", "codex"));
    let session_id = session_id_for_task(&f.state, task_id);

    f.state.auto_approval.set(
        &session_id,
        poke_poke_lib::codex_rollout::AutoApprovalVerdict::AutoApproved,
    );

    process_notify(&f.state, req(task_id, "idle", "codex"));

    assert_eq!(
        f.popup.show_count(),
        1,
        "unknown thread status should allow idle popup"
    );
}

/// 接入端手动断开后 `/notify` 应被忽略且不创建 session / popup。
#[test]
fn disconnected_integration_blocks_notify_side_effects() {
    let f = fixture();
    {
        let mut s = f.state.settings.lock().unwrap_or_else(|p| p.into_inner());
        s.settings.disconnected_sources.push("cursor".into());
        s.save();
    }

    let mut notify = req("cursor-disconnect-gate", "running", "cursor");
    notify.event_type = Some("sessionStart".into());

    let (code, body) = process_notify(&f.state, notify);
    assert_eq!(code, axum::http::StatusCode::OK);
    assert_eq!(body["ignored"], true);
    assert_eq!(body["reason"], "integration_disconnected");

    assert!(
        f.state
            .store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_all()
            .is_empty(),
        "disconnected source must not create sessions"
    );
    assert_eq!(
        f.popup.show_count(),
        0,
        "disconnected source must not show popup"
    );
    assert_eq!(f.sound.play_count(), 0);
}

// ==================== herdr_remote 远程监视器(brief 20260804-herdr-remote-notification 修订版) ====================
// 公司机 PokePoke 经 herdr socket API 监视远程 agent:状态翻译 → process_notify,
// 完整复用本地状态机 / popup gate。本组测试直接驱动 poll_once(不经 socket 层),
// 验证翻译 + diff + focused 抑制 + 消失清理。

use poke_poke_lib::herdr::HerdrAgentStatus;
use poke_poke_lib::herdr_remote::{poll_once, Connection, RemoteAgent};

fn hr_agent(pane: &str, agent: &str, status: HerdrAgentStatus, focused: bool) -> RemoteAgent {
    RemoteAgent {
        pane_id: pane.into(),
        workspace_id: "w1".into(),
        agent: Some(agent.into()),
        status,
        cwd: "/Users/x/MyProject".into(),
        foreground_cwd: "/Users/x/MyProject".into(),
        focused,
    }
}

fn hr_sessions(state: &AppState) -> Vec<poke_poke_lib::sessions::Session> {
    state
        .store
        .lock()
        .unwrap()
        .get_all()
        .iter()
        .filter(|s| s.task_id.starts_with("hr-"))
        .cloned()
        .collect()
}

fn hr_remote_conn() -> Connection {
    Connection::Remote {
        local_socket: "/tmp/fwd-hello.sock".into(),
        target: "hello".into(),
        control: "/tmp/ctl".into(),
        remote_socket: "/Users/x/.config/herdr/herdr.sock".into(),
    }
}

/// working → running(不弹);blocked → pending(弹 + 声音)。task_id 用 hr- 前缀。
/// brief 20260813 Phase 2 后用 Remote 连接:本地 + 七端 agent 已被 hook 通路
/// 接管,monitor 喂料路径的验证改走远程通道(本地非 hook 场景另有测试)。
#[test]
fn herdr_remote_working_then_blocked_creates_pending_popup() {
    let f = fixture();
    let conn = hr_remote_conn();
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent(
            "w1:p1",
            "claude",
            HerdrAgentStatus::Working,
            false,
        )],
    );
    let s = hr_sessions(&f.state)
        .into_iter()
        .find(|s| s.task_id == "hr-hello-w1:p1")
        .expect("working agent should create session");
    assert_eq!(s.status, SessionStatus::Running);
    assert_eq!(s.title, "MyProject");
    assert_eq!(f.popup.show_count(), 0, "running 不弹");

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent(
            "w1:p1",
            "claude",
            HerdrAgentStatus::Blocked,
            false,
        )],
    );
    let s = hr_sessions(&f.state)
        .into_iter()
        .find(|s| s.task_id == "hr-hello-w1:p1")
        .expect("session still present");
    assert_eq!(s.status, SessionStatus::Pending);
    assert_eq!(f.popup.show_count(), 1, "blocked 弹 popup");
    assert!(f.sound.play_count() >= 1, "blocked 播 request 音");
}

/// focused=true 不再抑制(2026-08-07 用户反馈:远程场景聚焦≠人在看,抑制会漏弹
/// blocked);focused 与否都喂状态变化,弹窗由 cooldown/ack 兜底防重复。
#[test]
fn herdr_remote_focused_does_not_suppress() {
    let f = fixture();
    let conn = hr_remote_conn();
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Working, true)],
    );
    assert_eq!(f.popup.show_count(), 0, "working 不弹");

    // 即使 focused=true,blocked 也喂 → 弹
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Blocked, true)],
    );
    let s = hr_sessions(&f.state)
        .into_iter()
        .find(|s| s.task_id == "hr-hello-w1:p1")
        .unwrap();
    assert_eq!(s.status, SessionStatus::Pending);
    assert_eq!(f.popup.show_count(), 1, "focused 时 blocked 也弹");
    assert!(f.sound.play_count() >= 1);
}

/// 远程连接(带 target)的 task_id 含 target:跨主机同 pane_id 不互相覆盖
/// (brief 20260807-herdr-remote-jumpback-fix)。
#[test]
fn herdr_remote_task_id_includes_target_end_to_end() {
    let f = fixture();
    let conn = Connection::Remote {
        local_socket: "/tmp/fwd-hello.sock".into(),
        target: "hello".into(),
        control: "/tmp/ctl".into(),
        remote_socket: "/Users/lishaoxin/.config/herdr/herdr.sock".into(),
    };
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent(
            "w1:p1",
            "claude",
            HerdrAgentStatus::Working,
            false,
        )],
    );
    let s = hr_sessions(&f.state)
        .into_iter()
        .find(|s| s.task_id == "hr-hello-w1:p1")
        .expect("remote session should use hr-<target>-<pane_id> task_id");
    assert_eq!(s.status, SessionStatus::Running);
    assert_eq!(s.herdr_pane_id.as_deref(), Some("w1:p1"));

    // 另一台远程主机同 pane_id:task_id 不同,不会覆盖上一个 session。
    let conn2 = Connection::Remote {
        local_socket: "/tmp/fwd-other.sock".into(),
        target: "other".into(),
        control: "/tmp/ctl2".into(),
        remote_socket: "/Users/x/.config/herdr/herdr.sock".into(),
    };
    let mut known2: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();
    poll_once(
        &f.state,
        &conn2,
        &mut known2,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Working, false)],
    );
    let ids: Vec<String> = hr_sessions(&f.state)
        .into_iter()
        .map(|s| s.task_id)
        .collect();
    assert!(ids.contains(&"hr-hello-w1:p1".to_string()));
    assert!(ids.contains(&"hr-other-w1:p1".to_string()));
    assert_eq!(ids.len(), 2, "两台主机同 pane 应各自独立 session");
}

/// 首次观测到已 idle/done 的 pane:不弹「干完了」(回溯状态不是迁移,
/// brief 20260810-herdr-remote-first-observation-silent,用户实测:herdr --remote
/// 刚连上即弹 + app 重启后重弹,根因同一)。标记已对齐后,真实迁移
/// (working → idle)才弹。
#[test]
fn herdr_remote_first_observation_idle_is_silent() {
    let f = fixture();
    let conn = hr_remote_conn();
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    // 首观测 idle:不建 session、不弹、不响
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "claude", HerdrAgentStatus::Idle, false)],
    );
    assert!(
        hr_sessions(&f.state).is_empty(),
        "首观测 idle 不应建 session"
    );
    assert_eq!(f.popup.show_count(), 0, "首观测 idle 不弹");
    assert_eq!(f.sound.play_count(), 0, "首观测 idle 不响");

    // 同状态再轮:known 已对齐,不会把回溯状态当迁移再喂
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "claude", HerdrAgentStatus::Idle, false)],
    );
    assert!(hr_sessions(&f.state).is_empty());
    assert_eq!(f.popup.show_count(), 0);

    // 真实迁移:working → idle 才弹
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent(
            "w1:p1",
            "claude",
            HerdrAgentStatus::Working,
            false,
        )],
    );
    let s = hr_sessions(&f.state)
        .into_iter()
        .find(|s| s.task_id == "hr-hello-w1:p1")
        .expect("working 应建 session");
    assert_eq!(s.status, SessionStatus::Running);
    assert_eq!(f.popup.show_count(), 0, "working 不弹");

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "claude", HerdrAgentStatus::Idle, false)],
    );
    assert_eq!(f.popup.show_count(), 1, "真实 idle 迁移才弹");
}

/// agent 从 agent.list 消失:连续 3 轮确认后才 session_end 清理(防偶发丢数据误清)。
#[test]
fn herdr_remote_disappearance_ends_session_after_threshold() {
    let f = fixture();
    let conn = hr_remote_conn();
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent(
            "w1:p1",
            "claude",
            HerdrAgentStatus::Working,
            false,
        )],
    );
    assert!(!hr_sessions(&f.state).is_empty());

    // 前两轮消失:保持会话
    poll_once(&f.state, &conn, &mut known, &[]);
    poll_once(&f.state, &conn, &mut known, &[]);
    assert!(!hr_sessions(&f.state).is_empty(), "2 轮消失不清理");

    // 第 3 轮:session_end
    poll_once(&f.state, &conn, &mut known, &[]);
    assert!(hr_sessions(&f.state).is_empty(), "3 轮消失后清理");
}

/// Phase 2(brief 20260813-herdr-triple-popup):本地 pane 跑的是自带 hook 的七端
/// agent 时,监视器不喂 pane 级任务(hook 通路已接管),状态迁移/消失清理均无
/// 副作用;本地非 hook agent(未知名)仍走 pane 级通道。
#[test]
fn herdr_local_hook_capable_agent_never_fed() {
    let f = fixture();
    let conn = Connection::Local("/tmp/fake.sock".into());
    let mut known: HashMap<String, poke_poke_lib::herdr_remote::Known> = HashMap::new();

    // pi(七端)在本地 pane:working/blocked/done 全程不建 session、不弹、不响。
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Working, false)],
    );
    assert!(
        hr_sessions(&f.state).is_empty(),
        "本地 pi pane 不应建 session"
    );
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Blocked, false)],
    );
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w1:p1", "pi", HerdrAgentStatus::Done, false)],
    );
    assert!(hr_sessions(&f.state).is_empty());
    assert_eq!(f.popup.show_count(), 0, "跳过期间全程不弹");
    assert_eq!(f.sound.play_count(), 0, "跳过期间全程不响");

    // 消失清理对从未建过的 session 是 no-op。
    poll_once(&f.state, &conn, &mut known, &[]);
    poll_once(&f.state, &conn, &mut known, &[]);
    poll_once(&f.state, &conn, &mut known, &[]);
    assert!(hr_sessions(&f.state).is_empty());

    // 反向守卫:本地非 hook agent(未知名)仍走 pane 级通道。
    poll_once(
        &f.state,
        &conn,
        &mut known,
        &[hr_agent("w2:p1", "foo", HerdrAgentStatus::Working, false)],
    );
    assert!(
        hr_sessions(&f.state)
            .iter()
            .any(|s| s.task_id == "hr-w2:p1"),
        "本地非 hook agent 应照常建 session"
    );
}
