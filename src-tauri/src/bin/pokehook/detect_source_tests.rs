use super::super::grok::clear_grok_hook_env;
use super::super::{detect_source, Source};
use serde_json::json;

#[test]
fn detect_source_payload_workspace_roots_overrides_arg() {
    clear_grok_hook_env();
    // Cursor third-party hooks 双触发场景：用户开启 third-party hooks 后，
    // Cursor 也会读 ~/.claude/settings.json 里的 `--source=cc` 命令，但实际
    // 触发的事件是 cursor 的（payload 含 workspace_roots）。强信号必须
    // override arg，否则 Cursor 事件会被错分成 cc。
    let payload = json!({"workspace_roots": ["/tmp/a"]});
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &payload),
        Source::Cursor
    );
    assert_eq!(detect_source(Some(Source::Codex), &payload), Source::Cursor);
    assert_eq!(detect_source(None, &payload), Source::Cursor);

    let payload_camel = json!({"workspaceRoots": ["/tmp/a"]});
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &payload_camel),
        Source::Cursor
    );
}

#[test]
fn detect_source_payload_turn_id_overrides_arg() {
    clear_grok_hook_env();
    // Codex 的非 SessionStart 事件都带 turn_id；强信号 override arg。
    let payload = json!({"turn_id": "t1"});
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &payload),
        Source::Codex
    );
    assert_eq!(detect_source(None, &payload), Source::Codex);
}

#[test]
fn detect_source_uses_arg_when_no_strong_payload_signal() {
    clear_grok_hook_env();
    let payload = json!({"session_id": "abc", "hook_event_name": "Stop"});
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &payload),
        Source::ClaudeCode
    );
    assert_eq!(detect_source(Some(Source::Codex), &payload), Source::Codex);
    assert_eq!(
        detect_source(Some(Source::Cursor), &payload),
        Source::Cursor
    );
}

#[test]
fn detect_source_falls_back_to_claude_code_when_arg_missing() {
    clear_grok_hook_env();
    // 没有强信号、也没有 arg 时（旧 hook 命令残留 / 用户手工配的）：
    // 走最常见的 ClaudeCode 兜底。本地 reinstall 后此分支不再被命中。
    let payload = json!({"session_id": "abc", "hook_event_name": "Stop"});
    assert_eq!(detect_source(None, &payload), Source::ClaudeCode);
}

#[test]
fn detect_source_no_longer_misclassifies_cc_stop_via_stop_hook_active_or_model() {
    clear_grok_hook_env();
    // 历史 bug 回归测试（详见 docs/briefs/20260421-detect-source-by-arg.md）：
    // cc Stop payload 同时带 stop_hook_active 和 model，旧 detect_source 会
    // 因为这两条字段嗅探规则把 cc 错分成 codex，导致幽灵 codex 会话 + popup
    // 误关。新设计完全不再嗅探这两个字段。
    let cc_stop = json!({
        "session_id": "cc-session-id",
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "model": "claude-opus-4-7",
    });
    assert_eq!(
        detect_source(Some(Source::ClaudeCode), &cc_stop),
        Source::ClaudeCode
    );
    // 即使 arg 缺失，也走 ClaudeCode 兜底而不是 Codex
    assert_eq!(detect_source(None, &cc_stop), Source::ClaudeCode);
}

#[test]
fn detect_source_treats_cc_session_start_source_field_as_claude_code() {
    clear_grok_hook_env();
    // CC 的 SessionStart payload 也带 source: startup/resume/clear/compact，
    // 历史上曾因此误判 → 已通过 arg-based 设计根除（见 detect_source 注释）。
    for source_val in ["startup", "resume", "clear", "compact"] {
        let payload = json!({
            "session_id": "abcdef123456",
            "cwd": "/tmp/proj",
            "hook_event_name": "SessionStart",
            "source": source_val,
        });
        assert_eq!(
            detect_source(Some(Source::ClaudeCode), &payload),
            Source::ClaudeCode,
            "cc SessionStart with source={source_val} should be ClaudeCode",
        );
    }
}
