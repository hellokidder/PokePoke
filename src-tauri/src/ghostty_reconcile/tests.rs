use super::*;

fn snap(id: &str, cwd: &str, title: &str) -> GhosttySnapshot {
    GhosttySnapshot {
        terminal_id: id.into(),
        working_directory: cwd.into(),
        title: title.into(),
    }
}

fn cand(task: &str, ext: Option<&str>, ws: Option<&str>, tid: Option<&str>) -> ReconcileCandidate {
    ReconcileCandidate {
        task_id: task.into(),
        external_session_id: ext.map(str::to_string),
        workspace_path: ws.map(str::to_string),
        terminal_id: tid.map(str::to_string),
    }
}

// 测试用 herdr pane 快照:仅 pane_id / agent 可变,其余字段固定
// (agent_status 新增后不重复写死,见 20260804-herdr-agent-status)。
fn pane(pid: &str, agent: Option<&str>) -> crate::herdr::HerdrPaneSnapshot {
    crate::herdr::HerdrPaneSnapshot {
        pane_id: pid.into(),
        workspace_id: "w3".into(),
        agent: agent.map(str::to_string),
        cwd: "/tmp/proj".into(),
        foreground_cwd: "/tmp/proj".into(),
        agent_status: crate::herdr::HerdrAgentStatus::Unknown,
        agent_session: None,
    }
}

// ---- title_matches_session_prefix ----

#[test]
fn title_matches_truncated_session_id_prefix() {
    // 现场真实 title 形态:CC 把 id 截成 16 字符尾缀。
    let hit = title_matches_session_prefix(
        "poke-poke · 看一下这个项目 · 14f71163-274b-4d",
        "14f71163-274b-4d0b-8e18-5a73f7c02bdd",
    );
    assert_eq!(hit, Some(16));
}

#[test]
fn title_overwritten_by_command_name_does_not_match() {
    // tab 里跑 `git log`,title 被覆盖,session id 消失 → 不该 title 命中。
    assert_eq!(
        title_matches_session_prefix("git log", "14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
        None
    );
    assert_eq!(
        title_matches_session_prefix("edy@kidder:~", "14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
        None
    );
}

#[test]
fn title_short_coincidental_prefix_rejected() {
    // title 里只巧合出现了 <8 字符的片段,不算命中(防短前缀撞车)。
    assert_eq!(
        title_matches_session_prefix("foo 14f7 bar", "14f71163-aaaa"),
        None
    );
}

#[test]
fn title_full_id_present_matches_full_length() {
    let id = "14f71163-274b-4d0b-8e18-5a73f7c02bdd";
    assert_eq!(
        title_matches_session_prefix(&format!("x {id} y"), id),
        Some(id.len())
    );
}

// ---- reconcile_assignments ----

#[test]
fn assigns_by_title_primary_key() {
    let snaps = vec![
        snap("UUID-A", "/Users/edy", "poke-poke · … · 14f71163-274b-4d"),
        snap("UUID-B", "/Users/edy", "git log"),
    ];
    let cands = vec![cand(
        "cc-14f71163",
        Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
        Some("/Users/edy/Documents/Kidder/poke-poke"),
        None,
    )];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].terminal_id, "UUID-A");
    assert_eq!(out[0].matched_by, MatchKey::Title);
}

#[test]
fn title_wins_over_cwd_when_both_available() {
    // snapshot B 的 cwd 跟 workspace 兼容,但 snapshot A 的 title 命中。
    // 必须选 A(title 优先),否则又回到 cwd 错绑老路。
    let snaps = vec![
        snap("UUID-A", "/Users/edy", "proj · … · 14f71163-274b-4d"),
        snap("UUID-B", "/Users/edy/proj", "git log"),
    ];
    let cands = vec![cand(
        "cc-1",
        Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
        Some("/Users/edy/proj"),
        None,
    )];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].terminal_id, "UUID-A");
    assert_eq!(out[0].matched_by, MatchKey::Title);
}

#[test]
fn falls_back_to_cwd_when_title_absent() {
    // unique_cwd_binds:title 全被命令名覆盖,但整个 workspace 只有 1 个 cwd 兼容
    // snapshot(1↔1)→ cwd 兜底命中(OSC7 正常的单窗口环境)。
    let snaps = vec![snap("UUID-A", "/Users/edy/proj", "pnpm dev")];
    let cands = vec![cand(
        "cc-1",
        Some("nomatch-id-xxxx"),
        Some("/Users/edy/proj"),
        None,
    )];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].terminal_id, "UUID-A");
    assert_eq!(out[0].matched_by, MatchKey::Cwd);
}

#[test]
fn two_sessions_same_cwd_disambiguated_by_title() {
    // 核心治本场景:两个 session 同项目同 cwd,靠 title 各自精准绑定,不撞车。
    let snaps = vec![
        snap("UUID-A", "/Users/edy", "poke-poke · … · 14f71163-274b-4d"),
        snap("UUID-B", "/Users/edy", "poke-poke · … · 910e7cbb-9168-46"),
    ];
    let cands = vec![
        cand(
            "cc-14f71163",
            Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
            Some("/p"),
            None,
        ),
        cand(
            "cc-910e7cbb",
            Some("910e7cbb-9168-4600-aaaa-bbbbccccdddd"),
            Some("/p"),
            None,
        ),
    ];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|x| x.task_id == "cc-14f71163").unwrap();
    let b = out.iter().find(|x| x.task_id == "cc-910e7cbb").unwrap();
    assert_eq!(a.terminal_id, "UUID-A");
    assert_eq!(b.terminal_id, "UUID-B");
}

#[test]
fn ambiguous_same_cwd_skips_without_title_key() {
    // 核心 regression:2 sessions ↔ 2 snapshots 同 cwd,title 无 key。
    // capture 层判弱证据不落库,reconcile 也必须一致 skip —— 否则枚举顺序决定谁
    // 绑谁,恒错(本次多 tab 同 cwd 错绑事故)。
    let snaps = vec![
        snap("UUID-A", "/Users/edy/proj", "pnpm dev"),
        snap("UUID-B", "/Users/edy/proj", "git log"),
    ];
    let cands = vec![
        cand("cc-1", Some("noidnoid-1111"), Some("/Users/edy/proj"), None),
        cand("cc-2", Some("noidnoid-2222"), Some("/Users/edy/proj"), None),
    ];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert!(
        out.is_empty(),
        "同 cwd 多候选无 title key 必须全部 skip,实际: {out:?}"
    );
}

#[test]
fn ambiguous_two_sessions_one_snapshot_skips() {
    // 反向 degree:2 sessions 抢同一个 cwd 兼容 snapshot,无 title key。
    // 该 snapshot 被 2 个 session 争用 → degree≥2 → skip(不能 first-match 挑一个)。
    let snaps = vec![snap("UUID-A", "/Users/edy/proj", "shell")];
    let cands = vec![
        cand("cc-1", Some("noidnoid-1111"), Some("/Users/edy/proj"), None),
        cand("cc-2", Some("noidnoid-2222"), Some("/Users/edy/proj"), None),
    ];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert!(
        out.is_empty(),
        "一个 snapshot 被多 session 争用必须 skip,实际: {out:?}"
    );
}

#[test]
fn title_key_still_disambiguates_same_cwd() {
    // title_key_still_disambiguates:2 sessions ↔ 2 snapshots 同 cwd,但 title 有
    // key → Pass1 强证据各自精准 bind,不受 Pass2 收紧影响。
    let snaps = vec![
        snap("UUID-A", "/Users/edy/proj", "proj · … · 14f71163-274b-4d"),
        snap("UUID-B", "/Users/edy/proj", "proj · … · 910e7cbb-9168-46"),
    ];
    let cands = vec![
        cand(
            "cc-14f71163",
            Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
            Some("/Users/edy/proj"),
            None,
        ),
        cand(
            "cc-910e7cbb",
            Some("910e7cbb-9168-4600-aaaa-bbbbccccdddd"),
            Some("/Users/edy/proj"),
            None,
        ),
    ];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|x| x.task_id == "cc-14f71163").unwrap();
    let b = out.iter().find(|x| x.task_id == "cc-910e7cbb").unwrap();
    assert_eq!(a.terminal_id, "UUID-A");
    assert_eq!(b.terminal_id, "UUID-B");
    assert_eq!(a.matched_by, MatchKey::Title);
}

#[test]
fn one_titled_one_cwd_unique_both_bind() {
    // 混合:session-1 靠 title 认领 snapshot-A,剩下 session-2 与 snapshot-B 成
    // 唯一 cwd 对(1↔1)→ Pass2 兜底 bind。验证 Pass1 认领后 Pass2 的 degree
    // 计算基于"剩余未认领"集合。
    let snaps = vec![
        snap("UUID-A", "/Users/edy/proj", "proj · … · 14f71163-274b-4d"),
        snap("UUID-B", "/Users/edy/other", "pnpm dev"),
    ];
    let cands = vec![
        cand(
            "cc-14f71163",
            Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
            Some("/Users/edy/proj"),
            None,
        ),
        cand(
            "cc-2",
            Some("noidnoid-2222"),
            Some("/Users/edy/other"),
            None,
        ),
    ];
    let out = reconcile_assignments(&snaps, &cands).assignments;
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|x| x.task_id == "cc-14f71163").unwrap();
    let b = out.iter().find(|x| x.task_id == "cc-2").unwrap();
    assert_eq!(a.terminal_id, "UUID-A");
    assert_eq!(a.matched_by, MatchKey::Title);
    assert_eq!(b.terminal_id, "UUID-B");
    assert_eq!(b.matched_by, MatchKey::Cwd);
}

#[test]
fn no_candidates_no_snapshots_empty() {
    assert!(reconcile_assignments(&[], &[]).assignments.is_empty());
    let snaps = vec![snap("U", "/c", "t")];
    assert!(reconcile_assignments(&snaps, &[]).assignments.is_empty());
}

#[test]
fn ambiguous_skip_surfaces_reason_in_result() {
    // 弱证据 skip 不再直接打日志,而是带出到 ReconcileResult.skips,让 watcher
    // 做状态转变 gate。验证 ambiguous-cwd 原因 + compatible_snapshots 数正确冒泡。
    let snaps = vec![
        snap("UUID-A", "/Users/edy/proj", "pnpm dev"),
        snap("UUID-B", "/Users/edy/proj", "git log"),
    ];
    let cands = vec![cand(
        "cc-1",
        Some("noidnoid-1111"),
        Some("/Users/edy/proj"),
        None,
    )];
    let result = reconcile_assignments(&snaps, &cands);
    assert!(result.assignments.is_empty());
    assert_eq!(result.skips.len(), 1);
    assert_eq!(result.skips[0].task_id, "cc-1");
    assert_eq!(
        result.skips[0].reason,
        SkipReason::AmbiguousCwd {
            compatible_snapshots: 2
        }
    );
}

#[test]
fn contested_skip_surfaces_reason_in_result() {
    // 反向 degree:2 sessions 抢 1 snapshot -> Contested 原因冒泡(contenders=2)。
    let snaps = vec![snap("UUID-A", "/Users/edy/proj", "shell")];
    let cands = vec![
        cand("cc-1", Some("noidnoid-1111"), Some("/Users/edy/proj"), None),
        cand("cc-2", Some("noidnoid-2222"), Some("/Users/edy/proj"), None),
    ];
    let result = reconcile_assignments(&snaps, &cands);
    assert!(result.assignments.is_empty());
    // 两个 candidate 都争用同一 snapshot -> 各产生一条 Contested skip
    assert_eq!(result.skips.len(), 2);
    for ev in &result.skips {
        assert_eq!(
            ev.reason,
            SkipReason::Contested { contenders: 2 },
            "expected Contested contenders=2, got {:?}",
            ev.reason
        );
    }
}

#[test]
fn bound_session_produces_no_skip_event() {
    // 成功 bind 的 session 不应产生 skip 事件(成功路径另有 log,不进 skips)。
    let snaps = vec![snap("UUID-A", "/Users/edy/proj", "pnpm dev")];
    let cands = vec![cand(
        "cc-1",
        Some("nomatch-id-xxxx"),
        Some("/Users/edy/proj"),
        None,
    )];
    let result = reconcile_assignments(&snaps, &cands);
    assert_eq!(result.assignments.len(), 1);
    assert!(
        result.skips.is_empty(),
        "bound session should not skip: {:?}",
        result.skips
    );
}

#[test]
fn should_log_skip_logs_on_first_occurrence() {
    // 首次出现(prev=None) -> 必须打,这是"进入 ambiguous"的有用信号。
    assert!(should_log_skip(
        None,
        &SkipReason::AmbiguousCwd {
            compatible_snapshots: 3
        }
    ));
}

#[test]
fn should_log_skip_suppresses_unchanged_repeat() {
    // 稳态:同 task_id 同原因(prev == cur) -> 不打。这是修复的核心目标:
    // 长 ambiguous 会话每 8s 不再刷一条。
    let prev = SkipReason::AmbiguousCwd {
        compatible_snapshots: 3,
    };
    let cur = SkipReason::AmbiguousCwd {
        compatible_snapshots: 3,
    };
    assert!(!should_log_skip(Some(&prev), &cur));
}

#[test]
fn should_log_skip_logs_on_reason_change() {
    // 原因变化(ambiguous 变 contested,或 snapshots 数变化) -> 打,让用户看到
    // 状态演进。compatible_snapshots 数变也视为转变。
    let prev = SkipReason::AmbiguousCwd {
        compatible_snapshots: 3,
    };
    let cur_more = SkipReason::AmbiguousCwd {
        compatible_snapshots: 4,
    };
    assert!(should_log_skip(Some(&prev), &cur_more));
    let cur_contested = SkipReason::Contested { contenders: 2 };
    assert!(should_log_skip(Some(&prev), &cur_contested));
}

// ---- parse_enumerate_output ----

#[test]
fn parse_enumerate_splits_records_and_fields() {
    let raw = format!(
        "UUID-A{f}/Users/edy{f}poke-poke · … · 14f71163{r}UUID-B{f}/Users/edy/proj{f}git log",
        f = FIELD_SEP,
        r = RECORD_SEP
    );
    let out = parse_enumerate_output(&raw);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].terminal_id, "UUID-A");
    assert_eq!(out[0].working_directory, "/Users/edy");
    assert_eq!(out[0].title, "poke-poke · … · 14f71163");
    assert_eq!(out[1].terminal_id, "UUID-B");
    assert_eq!(out[1].title, "git log");
}

#[test]
fn parse_enumerate_drops_empty_id_and_handles_missing_fields() {
    // 空 id 行丢弃;缺 title 字段容忍为空。
    let raw = format!(
        "{f}/cwd{f}orphan{r}UUID-C{f}/only-cwd",
        f = FIELD_SEP,
        r = RECORD_SEP
    );
    let out = parse_enumerate_output(&raw);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].terminal_id, "UUID-C");
    assert_eq!(out[0].working_directory, "/only-cwd");
    assert_eq!(out[0].title, "");
}

#[test]
fn parse_enumerate_empty_input() {
    assert!(parse_enumerate_output("").is_empty());
}

// ---- collect_candidates (review H5) ----

fn session_stub(
    source: Option<&str>,
    ext: Option<&str>,
    ws: Option<&str>,
    tid: Option<&str>,
) -> crate::sessions::Session {
    let now = chrono::Utc::now();
    crate::sessions::Session {
        id: "sess-x".into(),
        task_id: "tid-x".into(),
        title: "".into(),
        message: "".into(),
        source: source.map(str::to_string),
        terminal_tty: None,
        terminal_id: tid.map(str::to_string),
        terminal_id_captured_cwd: None,
        workspace_path: ws.map(str::to_string),
        external_session_id: ext.map(str::to_string),
        status: crate::sessions::SessionStatus::Running,
        priority: crate::sessions::Priority::Normal,
        failure_reason: None,
        aborted_at: None,
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
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn collect_candidates_includes_bound_session_with_ext_id() {
    // review H5:即便 terminal_id 已在 live 枚举里,只要有 ext_id 就该纳入,
    // 好让 Pass1 title 强证据 可能覆盖(错绑纠正)。
    let s = session_stub(
        Some("claude-code"),
        Some("14f71163-274b-4d0b-8e18-5a73f7c02bdd"),
        Some("/proj"),
        Some("UUID-A"), // 已绑
    );
    let mut live = std::collections::HashSet::new();
    live.insert("UUID-A".to_string()); // UUID-A 存活

    let cands = collect_candidates(&[s], &live);
    assert_eq!(cands.len(), 1, "应纳入,允许 Pass1 by_title 覆盖");
}

#[test]
fn collect_candidates_skips_bound_session_without_ext_id() {
    // 无 ext_id + terminal_id 已存活:幂等 skip(原有行为保留)
    let s = session_stub(
        Some("claude-code"),
        None, // 无 ext_id
        Some("/proj"),
        Some("UUID-A"),
    );
    let mut live = std::collections::HashSet::new();
    live.insert("UUID-A".to_string());

    let cands = collect_candidates(&[s], &live);
    assert!(cands.is_empty(), "无 ext_id + 已绑存活 → skip");
}

#[test]
fn collect_candidates_includes_empty_terminal_id() {
    // 补空:terminal_id 为 None
    let s = session_stub(
        Some("claude-code"),
        Some("some-ext-id"),
        Some("/proj"),
        None,
    );
    let live = std::collections::HashSet::new();
    let cands = collect_candidates(&[s], &live);
    assert_eq!(cands.len(), 1);
}

#[test]
fn collect_candidates_skips_non_ghostty_source() {
    // cursor / iTerm2 / Terminal 都不该走 Ghostty reconcile
    for src in ["cursor", "unknown"] {
        let s = session_stub(Some(src), Some("ext"), Some("/proj"), None);
        let live = std::collections::HashSet::new();
        let cands = collect_candidates(&[s], &live);
        assert!(cands.is_empty(), "source={} 应 skip", src);
    }
}

#[test]
fn collect_candidates_skips_wezterm_bound_session() {
    let mut s = session_stub(
        Some("claude-code"),
        Some("ext-wez"),
        Some("/tmp/proj"),
        None,
    );
    s.wezterm_pane_id = Some("4".into());
    s.wezterm_unix_socket = Some("/tmp/wez.sock".into());
    // W6:读侧走 surface;镜像字段仅供迁移,决策看 surface。
    s.surface = crate::surface::SurfaceBinding::Wezterm {
        pane_id: "4".into(),
        unix_socket: Some("/tmp/wez.sock".into()),
        executable: None,
    };
    let live = std::collections::HashSet::new();
    let cands = collect_candidates(&[s], &live);
    assert!(
        cands.is_empty(),
        "wezterm binding 已存在时不应 Ghostty reconcile"
    );
}

// ---- pick_herdr_pane_for_session ----

#[test]
fn pick_herdr_pane_unique_match_binds() {
    let panes = vec![pane("w3:p1", Some("claude")), pane("w3:p2", None)];
    let picked = pick_herdr_pane_for_session(&panes, "/tmp/proj", Some("claude-code"));
    assert_eq!(picked, Some(("w3:p1".into(), "w3".into())));
}

#[test]
fn pick_herdr_pane_multi_match_skips() {
    let panes = vec![pane("w3:p1", Some("claude")), pane("w3:p2", Some("claude"))];
    let picked = pick_herdr_pane_for_session(&panes, "/tmp/proj", Some("claude-code"));
    assert!(picked.is_none(), "同 workspace 多 claude pane 应 skip");
}

#[test]
fn pick_herdr_pane_wrong_source_skips() {
    let panes = vec![pane("w3:p1", Some("claude"))];
    // cursor 不该走 herdr binding path(有 tty 精准跳)
    assert!(pick_herdr_pane_for_session(&panes, "/tmp/proj", Some("cursor")).is_none());
    assert!(pick_herdr_pane_for_session(&panes, "/tmp/proj", None).is_none());
}

// ---- next_sleep backoff state machine ----
//
// brief 20260715-ghostty-reconcile-timeout-backoff §Acceptance:
// 0→1→N→封顶→重置 五个状态转换。

#[test]
fn next_sleep_returns_base_when_osascript_ok() {
    // 无论上一轮多长,一次成功立即归零到 RECONCILE_INTERVAL。
    assert_eq!(next_sleep(RECONCILE_INTERVAL, true), RECONCILE_INTERVAL);
    assert_eq!(
        next_sleep(Duration::from_secs(16), true),
        RECONCILE_INTERVAL
    );
    assert_eq!(
        next_sleep(Duration::from_secs(32), true),
        RECONCILE_INTERVAL
    );
    assert_eq!(next_sleep(RECONCILE_BACKOFF_MAX, true), RECONCILE_INTERVAL);
}

#[test]
fn next_sleep_doubles_on_timeout_from_base() {
    // 0→1: 8s(RECONCILE_INTERVAL) 一次 timeout 后 → 16s
    assert_eq!(
        next_sleep(RECONCILE_INTERVAL, false),
        Duration::from_secs(16)
    );
}

#[test]
fn next_sleep_doubles_on_repeated_timeout() {
    // 1→2→3: 16 → 32 → 60(先翻倍到 64,超过封顶,取封顶 60)
    assert_eq!(
        next_sleep(Duration::from_secs(16), false),
        Duration::from_secs(32)
    );
    assert_eq!(
        next_sleep(Duration::from_secs(32), false),
        RECONCILE_BACKOFF_MAX
    );
}

#[test]
fn next_sleep_caps_at_max() {
    // 封顶: 已经在封顶继续 timeout 不再增长
    assert_eq!(
        next_sleep(RECONCILE_BACKOFF_MAX, false),
        RECONCILE_BACKOFF_MAX
    );
    // 就算 prev 超过封顶(不应发生但极端保护),也钳到封顶
    assert_eq!(
        next_sleep(Duration::from_secs(120), false),
        RECONCILE_BACKOFF_MAX
    );
}

#[test]
fn next_sleep_recovers_after_success_in_the_middle() {
    // 重置: 退到 32s 后一次成功 → 立即回 8s
    let mid = Duration::from_secs(32);
    assert_eq!(next_sleep(mid, true), RECONCILE_INTERVAL);
}

#[test]
fn next_sleep_zero_prev_defends_to_base() {
    // 极端保护:prev == 0 时(不应发生)至少走 RECONCILE_INTERVAL,防止 busy loop。
    assert_eq!(next_sleep(Duration::ZERO, false), RECONCILE_INTERVAL);
}
