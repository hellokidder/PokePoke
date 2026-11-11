//! `build_payload` 与 source 门禁（从 pokehook.rs 抽出,见 AGENTS.md §4a）。

use super::*;
use serde_json::Value;

/// 与 server 端 `http_server::notify::PROTOCOL_VERSION` 对齐的 wire 协议版本。
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// `hook_mode` 与 `build_payload` 共用的 PermissionRequest source 门禁。
/// CC(2026-08-19)加入:CC 弹权限确认立即 fire PermissionRequest,是"等用户
/// 确认"的主信号;Notification.permission_prompt 只是 ~6s 后的兜底。Cursor 无
/// 此事件(beforeShellExecution 是拦截 hook,我们不订)。
/// omp(2026-09-23)加入:omp 原生 `tool_approval_requested` 语义同上。
pub(crate) fn accepts_permission_request_source(source: Source) -> bool {
    matches!(
        source,
        Source::ClaudeCode | Source::Codex | Source::Kimi | Source::Omp | Source::Dsh
    )
}

/// `hook_mode` 与 `build_payload` 共用的 StopFailure source 门禁。
/// pi(2026-08-04):扩展订阅 agent_end,agent_settled 时按 stopReason=error
/// 转发 StopFailure(reason=provider_error + detail 截断原文)。
/// omp(2026-09-23):同源于 pi 扩展,settle 事件换成 agent_end + willContinue。
pub(crate) fn accepts_stop_failure_source(source: Source) -> bool {
    matches!(
        source,
        Source::ClaudeCode
            | Source::OpenCode
            | Source::Kimi
            | Source::Grok
            | Source::Pi
            | Source::Omp
            | Source::Dsh
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_payload(
    event: &str,
    data: &Value,
    source: Source,
    tty: Option<String>,
) -> Option<Value> {
    let task_id = build_task_id(data, source);
    let external_session_id = external_session_id(data, source);
    let cwd = resolve_workspace_path(data, source);
    let project = project_name(&cwd);
    let mut payload = match event {
        "SessionStart" => serde_json::json!({
            "task_id": task_id,
            "event_type": "running",
            "title": project.to_string(),
            "message": format!("Session started\n{}", cwd),
            "source": source.as_str(),
            "status": "running",
            "workspace_path": cwd,
        }),
        "UserPromptSubmit" => serde_json::json!({
            "task_id": task_id,
            "event_type": "running",
            "title": project.to_string(),
            "message": format!("Working...\n{}", cwd),
            "source": source.as_str(),
            "status": "running",
            "workspace_path": cwd,
        }),
        "Notification" if matches!(source, Source::ClaudeCode | Source::OpenCode | Source::Dsh) => {
            // 与 hook_mode 同款过滤:notification_type=idle_prompt 是 cc 的
            // "idle ≥60s 提醒",不该升 Pending。详见 hook_mode `Notification` arm。
            if pick_notification_type(data).as_deref() == Some("idle_prompt") {
                return None;
            }
            serde_json::json!({
                "task_id": task_id,
                "event_type": "pending",
                "title": project.to_string(),
                "message": data["message"].as_str().unwrap_or(""),
                "source": source.as_str(),
                "status": "pending",
                "workspace_path": cwd,
            })
        }
        "Notification" if source == Source::Grok => {
            return super::grok::grok_pending_notification_payload(
                &task_id, &project, &cwd, data, source,
            );
        }
        "PermissionRequest" if accepts_permission_request_source(source) => {
            // Codex / Kimi / CC / omp 跑命令需要审批时触发(CC 的 PermissionRequest 与其
            // Notification.permission_prompt 同语义,但立即触发、是主信号)。
            // tool_name 是 Codex PermissionRequest 必含字段(参 codex hooks docs);
            // CC 同字段;驼峰 fallback 防 schema 漂移,缺失退化通用 "Awaiting approval"。
            // Kimi 字段名未真机确认,额外兜底 `tool` key。
            let tool_name = pick_str(data, &["tool_name", "toolName", "tool"]);
            let message = match tool_name.as_deref() {
                Some(name) if !name.is_empty() => format!("Awaiting approval: {}", name),
                _ => "Awaiting approval".to_string(),
            };
            serde_json::json!({
                "task_id": task_id,
                "event_type": "pending",
                "title": project.to_string(),
                "message": message,
                "source": source.as_str(),
                "status": "pending",
                "workspace_path": cwd,
            })
        }
        "Stop" if source == Source::Cursor => {
            // hookStatus 缺失/未知不再默认 completed(曾把 error 报成"完成",
            // grok review §2.4):completed/aborted → idle;其余(含缺失)→
            // last_failed + failure_reason=api_error,走 server 现有
            // last_failed 链路(红色 popup + failed 音,见 integration-gaps P2-①)。
            let hook_status = pick_str(data, &["status", "hookStatus"]).unwrap_or_default();
            let (status, msg) = match hook_status.as_str() {
                "completed" => ("idle", "Agent turn completed"),
                "aborted" => ("idle", "Agent turn aborted"),
                _ => ("last_failed", "Agent turn ended with error"),
            };
            let mut payload = serde_json::json!({
                "task_id": task_id,
                "event_type": "stop",
                "title": project.to_string(),
                "message": msg,
                "source": "cursor",
                "status": status,
                "workspace_path": cwd,
            });
            if status == "last_failed" {
                payload["failure_reason"] = Value::String("api_error".into());
            }
            // T1(brief 20260805-t1-cursor-aborted):aborted(用户取消)带标记,
            // server 据此写 aborted_at + 抑制"完成"音。completed/error 不带。
            if hook_status == "aborted" {
                payload["aborted"] = Value::Bool(true);
            }
            payload
        }
        "Stop" => {
            if source == Source::Grok && !super::grok::grok_stop_is_turn_end(data) {
                return None;
            }
            // 后台仍有任务在跑(子 agent / 后台 shell):Stop 只是 turn 边界、不是完成。
            // 丢掉这条事件 → server 端 session 保持 Running(由前序 UPS/SessionStart 设),
            // 不弹"完成"。background_tasks 缺失/全完成 → 照常发 idle 弹窗。
            if has_running_background_tasks(data) {
                return None;
            }
            let mut payload = serde_json::json!({
                "task_id": task_id,
                "event_type": "stop",
                "title": project.to_string(),
                "message": "Session completed",
                "source": source.as_str(),
                "status": "idle",
                "workspace_path": cwd,
            });
            // T1(brief 20260805-t1-cursor-aborted)扩展到 pi(brief 20260820-pi-aborted-no-completion-sound):
            // pi 扩展在用户取消时随 Stop 上报 aborted:true,透传给 server 抑制"完成"音。
            // 其它 source(CC/Codex/opencode/kimi/grok)目前不带,缺省 false 不输出字段。
            if data.get("aborted").and_then(|v| v.as_bool()) == Some(true) {
                payload["aborted"] = Value::Bool(true);
            }
            payload
        }
        "StopFailure" if accepts_stop_failure_source(source) => {
            // Kimi 的 StopFailure payload 字段未真机确认;pick_* 多 key 兜底 + 默认值,
            // 字段缺失时退化为 "unknown" / 空 detail,不阻塞弹窗。
            let reason = pick_stop_failure_reason(data);
            let detail = pick_stop_failure_detail(data);
            serde_json::json!({
                "task_id": task_id,
                "event_type": "stop",
                "title": project.to_string(),
                "message": detail,
                "source": source.as_str(),
                "status": "last_failed",
                "workspace_path": cwd,
                "failure_reason": reason,
            })
        }
        "SessionEnd" => serde_json::json!({
            "task_id": task_id,
            "event_type": "session_end",
            "title": project.to_string(),
            "message": "Session ended",
            "source": source.as_str(),
            "status": "idle",
            "workspace_path": cwd,
        }),
        _ => return None,
    };

    // DSH 是 GUI/Web 宿主,插件进程的祖先 tty 不是会话宿主;不采 tty/binding。
    if source != Source::Dsh {
        if let Some(tty) = tty {
            payload["terminal_tty"] = Value::String(tty);
        }
    }
    if source == Source::Dsh {
        if let Some(sock) = super::dsh::nav_socket_from_env() {
            payload["dsh_nav_socket"] = Value::String(sock);
        }
    }
    if source == Source::Dsh {
        // 不采 host-agent 环境(herdr/cmux/tmux),见 events.rs 同名说明。
    } else if let Some((pane, ws, sock)) = collect_herdr_env() {
        payload["herdr_pane_id"] = Value::String(pane);
        payload["herdr_workspace_id"] = Value::String(ws);
        payload["herdr_socket_path"] = Value::String(sock);
    } else if let Some((surface, ws)) = collect_cmux_env() {
        // herdr agent 在 herdr 内层 pty,外层宿主由 herdr client 决定;
        // 不与 cmux binding 混报(否则 iTerm2+herdr 会被残留 CMUX_* 拽去 cmux)。
        payload["cmux_surface_id"] = Value::String(surface);
        payload["cmux_workspace_id"] = Value::String(ws);
    } else if let Some((pane, sock, executable)) = collect_wezterm_env() {
        payload["wezterm_pane_id"] = Value::String(pane);
        payload["wezterm_unix_socket"] = Value::String(sock);
        if let Some(exe) = executable {
            payload["wezterm_executable"] = Value::String(exe);
        }
    } else if let Some(tmux) = tmux_env::collect_tmux_env() {
        payload["tmux_pane_id"] = Value::String(tmux.pane_id);
        payload["tmux_socket_path"] = Value::String(tmux.socket_path);
    }
    if let Some(external_session_id) = external_session_id {
        payload["external_session_id"] = Value::String(external_session_id);
    }
    // cc/codex 的 hook payload 自带 transcript_path
    // (cursor 用 camelCase 兜底),pokepoke 推送时用它读 jsonl 抽 session 摘要。
    // 任何事件透传,server 端 `Session.transcript_path` 仅在 Some(_) 时覆盖。
    if let Some(tp) = pick_str(data, &["transcript_path", "transcriptPath"]) {
        payload["transcript_path"] = Value::String(tp);
    }
    // brief 20260527-ghostty-capture-retry:`terminal_app` 字段从 SessionStart-only
    // (b20520)放开到**所有 cc/codex 事件**(UserPromptSubmit / Notification /
    // PermissionRequest / Stop / StopFailure)。server 端在 UserPromptSubmit /
    // Pending / Stop 上对 terminal_id 仍为 None 的 ghostty session 补幂等 capture,
    // 把 L1 命中率从 72% 拉到 ~95%。opencode 在终端(含 Ghostty)里跑,与 cc/codex
    // 同样带 terminal_app 以支持精准跳转;仅 Cursor(GUI,无 TTY)不带。
    if !matches!(source, Source::Cursor | Source::Dsh) {
        if let Some(ta) = terminal_app() {
            payload["terminal_app"] = Value::String(ta);
        }
    }
    // W9(brief 20260813-protocol-version):wire 协议版本,server 端检测点。
    // 缺省视为 v1(legacy hook 不传零迁移);版本超前由 server 记 WARN 不拒收。
    payload["protocol_version"] = Value::from(PROTOCOL_VERSION);

    Some(payload)
}
