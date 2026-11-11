use super::*;
use crate::settings::Settings;
use chrono::Utc;
use serial_test::serial;

// HOME 是进程级共享 env,改它的测试必须串行,否则 cargo test 默认多线程下打架。
// 走 serial_test 全局锁,与 agent_presence.rs 等其他改 HOME 的模块跨文件互斥。

fn make_session(status: SessionStatus, source: &str) -> Session {
    let now = Utc::now();
    Session {
        id: "id".into(),
        task_id: "task".into(),
        external_session_id: None,
        title: "SECRET_TITLE".into(),
        message: "SECRET_MESSAGE_BODY".into(),
        source: Some(source.into()),
        priority: Default::default(),
        status,
        created_at: now,
        updated_at: now,
        terminal_tty: Some("/dev/ttys003".into()),
        workspace_path: Some("/Users/secret/project".into()),
        failure_reason: None,
        aborted_at: None,
        terminal_id: None,
        terminal_id_captured_cwd: None,
        transcript_path: Some("/Users/secret/.claude/transcript.jsonl".into()),
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
fn settings_digest_includes_non_sensitive_fields() {
    let digest = settings_digest(&Settings::default());
    let json = serde_json::to_string(&digest).unwrap();
    assert!(
        json.contains("\"locale\""),
        "locale should be present: {json}"
    );
    assert!(
        json.contains("\"notifications_enabled\""),
        "field should be present: {json}"
    );
}

#[test]
fn sessions_digest_counts_without_leaking_content() {
    let sessions = vec![
        make_session(SessionStatus::Running, "claude-code"),
        make_session(SessionStatus::Pending, "claude-code"),
        make_session(SessionStatus::Idle, "cursor"),
    ];
    let digest = sessions_digest(&sessions);
    let json = serde_json::to_string(&digest).unwrap();

    assert_eq!(digest.total, 3);
    assert_eq!(digest.by_status.running, 1);
    assert_eq!(digest.by_status.pending, 1);
    assert_eq!(digest.by_status.idle, 1);
    assert_eq!(digest.by_source.get("claude-code"), Some(&2));
    assert_eq!(digest.by_source.get("cursor"), Some(&1));

    // 单条内容绝不出现
    assert!(
        !json.contains("SECRET_MESSAGE_BODY"),
        "message leaked: {json}"
    );
    assert!(!json.contains("SECRET_TITLE"), "title leaked: {json}");
    assert!(
        !json.contains("/Users/secret"),
        "workspace path leaked: {json}"
    );
    assert!(!json.contains("ttys003"), "tty leaked: {json}");
    assert!(
        !json.contains("transcript"),
        "transcript path leaked: {json}"
    );
}

#[test]
#[serial]
fn tilde_collapse_replaces_home_prefix() {
    std::env::set_var("HOME", "/Users/alice");
    assert_eq!(
        tilde_collapse("/Users/alice/.local/bin/poke-hook"),
        "~/.local/bin/poke-hook"
    );
    // 非 HOME 前缀不动
    assert_eq!(tilde_collapse("/opt/tool"), "/opt/tool");
}

#[test]
fn whitelist_check_drops_unknown_fields() {
    let raw = serde_json::json!({
        "installed": true,
        "hooks_configured": true,
        "connected": true,
        "hooks_outdated": false,
        "secret_path": "/Users/secret/leak",
    });
    let check = whitelist_check(&raw, None);
    let json = serde_json::to_string(&check).unwrap();
    assert!(
        !json.contains("/Users/secret/leak"),
        "extra field leaked: {json}"
    );
    assert!(!json.contains("secret_path"), "extra key leaked: {json}");
    assert!(check.installed && check.hooks_configured && check.connected);
}

#[test]
#[serial]
fn file_facts_reports_metadata_only_with_tilde_path() {
    std::env::set_var("HOME", "/tmp/pokepoke-diag-home");
    let dir = std::path::Path::new("/tmp/pokepoke-diag-home/.pokepoke");
    std::fs::create_dir_all(dir).unwrap();
    let f = dir.join("sessions.json");
    std::fs::write(&f, b"hello").unwrap();

    let facts = file_facts(&f);
    assert!(facts.exists);
    assert_eq!(facts.size_bytes, Some(5));
    assert!(facts.modified.is_some());
    // 路径必须 tilde 折叠,不暴露 HOME
    assert_eq!(facts.path, "~/.pokepoke/sessions.json");

    let missing = file_facts(&dir.join("nope.json"));
    assert!(!missing.exists);
    assert_eq!(missing.size_bytes, None);
}

#[test]
#[serial]
fn recent_log_warnings_filters_levels_redacts_home_and_tails() {
    std::env::set_var("HOME", "/Users/bob");
    let dir = std::env::temp_dir().join("pokepoke-diag-log");
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("pokepoke.log");
    let content = "\
2026-06-18T10:00:00+08:00 [INFO] [lifecycle] hook deployed to /Users/bob/.local/bin/poke-hook
2026-06-18T10:00:01+08:00 [DEBUG] [panel] focused=true
2026-06-18T10:00:02+08:00 [WARN] [lifecycle] drift detected for cursor at /Users/bob/.cursor/hooks.json
2026-06-18T10:00:04+08:00 [ERROR] [lifecycle] deploy failed dst=/Users/bob/.local/bin/poke-hook: EACCES
";
    std::fs::write(&log, content).unwrap();

    let lines = recent_log_warnings(&log, 80);
    assert_eq!(lines.len(), 2, "only WARN+ERROR taken: {lines:?}");
    let joined = lines.join("\n");
    // INFO / DEBUG 行不取
    assert!(!joined.contains("hook deployed"), "INFO leaked: {joined}");
    assert!(!joined.contains("focused=true"), "DEBUG leaked: {joined}");
    // HOME 折叠,不暴露真实用户名
    assert!(!joined.contains("/Users/bob"), "HOME leaked: {joined}");
    assert!(
        joined.contains("~/.cursor/hooks.json"),
        "redacted path missing: {joined}"
    );
    assert!(
        joined.contains("~/.local/bin/poke-hook"),
        "redacted path missing: {joined}"
    );
}

#[test]
#[serial]
fn recent_log_warnings_collapses_repeated_messages() {
    std::env::set_var("HOME", "/Users/bob");
    let dir = std::env::temp_dir().join("pokepoke-diag-log-dedup");
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("pokepoke.log");
    // 3 条同签名(只时间戳不同)+ 1 条独立
    let body = "\
2026-06-18T10:00:00+08:00 [WARN] [transport] UDS already owned — skipping bind
2026-06-18T10:00:05+08:00 [WARN] [transport] UDS already owned — skipping bind
2026-06-18T10:01:00+08:00 [WARN] [updater] check failed: network down
2026-06-18T10:02:00+08:00 [WARN] [transport] UDS already owned — skipping bind
";
    std::fs::write(&log, body).unwrap();

    let lines = recent_log_warnings(&log, 80);
    assert_eq!(lines.len(), 2, "repeated sig collapsed to one: {lines:?}");
    let collapsed = lines
        .iter()
        .find(|l| l.contains("UDS already owned"))
        .unwrap();
    assert!(
        collapsed.contains("[×3, first 2026-06-18T10:00:00+08:00]"),
        "count/first missing: {collapsed}"
    );
    // 折叠条保留的是最后一次出现的时间戳
    assert!(
        collapsed.starts_with("2026-06-18T10:02:00+08:00"),
        "should keep last occurrence ts: {collapsed}"
    );
    // 独立条不带计数后缀
    let single = lines.iter().find(|l| l.contains("check failed")).unwrap();
    assert!(
        !single.contains("[×"),
        "single line should not carry count: {single}"
    );
}

#[test]
#[serial]
fn recent_log_warnings_keeps_only_last_n() {
    std::env::set_var("HOME", "/Users/bob");
    let dir = std::env::temp_dir().join("pokepoke-diag-log-tail");
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("pokepoke.log");
    let body: String = (0..10)
        .map(|i| format!("2026-06-18T10:00:0{i}+08:00 [WARN] [t] line{i}\n"))
        .collect();
    std::fs::write(&log, body).unwrap();

    let lines = recent_log_warnings(&log, 3);
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("line7"));
    assert!(lines[2].contains("line9"));
}

#[test]
fn files_equal_detects_match_and_mismatch() {
    let dir = std::env::temp_dir().join("pokepoke-diag-eq");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a");
    let b = dir.join("b");
    let c = dir.join("c");
    std::fs::write(&a, b"same").unwrap();
    std::fs::write(&b, b"same").unwrap();
    std::fs::write(&c, b"diff").unwrap();

    assert_eq!(files_equal(&a, &b), Some(true));
    assert_eq!(files_equal(&a, &c), Some(false));
    assert_eq!(files_equal(&a, &dir.join("missing")), None);
}

#[test]
fn is_operational_log_line_matches_osascript_and_ghostty() {
    assert!(is_operational_log_line(
        "2026-07-16T10:00:00+08:00 [WARN] [ghostty] reconcile skip"
    ));
    assert!(is_operational_log_line(
        "2026-07-16T10:00:01+08:00 [INFO] [transport] osascript TIMEOUT op=enumerate"
    ));
    assert!(!is_operational_log_line(
        "2026-07-16T10:00:02+08:00 [INFO] [panel] sessions-updated"
    ));
}

#[test]
#[serial]
fn recent_operational_log_prefers_current_file_over_rotated_tail() {
    std::env::set_var("HOME", "/Users/bob");
    let dir = std::env::temp_dir().join("pokepoke-diag-oplog-priority");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let current = dir.join("pokepoke.log");
    let rotated = dir.join("pokepoke.log.1");
    let old_lines: String = (0..80)
            .map(|i| {
                format!(
                    "2026-07-28T10:00:{i:02}+08:00 [INFO] [popup] close_popup_by_label label=old-{i} in_list=true\n"
                )
            })
            .collect();
    std::fs::write(&rotated, old_lines).unwrap();
    std::fs::write(
            &current,
            "2026-07-29T16:00:00+08:00 [INFO] [notify] popup focus_gate task_id=t decision=show suppress_reason=-\n\
             2026-07-29T16:01:00+08:00 [INFO] [popup] close_popup_by_label label=popup-new in_list=true reason=frontend_dismiss\n",
        )
        .unwrap();

    let lines = recent_operational_log(&current, Some(&rotated), 2);
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].contains("popup focus_gate"),
        "current log should win: {lines:?}"
    );
    assert!(
        lines[1].contains("reason=frontend_dismiss"),
        "current close reason should appear: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("old-")),
        "rotated should not appear when current fills quota: {lines:?}"
    );
}

#[test]
fn is_operational_log_line_matches_popup_observability() {
    assert!(is_operational_log_line(
        "2026-07-29T10:00:00+08:00 [INFO] [notify] popup focus_gate task_id=t decision=show"
    ));
    assert!(is_operational_log_line(
            "2026-07-29T10:00:01+08:00 [INFO] [popup] close_popup_by_label label=popup-1 reason=focus_watcher"
        ));
    assert!(is_operational_log_line(
            "2026-07-29T10:00:02+08:00 [INFO] [notify] show popup SUPPRESSED task_id=t reason=already_focused"
        ));
    assert!(is_operational_log_line(
        "2026-07-29T10:00:03+08:00 [INFO] [notify] sound SUPPRESSED task_id=t reason=quiet_mode"
    ));
}

#[test]
fn build_analysis_hints_flags_popup_suppress_already_focused() {
    let mut metrics = RuntimeMetrics::new().snapshot(0);
    metrics
        .popup
        .suppress_by_reason
        .insert("already_focused".into(), 6);
    let hints = build_analysis_hints(600, &metrics, &[], &[], &[]);
    assert!(
        hints
            .iter()
            .any(|h| h.contains("popup_suppress_already_focused")),
        "expected popup suppress hint: {hints:?}"
    );
}

#[test]
fn build_analysis_hints_flags_short_uptime() {
    let metrics = RuntimeMetrics::new().snapshot(0);
    let hints = build_analysis_hints(60, &metrics, &[], &[], &[]);
    assert!(
        hints.iter().any(|h| h.contains("uptime_under_5min")),
        "expected short uptime hint: {hints:?}"
    );
}

fn sample_file_facts() -> FileFacts {
    FileFacts {
        path: "~/.pokepoke/test".into(),
        exists: false,
        size_bytes: None,
        modified: None,
    }
}

fn sample_integration_check() -> IntegrationCheck {
    IntegrationCheck {
        installed: false,
        hooks_configured: false,
        connected: false,
        hooks_outdated: None,
        feature_enabled: None,
        issue: None,
    }
}

fn sample_diagnostics_report() -> DiagnosticsReport {
    let hook_binary = HookBinaryInfo {
        deployed: sample_file_facts(),
        deployed_executable: false,
        bundled: sample_file_facts(),
        matches_bundled: None,
    };
    let metrics = RuntimeMetrics::new().snapshot(0);
    DiagnosticsReport {
        schema_version: SCHEMA_VERSION,
        generated_at: "2026-07-31T12:00:00+08:00".into(),
        app: AppInfo {
            version: "1.1.3".into(),
            os: "macOS".into(),
            arch: "aarch64".into(),
            debug_logging: false,
            dev_build: true,
        },
        runtime: RuntimeInfo {
            pokepoke_dir: "~/.pokepoke".into(),
            server_socket: sample_file_facts(),
            server_responding: true,
            snapshots_count: 0,
            data_files: vec![],
            recent_log_warnings: vec!["2026-07-31 [WARN] test".into()],
            rotated_log: None,
            recent_operational_log: vec![
                "2026-07-31 [INFO] [notify] popup focus_gate task_id=t decision=show".into(),
            ],
        },
        health: HealthInfo {
            uptime_secs: 120,
            active_popup_count: 0,
            metrics,
            recent_events: vec![],
            analysis_hints: vec![],
        },
        integration: IntegrationInfo {
            hook_binary,
            cc: sample_integration_check(),
            codex: sample_integration_check(),
            cursor: sample_integration_check(),
            opencode: sample_integration_check(),
            pi: sample_integration_check(),
            kimi: sample_integration_check(),
            grok: sample_integration_check(),
            config_files: BTreeMap::new(),
            drift_outdated_sources: vec![],
        },
        settings: settings_digest(&Settings::default()),
        sessions: SessionsDigest {
            total: 0,
            by_status: StatusCounts::default(),
            by_source: BTreeMap::new(),
        },
        notes: vec!["test".into()],
    }
}

#[test]
fn diagnostics_report_json_shape_round_trip() {
    // schema v6(brief 20260813-watcher-supervisor):health.metrics 增加 watchers
    // 健康表(此前 v5 已含 notify_decisions)。
    let report = sample_diagnostics_report();
    let json = serde_json::to_string_pretty(&report).expect("serialize report");
    let value: Value = serde_json::from_str(&json).expect("parse serialized report");

    assert_eq!(value["schema_version"], 6);

    for key in ["cc", "codex", "cursor", "opencode", "pi", "kimi", "grok"] {
        assert!(
            value["integration"].get(key).is_some(),
            "integration.{key} missing: {json}"
        );
    }

    assert!(value["health"]["metrics"]["popup"].is_object());
    assert!(value["health"]["metrics"]["popup"]["suppress_by_reason"].is_object());
    assert!(value["health"]["metrics"]["popup"]["close_by_reason"].is_object());
    assert!(value["health"]["metrics"]["popup"]["focus_gate_by_decision"].is_object());
    assert!(
        value["health"]["metrics"]["notify_decisions"].is_array(),
        "notify_decisions missing: {json}"
    );
    assert!(
        value["health"]["metrics"]["watchers"].is_object(),
        "watchers missing: {json}"
    );

    assert!(value["runtime"]["recent_operational_log"].is_array());
    assert_eq!(
        value["runtime"]["recent_operational_log"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    for key in ["sound_pending", "sound_idle", "sound_failed"] {
        assert!(
            value["settings"].get(key).is_some(),
            "settings.{key} missing: {json}"
        );
    }

    // rotated_log 为 None 时应被 skip_serializing_if 省略
    assert!(value["runtime"].get("rotated_log").is_none());
}
