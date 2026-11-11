//! `/notify` 全链路结构化决策 trace —— 每次 notify 一条 JSON line,落盘
//! `~/.pokepoke/notify-decisions.jsonl`(1MB cap 事件环),诊断导出可见。
//! 架构增强 W2,brief 20260813-decision-trace。
//!
//! 背景:每个 gate 的 SUPPRESSED 理由散落在 7+ 文件且是文本行日志,事后拼
//! 不出一次 /notify 的完整决策链。「为什么这条没弹」是第一支持问题——本模块
//! 把 process_notify 全链路每个判定点的结构化结果记成一条记录,排障直接查
//! 最近 N 条 trace。
//!
//! 硬约束(纯观测路径):
//! - 不参与任何 gate 判定,判定行为零变化;
//! - 追加 / 裁剪失败一律静默,绝不影响 /notify 主链路;
//! - 字段缺省值 =「该 gate 未走到」(serde default),不堆 Option。
//!
//! 字段语义:布尔字段 `true` = 该 gate 命中;字符串字段空 = 未走到,非空 =
//! 该 gate 的取值 / 抑制原因。`sound_played` 取值:`played` /
//! `skipped_refresh_only` / `not_played` / `suppressed:quiet_mode` /
//! `suppressed:aborted` / `suppressed:sound_cooldown` /
//! `suppressed:free_quota_exhausted`。

use serde::Serialize;

use super::notify::NotifyRequest;
use crate::runtime_metrics::{self, NOTIFY_DECISIONS_MAX_BYTES};
use crate::sessions::{SessionStatus, UpsertResult};

/// 一条 /notify 的完整决策记录。字段缺省 = 该 gate 未走到(serde default)。
#[derive(Debug, Default, Serialize)]
pub struct NotifyDecisionTrace {
    pub ts: String,
    pub task_id: String,
    pub session_id: String,
    pub event_type: String,
    pub status: String,
    pub prev_status: String,
    pub is_new: bool,

    // 逐 gate verdict(缺省 = 未走到)
    /// 集成断开 source 被拒(early_reject)。
    pub integration_disconnected: bool,
    /// 100ms (task_id, signature) 去重 short-circuit。
    pub dedupe_duplicate: bool,
    /// SessionEnd 独立分支(删 session + 关 popup)。
    pub session_end_handled: bool,
    /// stage eligible(stage-ending / Pending 提醒条件)。
    pub stage_eligible: bool,
    /// Codex 自动审批抑制 reason(空 = 未抑制)。
    pub codex_suppress_reason: String,
    pub already_focused: bool,
    pub focus_suppression_enabled: bool,
    pub quiet_mode_active: bool,
    pub gate_allow: bool,
    pub recently_visited: bool,
    pub stage_acked: bool,
    pub cooldown_available: bool,
    /// 呈现决策:show / suppress_popup_keep_sound / suppress_silent / skip。
    pub presentation: String,
    /// popup 抑制 reason(not_stage_eligible / already_focused / gate_off /
    /// recently_visited / stage_acked / cooldown)。
    pub suppress_reason: String,
    /// AttemptShow 前的 store 重查(race guard)。
    pub still_in_store: bool,
    /// state.popup.show() 是否真的建了窗口。
    pub popup_created: bool,
    /// 播音结果(含抑制原因,见模块注释取值表;空 = 未走到播音决策)。
    pub sound_played: String,
}

impl NotifyDecisionTrace {
    /// 在 `process_notify` 入口创建:先记请求级字段,gate 段由
    /// `popup_gate::evaluate_and_show_popup` 填入,`set_session_context` 在
    /// upsert 之后补 session 上下文。
    pub fn new(req: &NotifyRequest) -> Self {
        Self {
            ts: chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string(),
            task_id: req.task_id.clone(),
            event_type: req.event_type.clone().unwrap_or_default(),
            status: req.status.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// upsert 之后填入解析后的 session 上下文(status 为解析结果,与 wire 原
    /// 串可能不同——如 status 缺失时按 event_type 推导)。
    pub fn set_session_context(&mut self, result: &UpsertResult) {
        self.session_id = result.session.id.clone();
        self.status = status_str(&result.session.status).to_string();
        self.prev_status = result
            .prev_status
            .as_ref()
            .map(status_str)
            .unwrap_or_default()
            .to_string();
        self.is_new = result.is_new;
    }

    /// 落盘:序列化为 JSON line 追加到决策环文件(cap 内裁剪)。失败静默。
    /// `process_notify` 的**每个 return 点**都必须调用一次。
    pub fn finish(&self) {
        let Ok(line) = serde_json::to_string(self) else {
            return;
        };
        runtime_metrics::append_jsonl(
            &runtime_metrics::notify_decisions_path(),
            &format!("{line}\n"),
            NOTIFY_DECISIONS_MAX_BYTES,
        );
    }
}

/// SessionStatus 的 wire 风格字符串(lowercase,与 parse_status 输入对齐)。
fn status_str(s: &SessionStatus) -> &'static str {
    match s {
        SessionStatus::Pending => "pending",
        SessionStatus::Running => "running",
        SessionStatus::Idle => "idle",
        SessionStatus::LastFailed => "last_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::Session;
    use chrono::Utc;
    use serde_json::Value;

    fn fake_result(status: SessionStatus, prev: Option<SessionStatus>) -> UpsertResult {
        let now = Utc::now();
        UpsertResult {
            session: Session {
                id: "sess-1".into(),
                task_id: "cc-1".into(),
                external_session_id: None,
                title: "t".into(),
                message: "m".into(),
                source: None,
                priority: Default::default(),
                status,
                created_at: now,
                updated_at: now,
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
            },
            is_new: true,
            prev_status: prev,
            prev_message: None,
        }
    }

    #[test]
    fn trace_serializes_all_fields_with_defaults_for_unwalked_gates() {
        let mut req = NotifyRequest {
            title: "t".into(),
            message: "m".into(),
            task_id: "cc-1".into(),
            protocol_version: None,
            external_session_id: None,
            source: None,
            priority: None,
            event_type: Some("Stop".into()),
            status: Some("idle".into()),
            terminal_tty: None,
            workspace_path: None,
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
        // 老 hook 可能缺 event_type/status:字段应落默认空串而非缺键。
        req.event_type = None;
        req.status = None;

        let mut trace = NotifyDecisionTrace::new(&req);
        trace.set_session_context(&fake_result(
            SessionStatus::Idle,
            Some(SessionStatus::Running),
        ));

        let v: Value = serde_json::to_value(&trace).unwrap();
        assert_eq!(v["task_id"], "cc-1");
        assert_eq!(v["session_id"], "sess-1");
        assert_eq!(v["event_type"], "");
        assert_eq!(v["status"], "idle");
        assert_eq!(v["prev_status"], "running");
        assert_eq!(v["is_new"], true);
        // 未走到 gate 的缺省:false / 空串
        assert_eq!(v["dedupe_duplicate"], false);
        assert_eq!(v["stage_eligible"], false);
        assert_eq!(v["presentation"], "");
        assert_eq!(v["suppress_reason"], "");
        assert_eq!(v["sound_played"], "");
        assert!(!v["ts"].as_str().unwrap().is_empty());
    }

    #[test]
    fn status_str_matches_wire_lowercase() {
        assert_eq!(status_str(&SessionStatus::Pending), "pending");
        assert_eq!(status_str(&SessionStatus::Running), "running");
        assert_eq!(status_str(&SessionStatus::Idle), "idle");
        assert_eq!(status_str(&SessionStatus::LastFailed), "last_failed");
    }
}
