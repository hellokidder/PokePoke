//! upsert 状态机 property-based 测试(W10,brief 20260813-upsert-proptest)。
//!
//! 随机事件序列(小 task_id 集合 × status/aborted 组合)驱动 `upsert_session`,
//! 每步与**参考模型**对账。模型规则逐条对照 `sessions.rs::upsert_session`
//! 实现(行号引用以 2026-08-13 HEAD 为准),发现力来自组合空间遍历,而非
//! example-based 的点覆盖。

use super::{Priority, SessionStatus, SessionStore, UpsertRequest};
use proptest::prelude::*;
use std::collections::HashMap;

/// 模型里对账的字段集合(与 brief 对齐:status / aborted_at 有无 /
/// failure_reason 有无与值)。
#[derive(Debug, Clone, PartialEq)]
struct ModelEntry {
    status: SessionStatus,
    aborted_at: bool,
    failure_reason: Option<String>,
}

/// 参考模型:HashMap<task_id, ModelEntry>。转移规则逐条对照实现:
///
/// 新建分支(`upsert_session` L378-495):
/// - status = req.status(L397);
/// - aborted_at = Some iff aborted == Some(true)(L404-408);
/// - failure_reason = Some(req.failure_reason) iff status == LastFailed,否则 None
///   (L383-386,注释「非 LastFailed 新 session 不该携带 failure_reason」)。
///
/// 已存在分支(L348-426):
/// - **aborted_echo**(L349-361):incoming=LastFailed && aborted != Some(true)
///   && existing.is_recently_aborted() → 整体早退(状态/failure_reason 不变)。
///   模型用 `aborted_at` 在场近似 `is_recently_aborted`(L184-191:5s 窗口)。
///   proptest 每步毫秒级,窗口恒在;窗口过期分支由既有单测覆盖
///   (`upsert_last_failed_outside_aborted_window_overrides`)。
/// - status 无条件覆盖(L370);
/// - aborted_at(L371-374):aborted==Some(true) → 钉住;status==Running → 清;
///   否则保留旧值;
/// - failure_reason(L376-379):status==LastFailed → Some(req.failure_reason),
///   否则 None —— 注意实现允许 LastFailed 携带 None(事件没带 reason),因此
///   断言方向是 `failure_reason.is_some() ⟹ LastFailed`,不是双向。
///
/// 注意:真实 `is_recently_aborted` 按墙钟 5s 窗口;若测试进程被暂停 >5s,
/// 真实实现会走「窗口过期」而模型按「恒在」—— 理论漂移,proptest 实际
/// 毫秒级,可接受。
#[derive(Debug, Default)]
struct Model {
    entries: HashMap<String, ModelEntry>,
}

impl Model {
    fn apply(&mut self, task_id: &str, status: SessionStatus, aborted: Option<bool>) -> bool {
        let Some(entry) = self.entries.get_mut(task_id) else {
            // 新建(对照 L378-495)
            let failure_reason = if status == SessionStatus::LastFailed {
                Some(format!("fr-{task_id}"))
            } else {
                None
            };
            self.entries.insert(
                task_id.to_string(),
                ModelEntry {
                    status,
                    aborted_at: aborted == Some(true),
                    failure_reason,
                },
            );
            return true; // is_new
        };
        // aborted_echo(对照 L349-361):LastFailed && 非显式 aborted && 窗口内
        if status == SessionStatus::LastFailed && aborted != Some(true) && entry.aborted_at {
            return false; // 早退,状态不变
        }
        // status 覆盖(L370)
        entry.status = status.clone();
        // aborted_at(L371-374)
        if aborted == Some(true) {
            entry.aborted_at = true;
        } else if status == SessionStatus::Running {
            entry.aborted_at = false;
        }
        // failure_reason(L376-379)
        entry.failure_reason = if status == SessionStatus::LastFailed {
            Some(format!("fr-{task_id}"))
        } else {
            None
        };
        false // is_new
    }
}

/// 从 store 按 task_id 抽取对账视图(status / aborted_at / failure_reason)。
fn store_view(store: &SessionStore) -> HashMap<String, ModelEntry> {
    store
        .sessions
        .iter()
        .map(|s| {
            (
                s.task_id.clone(),
                ModelEntry {
                    status: s.status.clone(),
                    aborted_at: s.aborted_at.is_some(),
                    failure_reason: s.failure_reason.clone(),
                },
            )
        })
        .collect()
}

fn upsert_req(task_id: &str, status: &SessionStatus, aborted: Option<bool>) -> UpsertRequest {
    UpsertRequest {
        task_id: task_id.to_string(),
        external_session_id: None,
        title: format!("t-{task_id}"),
        message: format!("m-{task_id}"),
        source: Some("claude-code".into()),
        priority: Priority::Normal,
        status: status.clone(),
        terminal_tty: None,
        workspace_path: None,
        failure_reason: if *status == SessionStatus::LastFailed {
            Some(format!("fr-{task_id}"))
        } else {
            None
        },
        transcript_path: None,
        terminal_app: None,
        dsh_nav_socket: None,
        aborted,
    }
}

prop_compose! {
    /// 事件:task_id(小集合,撞已有记录概率高)× status × aborted。
    fn event()(
        task in prop_oneof![Just("t1"), Just("t2")],
        status in prop_oneof![
            Just(SessionStatus::Pending),
            Just(SessionStatus::Running),
            Just(SessionStatus::Idle),
            Just(SessionStatus::LastFailed),
        ],
        aborted in any::<bool>(),
    ) -> (String, SessionStatus, bool) {
        (task.to_string(), status, aborted)
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    /// 随机事件序列(1~60 步)驱动 upsert,每步与参考模型对账。
    #[test]
    fn upsert_matches_reference_model(
        events in prop::collection::vec(event(), 1..60),
    ) {
        let (_dir, mut store, _path) = super::tests::temp_store();
        let mut model = Model::default();
        let mut is_new_counts: HashMap<String, usize> = HashMap::new();

        for (task_id, status, aborted) in events {
            let req = upsert_req(&task_id, &status, Some(aborted));
            let expected_new = !model.entries.contains_key(&task_id);
            let result = store.upsert_session(req);

            // is_new 恰好首次为 true(brief 断言 5)
            assert_eq!(
                result.is_new, expected_new,
                "task_id={task_id} status={status:?} aborted={aborted} is_new 应恰为首次创建"
            );
            *is_new_counts.entry(task_id.clone()).or_insert(0) += 1;
            if expected_new {
                assert_eq!(is_new_counts[&task_id], 1);
            }

            // 更新模型
            model.apply(&task_id, status, Some(aborted));

            // 每步对账(brief 断言 1/2/3/4)
            let view = store_view(&store);
            // 无重复 task_id + 集合全等
            let mut seen = std::collections::HashSet::new();
            for s in &store.sessions {
                assert!(seen.insert(s.task_id.clone()), "task_id {} 重复", s.task_id);
                assert!(
                    matches!(
                        s.status,
                        SessionStatus::Pending
                            | SessionStatus::Running
                            | SessionStatus::Idle
                            | SessionStatus::LastFailed
                    ),
                    "status 非法"
                );
                // failure_reason 方向性不变量:is_some ⟹ LastFailed
                if s.failure_reason.is_some() {
                    assert_eq!(s.status, SessionStatus::LastFailed);
                }
            }
            assert_eq!(view.len(), model.entries.len(), "store 与模型 task 集合不一致");
            for (tid, m) in &model.entries {
                let v = view.get(tid).unwrap_or_else(|| panic!("模型有 {tid},store 无"));
                assert_eq!(&v.status, &m.status, "{tid} status 对账失败");
                assert_eq!(v.aborted_at, m.aborted_at, "{tid} aborted_at 对账失败");
                assert_eq!(
                    v.failure_reason, m.failure_reason,
                    "{tid} failure_reason 对账失败"
                );
            }
        }
    }
}
