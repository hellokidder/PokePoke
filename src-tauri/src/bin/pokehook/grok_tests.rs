use super::super::grok::{
    clear_grok_hook_env, grok_hooks_configured, grok_hooks_document, grok_notification_is_pending,
    grok_pending_notification_payload, grok_stop_is_turn_end, GROK_HOOK_EVENTS,
};
use super::super::{
    accepts_stop_failure_source, build_payload, detect_source, expected_hook_command,
    external_session_id, normalize_event, parse_source_arg, Source,
};
use serde_json::json;

#[test]
fn parse_source_arg_recognizes_grok() {
    assert_eq!(
        parse_source_arg(&["--source=grok".into()]),
        Some(Source::Grok)
    );
    assert_eq!(Source::Grok.as_str(), "grok");
    assert_eq!(Source::Grok.task_id_prefix(), "grok");
}

#[test]
#[serial_test::serial]
fn detect_source_grok_hook_event_overrides_vendor_signals() {
    clear_grok_hook_env();
    unsafe {
        std::env::set_var("GROK_HOOK_EVENT", "session_start");
    }
    let payload = json!({
        "workspace_roots": ["/tmp"],
        "turn_id": "turn-1",
        "sessionId": "abc12345",
    });
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &payload),
        Source::Grok
    );
    clear_grok_hook_env();
}

#[test]
fn normalize_event_maps_grok_snake_case() {
    assert_eq!(normalize_event("session_start"), "SessionStart");
    assert_eq!(normalize_event("user_prompt_submit"), "UserPromptSubmit");
    assert_eq!(normalize_event("notification"), "Notification");
    assert_eq!(normalize_event("stop_failure"), "StopFailure");
}

#[test]
fn grok_external_session_id_reads_session_id_camel_case() {
    let payload = json!({"sessionId": "abc12345"});
    assert_eq!(
        external_session_id(&payload, Source::Grok).as_deref(),
        Some("abc12345")
    );
}

#[test]
fn grok_build_payload_session_start_is_running() {
    let data = json!({"sessionId": "abc12345", "cwd": "/tmp/proj"});
    let out = build_payload("SessionStart", &data, Source::Grok, None).unwrap();
    assert_eq!(out["source"], "grok");
    assert_eq!(out["task_id"], "grok-abc12345");
}

#[test]
fn grok_notification_pending_only_for_permission_and_elicitation() {
    assert!(grok_notification_is_pending(
        &json!({"notificationType": "permission_prompt"})
    ));
    assert!(grok_notification_is_pending(
        &json!({"notificationType": "elicitation_dialog"})
    ));
    assert!(!grok_notification_is_pending(
        &json!({"notificationType": "task_complete"})
    ));
}

#[test]
fn grok_stop_only_end_turn_maps_idle() {
    let data = json!({"sessionId": "s1", "cwd": "/tmp", "reason": "end_turn"});
    let out = build_payload("Stop", &data, Source::Grok, None).unwrap();
    assert_eq!(out["status"], "idle");
    assert!(build_payload(
        "Stop",
        &json!({"sessionId": "s1", "cwd": "/tmp", "reason": "channel_closed"}),
        Source::Grok,
        None
    )
    .is_none());
}

#[test]
fn grok_stop_failure_is_last_failed() {
    let data = json!({
        "sessionId": "s1",
        "cwd": "/tmp",
        "error": "rate_limit",
        "lastAssistantMessage": "rate limited",
    });
    let out = build_payload("StopFailure", &data, Source::Grok, None).unwrap();
    assert_eq!(out["status"], "last_failed");
    assert_eq!(out["failure_reason"], "rate_limit");
}

#[test]
fn hook_stop_failure_source_gate_includes_grok() {
    assert!(accepts_stop_failure_source(Source::Grok));
}

#[test]
fn grok_hooks_document_contains_all_events() {
    let doc = grok_hooks_document("/bin/poke-hook --source=grok");
    for event in GROK_HOOK_EVENTS {
        assert!(doc["hooks"][event].is_array(), "missing event {event}");
    }
}

#[test]
fn grok_background_tasks_camel_case_suppresses_idle_stop() {
    let data = json!({
        "sessionId": "s1",
        "cwd": "/tmp",
        "reason": "end_turn",
        "backgroundTasks": [{"status": "running"}],
    });
    assert!(build_payload("Stop", &data, Source::Grok, None).is_none());
}

#[test]
fn grok_stop_is_turn_end_helper() {
    assert!(grok_stop_is_turn_end(&json!({"reason": "end_turn"})));
    assert!(!grok_stop_is_turn_end(&json!({"reason": "channel_closed"})));
    assert!(grok_stop_is_turn_end(&json!({})));
}

#[test]
fn grok_hooks_configured_accepts_fresh_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokepoke.json");
    let doc = grok_hooks_document(&expected_hook_command("grok"));
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(grok_hooks_configured(&path));
}

#[test]
fn grok_hooks_configured_false_when_event_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokepoke.json");
    let mut doc = grok_hooks_document(&expected_hook_command("grok"));
    doc["hooks"].as_object_mut().unwrap().remove("Stop");
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(!grok_hooks_configured(&path));
}

#[test]
fn grok_hooks_configured_false_for_unrelated_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokepoke.json");
    std::fs::write(&path, r#"{"description":"other","hooks":{}}"#).unwrap();
    assert!(!grok_hooks_configured(&path));
}

#[test]
fn grok_hooks_configured_false_for_malformed_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokepoke.json");
    std::fs::write(&path, "not json at all").unwrap();
    assert!(!grok_hooks_configured(&path));
}

#[test]
fn grok_hooks_configured_false_for_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokepoke.json");
    assert!(!grok_hooks_configured(&path));
}

#[test]
fn grok_pending_payload_carries_protocol_version() {
    // W9 review fix:grok 专用出口(绕开 build_payload 公共出口)同样带协议版本。
    let data = json!({
        "session_id": "g-1",
        "cwd": "/tmp/grok-proj",
        "notificationType": "permission_prompt",
        "message": "Awaiting your input",
    });
    let payload = grok_pending_notification_payload(
        "grok-1",
        "grok-proj",
        "/tmp/grok-proj",
        &data,
        super::Source::Grok,
    )
    .unwrap();
    assert_eq!(payload["protocol_version"], 1);
}
