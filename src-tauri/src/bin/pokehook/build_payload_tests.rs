//! pokehook 单测:build_payload(从 tests.rs 整段抽出,编排者批准,brief
//! 20260813-protocol-version 门禁处理)。共享 helper(with_env / with_envs /
//! json)经 `super::tests` 引用。

// 本文件挂在 tests.rs 内部(#[path] mod build_payload_tests),`super` 即
// tests 模块本身;build_payload / Source 经 super 再上溯到 pokehook 模块。
use super::build_payload;
use super::{with_env, with_envs, Source};
use serde_json::json;

#[test]
fn build_payload_session_start_includes_terminal_app_when_env_set() {
    let data = json!({
        "session_id": "abc",
        "cwd": "/Users/tester/proj"
    });
    // 用 with_env 模拟 ghostty 启动的 shell
    with_env("TERM_PROGRAM", Some("ghostty"), || {
        let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
        assert_eq!(payload["terminal_app"], "ghostty");
    });
}

#[test]
fn build_payload_session_start_lowercases_iterm_program() {
    let data = json!({"session_id": "x", "cwd": "/tmp"});
    with_env("TERM_PROGRAM", Some("iTerm.app"), || {
        let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
        // 大小写归一,server 端可稳定比较
        assert_eq!(payload["terminal_app"], "iterm.app");
    });
}

#[test]
fn build_payload_session_start_recognizes_warp_terminal_program() {
    let data = json!({"session_id": "x", "cwd": "/tmp"});
    with_envs(
        &[
            ("TERM_PROGRAM", Some("WarpTerminal")),
            ("WARP_IS_LOCAL_SHELL_SESSION", None),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            // 小写归一化 + Warp 官方同名信号(TERM_PROGRAM=WarpTerminal)
            assert_eq!(payload["terminal_app"], "warpterminal");
        },
    );
}

#[test]
fn build_payload_session_start_warp_fallback_env_without_term_program() {
    // cc daemon spawn 丢 TERM_PROGRAM 的场景:WARP_IS_LOCAL_SHELL_SESSION=1 兜底
    let data = json!({"session_id": "x", "cwd": "/tmp"});
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("xterm-256color")),
            ("GHOSTTY_RESOURCES_DIR", None),
            ("GHOSTTY_SHELL_FEATURES", None),
            ("WARP_IS_LOCAL_SHELL_SESSION", Some("1")),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["terminal_app"], "warpterminal");
        },
    );
}

#[test]
fn build_payload_warp_signal_loses_to_term_program() {
    // 优先级契约:TERM_PROGRAM 是第一优先级,即便 WARP 兜底信号也在场
    let data = json!({"session_id": "x", "cwd": "/tmp"});
    with_envs(
        &[
            ("TERM_PROGRAM", Some("iTerm.app")),
            ("WARP_IS_LOCAL_SHELL_SESSION", Some("1")),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["terminal_app"], "iterm.app");
        },
    );
}

#[test]
fn build_payload_session_start_omits_terminal_app_when_env_unset() {
    let data = json!({"session_id": "x", "cwd": "/tmp"});
    // 把所有 terminal_app 信号源都拿掉,模拟无识别能力的 env
    // (跑测试的 host 可能就是 Ghostty,fallback 信号会命中,要显式清掉)
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("xterm-256color")),
            ("GHOSTTY_RESOURCES_DIR", None),
            ("GHOSTTY_SHELL_FEATURES", None),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert!(
                payload.get("terminal_app").is_none(),
                "所有 terminal_app 信号都缺时不该输出该字段"
            );
        },
    );
}

#[test]
fn build_payload_cc_codex_events_include_terminal_app_when_env_set() {
    // brief 20260527-ghostty-capture-retry:b20520 的"其它事件不带"被推翻,
    // 现在所有 cc/codex 事件都带,让 server 端能在它们上面补幂等 capture。
    let data = json!({
        "session_id": "x",
        "cwd": "/tmp",
        "message": "msg"
    });
    with_env("TERM_PROGRAM", Some("ghostty"), || {
        for (event, source) in [
            ("UserPromptSubmit", Source::ClaudeCode),
            ("Notification", Source::ClaudeCode),
            ("PermissionRequest", Source::ClaudeCode),
            ("Stop", Source::ClaudeCode),
            ("StopFailure", Source::ClaudeCode),
            ("UserPromptSubmit", Source::Codex),
            ("PermissionRequest", Source::Codex),
            ("Stop", Source::Codex),
        ] {
            let payload = build_payload(event, &data, source, None);
            if let Some(p) = payload {
                assert_eq!(
                    p["terminal_app"], "ghostty",
                    "{}@{:?} 应携带 terminal_app=ghostty 让 server 端识别 Ghostty session",
                    event, source
                );
            }
        }
    });
}

#[test]
fn build_payload_cursor_events_never_include_terminal_app() {
    // Cursor 一般不在 Ghostty 里跑(Cursor 有内嵌终端),server 端也不对
    // cursor session 触发 ghostty capture,所以 cursor 事件不带 terminal_app
    // 避免 schema 噪声。Stop@Cursor 和 SessionEnd 都跳过。
    let data = json!({
        "conversation_id": "c",
        "workspace_roots": ["/tmp"],
        "message": "m"
    });
    with_env("TERM_PROGRAM", Some("ghostty"), || {
        for event in ["Stop", "SessionEnd"] {
            let payload = build_payload(event, &data, Source::Cursor, None);
            if let Some(p) = payload {
                assert!(
                    p.get("terminal_app").is_none(),
                    "{}@Cursor 不该携带 terminal_app",
                    event
                );
            }
        }
    });
}

#[test]
fn build_payload_maps_cursor_stop_to_idle() {
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"],
        "hookStatus": "completed"
    });

    let payload = build_payload("Stop", &data, Source::Cursor, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["source"], "cursor");
    assert_eq!(payload["message"], "Agent turn completed");
    assert_eq!(payload["task_id"], "cursor-cursor-s");
    assert_eq!(payload["external_session_id"], "cursor-session-abcdefgh");
    // T1:completed 不带 aborted 标记
    assert!(payload.get("aborted").is_none());
}

#[test]
fn build_payload_maps_cursor_stop_error_to_last_failed() {
    // integration-gaps P2-①:hookStatus=error 曾映射 idle 播"完成"音。
    // 现在走 server 现有 last_failed 链路(红色 popup + failed 音)。
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"],
        "hookStatus": "error"
    });

    let payload = build_payload("Stop", &data, Source::Cursor, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "last_failed");
    assert_eq!(payload["failure_reason"], "api_error");
    assert_eq!(payload["message"], "Agent turn ended with error");
    // T1:error 分支不带 aborted 标记
    assert!(payload.get("aborted").is_none());
}

#[test]
fn build_payload_cursor_stop_missing_hook_status_is_not_completed() {
    // grok review §2.4:hookStatus 缺失曾默认 completed,把 error 报成"完成"。
    // 现在缺失落入 last_failed 分支,status 非 idle 且 message 非 completed。
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"]
    });

    let payload = build_payload("Stop", &data, Source::Cursor, None).unwrap();
    assert_eq!(payload["status"], "last_failed");
    assert_ne!(payload["status"], "idle");
    assert_ne!(payload["message"], "Agent turn completed");
    assert_eq!(payload["failure_reason"], "api_error");
}

#[test]
fn build_payload_maps_cursor_stop_aborted_to_idle() {
    // aborted(用户中止)不是失败,保持 idle + 独立文案。
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"],
        "hookStatus": "aborted"
    });

    let payload = build_payload("Stop", &data, Source::Cursor, None).unwrap();
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["message"], "Agent turn aborted");
    assert!(payload.get("failure_reason").is_none());
    // T1:aborted 带标记,server 据此写 aborted_at + 抑制"完成"音
    assert_eq!(payload["aborted"], true);
}

#[test]
fn build_payload_stop_suppressed_when_background_subagent_running() {
    // cc Stop payload 自带 background_tasks:有 running 项 = agent 还在等异步结果,
    // 这次 Stop 只是 turn 边界 → 丢掉,不弹"完成"。详见 project_stop_background_tasks_signal。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop",
        "background_tasks": [
            {"id": "sub-1", "type": "subagent", "status": "running", "description": "implementer"}
        ]
    });
    assert!(build_payload("Stop", &data, Source::ClaudeCode, None).is_none());
}

#[test]
fn build_payload_stop_suppressed_when_background_shell_running() {
    // run_in_background 的 Bash 也进 background_tasks(type=shell),同样要抑制。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop",
        "background_tasks": [
            {"id": "shell-1", "type": "shell", "status": "running", "command": "sleep 90"}
        ]
    });
    assert!(build_payload("Stop", &data, Source::ClaudeCode, None).is_none());
}

#[test]
fn build_payload_stop_suppressed_when_mixed_running_and_completed() {
    // 只要有一个 running 就抑制(completed 项不影响判定)。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop",
        "background_tasks": [
            {"id": "sub-1", "type": "subagent", "status": "completed"},
            {"id": "sub-2", "type": "subagent", "status": "running"}
        ]
    });
    assert!(build_payload("Stop", &data, Source::ClaudeCode, None).is_none());
}

#[test]
fn build_payload_stop_sent_when_background_tasks_all_completed() {
    // 全 completed = 没有在跑的活 → 照常发 idle 弹窗(真完成)。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop",
        "background_tasks": [
            {"id": "sub-1", "type": "subagent", "status": "completed"}
        ]
    });
    let payload = build_payload("Stop", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload["status"], "idle");
}

#[test]
fn build_payload_stop_sent_when_background_tasks_empty_or_absent() {
    // 空数组 / 字段缺失 = 没有后台活 → 照常发 idle(现状行为,不回归)。
    let empty = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop",
        "background_tasks": []
    });
    assert_eq!(
        build_payload("Stop", &empty, Source::ClaudeCode, None).unwrap()["status"],
        "idle"
    );

    let absent = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "Stop"
    });
    assert_eq!(
        build_payload("Stop", &absent, Source::ClaudeCode, None).unwrap()["status"],
        "idle"
    );
}

#[test]
fn build_payload_maps_claude_stop_failure_with_reason() {
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "reason": "rate_limit"
    });

    let payload = build_payload(
        "StopFailure",
        &data,
        Source::ClaudeCode,
        Some("/dev/ttys001".into()),
    )
    .unwrap();

    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "last_failed");
    assert_eq!(payload["failure_reason"], "rate_limit");
    assert_eq!(payload["source"], "claude-code");
    assert_eq!(payload["terminal_tty"], "/dev/ttys001");
    assert_eq!(payload["external_session_id"], "abcdef123456");
}

#[test]
fn build_payload_maps_claude_stop_failure_with_error_field() {
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "hook_event_name": "StopFailure",
        "error": "invalid_request",
        "error_details": "400 Bad Request",
        "last_assistant_message": "API Error: invalid model"
    });

    let payload = build_payload("StopFailure", &data, Source::ClaudeCode, None).unwrap();

    assert_eq!(payload["failure_reason"], "invalid_request");
}

#[test]
fn build_payload_prefers_error_field_over_legacy_reason() {
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "error": "model_not_found",
        "reason": "unknown"
    });

    let payload = build_payload("StopFailure", &data, Source::ClaudeCode, None).unwrap();

    assert_eq!(payload["failure_reason"], "model_not_found");
}

#[test]
fn build_payload_maps_claude_stop_failure_detail_message() {
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "error": "unknown",
        "last_assistant_message": "API Error: System protection triggered by request burst"
    });

    let payload = build_payload("StopFailure", &data, Source::ClaudeCode, None).unwrap();

    assert_eq!(payload["failure_reason"], "unknown");
    assert_eq!(
        payload["message"],
        "API Error: System protection triggered by request burst"
    );
}

#[test]
fn build_payload_keeps_notification_message_for_claude_code() {
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "message": "Need permission"
    });

    let payload = build_payload("Notification", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload["event_type"], "pending");
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["message"], "Need permission");
    assert_eq!(payload["source"], "claude-code");
    assert_eq!(payload["external_session_id"], "abcdef123456");
}

#[test]
fn build_payload_drops_idle_prompt_notification_for_claude_code() {
    // cc 的 Notification hook 在 idle ≥60s 时会以
    // notification_type="idle_prompt" 触发"Claude is waiting for your input",
    // 这是上一轮 Idle 弹过 popup 之后的"还没输入哦"提醒,语义上不是 Pending,
    // 不该升级。验证 build_payload 直接 None,hook_mode 同款过滤。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "message": "Claude is waiting for your input",
        "notification_type": "idle_prompt"
    });

    assert!(build_payload("Notification", &data, Source::ClaudeCode, None).is_none());
}

#[test]
fn build_payload_codex_permission_request_maps_to_pending_with_tool_name() {
    let data = json!({
        "session_id": "codex-sess-abc",
        "cwd": "/Users/tester/proj-x",
        "tool_name": "shell"
    });
    let payload = build_payload("PermissionRequest", &data, Source::Codex, None).unwrap();
    assert_eq!(payload["event_type"], "pending");
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["source"], "codex");
    assert_eq!(payload["message"], "Awaiting approval: shell");
}

#[test]
fn build_payload_codex_permission_request_handles_camelcase_toolname() {
    // 防 Codex schema 漂移:tool_name / toolName 都识别
    let data = json!({
        "session_id": "codex-sess-abc",
        "cwd": "/Users/tester/proj-x",
        "toolName": "apply_patch"
    });
    let payload = build_payload("PermissionRequest", &data, Source::Codex, None).unwrap();
    assert_eq!(payload["message"], "Awaiting approval: apply_patch");
}

#[test]
fn build_payload_codex_permission_request_falls_back_when_tool_name_missing() {
    // tool_name 缺失时退化为通用文案,不该 None / panic
    let data = json!({
        "session_id": "codex-sess-abc",
        "cwd": "/Users/tester/proj-x"
    });
    let payload = build_payload("PermissionRequest", &data, Source::Codex, None).unwrap();
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["message"], "Awaiting approval");
}

#[test]
fn build_payload_permission_request_for_cc_and_cursor() {
    // CC(2026-08-19 起订阅)走 PermissionRequest → Pending;Cursor 仍无此事件。
    let data = json!({
        "session_id": "x",
        "cwd": "/tmp",
        "tool_name": "Bash"
    });
    let cc = build_payload("PermissionRequest", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(cc["status"], "pending");
    assert_eq!(cc["message"], "Awaiting approval: Bash");
    assert!(build_payload("PermissionRequest", &data, Source::Cursor, None).is_none());
}

#[test]
fn build_payload_keeps_other_notification_types_for_claude_code() {
    // 非 idle_prompt 的 notification_type(如 permission)仍然走 Pending,
    // 防止误吞真权限请求。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "message": "Bash command needs approval",
        "notification_type": "permission"
    });

    let payload = build_payload("Notification", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["message"], "Bash command needs approval");
}

#[test]
fn build_payload_keeps_notification_when_type_field_missing() {
    // 老版本 cc 没有 notification_type 字段,过滤逻辑应放行而不是误吞。
    let data = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-a",
        "message": "Need permission"
    });

    let payload = build_payload("Notification", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload["status"], "pending");
}

#[test]
fn build_payload_maps_codex_stop_to_idle_and_prefixes_task_id() {
    let data = json!({
        "session_id": "codexsession1234",
        "cwd": "/Users/tester/proj-c"
    });

    let payload = build_payload("Stop", &data, Source::Codex, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["task_id"], "codex-codexses");
    assert_eq!(payload["title"], "proj-c");
    assert_eq!(payload["external_session_id"], "codexsession1234");
}

#[test]
fn build_payload_marks_session_end_as_remove_signal() {
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"]
    });

    let payload = build_payload("SessionEnd", &data, Source::Cursor, None).unwrap();
    assert_eq!(payload["event_type"], "session_end");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["external_session_id"], "cursor-session-abcdefgh");
}

#[test]
fn build_payload_cursor_stop_carries_terminal_tty() {
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"],
        "hookStatus": "completed"
    });

    let payload =
        build_payload("Stop", &data, Source::Cursor, Some("/dev/ttys008".into())).unwrap();
    assert_eq!(payload["source"], "cursor");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["terminal_tty"], "/dev/ttys008");
}

#[test]
fn build_payload_cursor_session_end_carries_terminal_tty() {
    let data = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"]
    });

    let payload = build_payload(
        "SessionEnd",
        &data,
        Source::Cursor,
        Some("/dev/ttys009".into()),
    )
    .unwrap();
    assert_eq!(payload["event_type"], "session_end");
    assert_eq!(payload["terminal_tty"], "/dev/ttys009");
}

#[test]
fn build_payload_maps_opencode_notification_to_pending() {
    // plugin 把 opencode permission.updated 归一成 Notification + 权限标题
    let data = json!({
        "session_id": "ses_abcdefgh",
        "cwd": "/Users/tester/proj-oc",
        "message": "Awaiting approval: bash"
    });

    let payload = build_payload("Notification", &data, Source::OpenCode, None).unwrap();
    assert_eq!(payload["event_type"], "pending");
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["source"], "opencode");
    assert_eq!(payload["message"], "Awaiting approval: bash");
    assert_eq!(payload["task_id"], "opencode-ses_abcd");
}

#[test]
fn build_payload_maps_opencode_session_error_to_last_failed() {
    // plugin 把 opencode session.error 归一成 StopFailure + reason=error 类型
    let data = json!({
        "session_id": "ses_abcdefgh",
        "cwd": "/Users/tester/proj-oc",
        "reason": "ProviderAuthError"
    });

    let payload = build_payload("StopFailure", &data, Source::OpenCode, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "last_failed");
    assert_eq!(payload["failure_reason"], "ProviderAuthError");
    assert_eq!(payload["source"], "opencode");
}

#[test]
fn build_payload_opencode_events_include_terminal_app_when_ghostty() {
    // opencode 在终端(含 Ghostty)里跑,与 cc/codex 一样带 terminal_app 以支持
    // 精准跳转(此前误当作 cursor GUI 排除,导致回跳退化到 L2 cwd 匹配)。
    with_env("TERM_PROGRAM", Some("ghostty"), || {
        for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
            let data = json!({
                "session_id": "ses_abcdefgh",
                "cwd": "/Users/tester/proj-oc"
            });
            let payload = build_payload(event, &data, Source::OpenCode, None).unwrap();
            assert_eq!(
                payload.get("terminal_app").and_then(|v| v.as_str()),
                Some("ghostty"),
                "opencode {} should carry terminal_app=ghostty",
                event
            );
        }
    });
}

#[test]
fn build_payload_carries_cmux_env_when_present() {
    with_envs(
        &[
            ("CMUX_SURFACE_ID", Some("SURF-XYZ")),
            ("CMUX_WORKSPACE_ID", Some("WS-XYZ")),
            ("TERM_PROGRAM", Some("ghostty")),
        ],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(
                payload["cmux_surface_id"].as_str(),
                Some("SURF-XYZ"),
                "SessionStart 应携带 cmux_surface_id"
            );
            assert_eq!(payload["cmux_workspace_id"].as_str(), Some("WS-XYZ"));
            // terminal_app 短路成 cmux,不再是 ghostty
            assert_eq!(payload["terminal_app"].as_str(), Some("cmux"));
        },
    );
}

#[test]
fn build_payload_omits_cmux_fields_when_env_missing() {
    with_envs(
        &[
            ("CMUX_SURFACE_ID", None),
            ("CMUX_WORKSPACE_ID", None),
            ("TERM_PROGRAM", Some("iTerm.app")),
        ],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert!(payload.get("cmux_surface_id").is_none());
            assert!(payload.get("cmux_workspace_id").is_none());
        },
    );
}

#[test]
fn build_payload_herdr_env_suppresses_cmux_even_when_cmux_env_present() {
    with_envs(
        &[
            ("HERDR_PANE_ID", Some("w1:p1")),
            ("HERDR_WORKSPACE_ID", Some("w1")),
            ("HERDR_SOCKET_PATH", Some("/tmp/work/herdr.sock")),
            ("CMUX_SURFACE_ID", Some("SURF-HERDR-AGENT")),
            ("CMUX_WORKSPACE_ID", Some("WS-CMUX")),
        ],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["herdr_pane_id"].as_str(), Some("w1:p1"));
            assert!(payload.get("cmux_surface_id").is_none());
            assert!(payload.get("cmux_workspace_id").is_none());
        },
    );
}

#[test]
fn build_payload_wezterm_env_when_pane_and_socket_present() {
    with_envs(
        &[
            ("WEZTERM_PANE", Some("7")),
            ("WEZTERM_UNIX_SOCKET", Some("/tmp/wezterm.sock")),
            (
                "WEZTERM_EXECUTABLE",
                Some("/Applications/WezTerm.app/Contents/MacOS/wezterm"),
            ),
        ],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["wezterm_pane_id"].as_str(), Some("7"));
            assert_eq!(
                payload["wezterm_unix_socket"].as_str(),
                Some("/tmp/wezterm.sock")
            );
            assert_eq!(
                payload["wezterm_executable"].as_str(),
                Some("/Applications/WezTerm.app/Contents/MacOS/wezterm")
            );
        },
    );
}

#[test]
fn build_payload_wezterm_env_requires_socket() {
    with_envs(
        &[("WEZTERM_PANE", Some("7")), ("WEZTERM_UNIX_SOCKET", None)],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert!(payload.get("wezterm_pane_id").is_none());
            assert!(payload.get("wezterm_unix_socket").is_none());
        },
    );
}

#[test]
fn build_payload_cmux_env_suppresses_wezterm_even_when_wezterm_env_present() {
    with_envs(
        &[
            ("CMUX_SURFACE_ID", Some("SURF-1")),
            ("CMUX_WORKSPACE_ID", Some("WS-1")),
            ("WEZTERM_PANE", Some("7")),
            ("WEZTERM_UNIX_SOCKET", Some("/tmp/wezterm.sock")),
        ],
        || {
            let data = json!({
                "session_id": "abc12345",
                "cwd": "/tmp/proj"
            });
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["cmux_surface_id"].as_str(), Some("SURF-1"));
            assert!(payload.get("wezterm_pane_id").is_none());
            assert!(payload.get("wezterm_unix_socket").is_none());
        },
    );
}

#[test]
fn build_payload_tmux_env_when_pane_and_socket_present() {
    let data = json!({"session_id": "x", "cwd": "/tmp/proj"});
    with_envs(
        &[
            ("TMUX", Some("/tmp/tmux-501/default,20745,3")),
            ("TMUX_PANE", Some("%12")),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["tmux_pane_id"].as_str(), Some("%12"));
            assert_eq!(
                payload["tmux_socket_path"].as_str(),
                Some("/tmp/tmux-501/default")
            );
        },
    );
}

#[test]
fn build_payload_omits_tmux_fields_when_env_missing() {
    let data = json!({"session_id": "x", "cwd": "/tmp/proj"});
    with_envs(&[("TMUX", None), ("TMUX_PANE", None)], || {
        let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
        assert!(payload.get("tmux_pane_id").is_none());
        assert!(payload.get("tmux_socket_path").is_none());
    });
}

#[test]
fn build_payload_herdr_env_suppresses_tmux_even_when_tmux_env_present() {
    let data = json!({"session_id": "x", "cwd": "/tmp/proj"});
    with_envs(
        &[
            ("HERDR_PANE_ID", Some("w1:p1")),
            ("HERDR_WORKSPACE_ID", Some("w1")),
            ("HERDR_SOCKET_PATH", Some("/tmp/work/herdr.sock")),
            ("TMUX", Some("/tmp/tmux-501/default,20745,3")),
            ("TMUX_PANE", Some("%12")),
        ],
        || {
            let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
            assert_eq!(payload["herdr_pane_id"].as_str(), Some("w1:p1"));
            assert!(payload.get("tmux_pane_id").is_none());
            assert!(payload.get("tmux_socket_path").is_none());
        },
    );
}

#[test]
fn build_payload_carries_protocol_version() {
    // W9(brief 20260813-protocol-version):/notify 出口必须带 protocol_version=1。
    let data = json!({"session_id": "abc", "cwd": "/Users/tester/proj"});
    let payload = build_payload("SessionStart", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload["protocol_version"], 1);
    // 非 SessionStart 事件同样带(公共出口)。
    let payload2 = build_payload("Stop", &data, Source::ClaudeCode, None).unwrap();
    assert_eq!(payload2["protocol_version"], 1);
}
