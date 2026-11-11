//! pokehook 单测:omp(oh-my-pi)接入(AGENTS.md §4a:按 agent 拆测试文件)。
//!
//! 本文件挂在 tests.rs 内部(`#[path] mod omp_tests`),`super` 即 tests 模块,
//! `super::super` 即 pokehook 根模块。omp 与 pi 同族但事件目录不同,这里覆盖:
//! source 往返 / payload 映射 / 扩展内容契约 / 落点与共用目录冲突判定。
use super::super::omp::{
    omp_extension_configured, omp_extension_contents, omp_extension_outdated,
    omp_shared_agent_dir_conflict, omp_uninstall_refusal,
};
use super::super::{build_payload, Source};
use super::plan_snapshot_path;
use serde_json::json;
use serial_test::serial;
use tempfile::tempdir;

// ==================== source 往返 ====================

#[test]
fn omp_source_roundtrip() {
    assert_eq!(Source::Omp.as_str(), "omp");
    assert_eq!(Source::Omp.task_id_prefix(), "omp");
    assert_eq!(Source::from_arg("omp"), Some(Source::Omp));
}

#[test]
fn omp_task_id_uses_sid8_prefix() {
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj"
    });
    let payload = build_payload("SessionStart", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["task_id"], "omp-01a0ce59");
}

// ==================== payload 映射 ====================

#[test]
fn omp_session_start_payload_maps_running() {
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("SessionStart", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["event_type"], "running");
    assert_eq!(payload["status"], "running");
    assert_eq!(payload["source"], "omp");
    assert_eq!(payload["workspace_path"], "/Users/edy/proj");
    assert_eq!(
        payload["external_session_id"],
        "01a0ce59-9c11-76b8-949a-e882129cb317"
    );
}

#[test]
fn omp_stop_payload_maps_idle() {
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("Stop", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["source"], "omp");
    assert!(payload.get("aborted").is_none());
}

#[test]
fn omp_stop_payload_aborted_carries_flag() {
    // omp 扩展在 stopReason=aborted(用户取消)时随 Stop 上报 aborted:true,
    // server 据此抑制"完成"音(与 pi / cursor 对齐)。
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj",
        "aborted": true
    });

    let payload = build_payload("Stop", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["aborted"], true);
}

#[test]
fn omp_permission_request_payload_maps_pending() {
    // omp 原生 tool_approval_requested -> PermissionRequest -> Pending
    // (等价 CC 的权限确认)。
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj",
        "tool_name": "bash"
    });

    let payload = build_payload("PermissionRequest", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["event_type"], "pending");
    assert_eq!(payload["status"], "pending");
    assert_eq!(payload["message"], "Awaiting approval: bash");
}

#[test]
fn omp_stop_failure_payload_maps_last_failed() {
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj",
        "reason": "provider_error",
        "detail": "HTTP 429 rate limited"
    });

    let payload = build_payload("StopFailure", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "last_failed");
    assert_eq!(payload["source"], "omp");
    assert_eq!(payload["message"], "HTTP 429 rate limited");
}

#[test]
fn omp_session_end_payload_maps_session_end() {
    let data = json!({
        "session_id": "01a0ce59-9c11-76b8-949a-e882129cb317",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("SessionEnd", &data, Source::Omp, None).unwrap();
    assert_eq!(payload["event_type"], "session_end");
    assert_eq!(payload["source"], "omp");
}

// ==================== 扩展内容契约(编译期只嵌入字符串,内容断言是唯一防回归手段)====================

#[test]
#[serial]
fn omp_extension_contents_bakes_absolute_hook_path() {
    let contents = omp_extension_contents();
    // 占位符必须被替换,不能残留
    assert!(!contents.contains("__POKE_HOOK_BIN__"));
    // 必须含 PokePoke 标记 + source 调用(check/verify 都靠这两个子串判定)
    assert!(contents.contains("PokePoke omp integration"));
    assert!(contents.contains("--source=omp"));
    // 烘焙进去的是绝对路径 poke-hook
    assert!(contents.contains("/.local/bin/poke-hook"));
}

#[test]
#[serial]
fn omp_extension_uses_agent_end_settle_not_agent_settled() {
    // 回归护栏:omp 事件目录里**没有** agent_settled(注册不报错但永不 fire)。
    // 若有人把 pi 变体拷过来当 omp 用,session 会永远卡 Running —— 这里显式拦下。
    let contents = omp_extension_contents();
    assert!(
        !contents.contains("pi.on(\"agent_settled\""),
        "omp 变体不得订阅 agent_settled(该事件在 omp 里永不存在)"
    );
    assert!(contents.contains("agent_end"));
    assert!(contents.contains("willContinue"));
    // 能力探测:pi 的 agent_end 没有 willContinue 属性,靠 `in` 判定宿主。
    assert!(contents.contains("\"willContinue\" in "));
}

#[test]
#[serial]
fn omp_extension_filters_subagent_sessions() {
    // subagent(task tool)会在同进程再跑一次工厂,且 sub-session id 前 8 字符与父相同
    // → 不过滤就会撞 task_id 并冒充父 session 的 Stop。
    let contents = omp_extension_contents();
    assert!(contents.contains("localProtocolOptions"));
    assert!(contents.contains("isSubSession"));
    // 关键事件关键字(防模板漂移)
    assert!(contents.contains("session_start"));
    assert!(contents.contains("before_agent_start"));
    assert!(contents.contains("session_switch"));
    assert!(contents.contains("session_shutdown"));
    assert!(contents.contains("tool_approval_requested"));
}

#[test]
#[serial]
fn omp_extension_configured_requires_both_markers() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("index.ts");

    std::fs::write(&path, omp_extension_contents()).unwrap();
    assert!(omp_extension_configured(&path));

    // pi 变体文件(少了 omp 标记)-> 未接入(共用 agent dir 场景即靠这条暴露)
    std::fs::write(
        &path,
        "// PokePoke pi integration\nconst x = \"--source=pi\";\n",
    )
    .unwrap();
    assert!(!omp_extension_configured(&path));

    std::fs::write(&path, "export default function () {}\n").unwrap();
    assert!(!omp_extension_configured(&path));

    std::fs::remove_file(&path).unwrap();
    assert!(!omp_extension_configured(&path));
}

#[test]
#[serial]
fn omp_extension_outdated_when_baked_path_differs() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("index.ts");

    std::fs::write(&path, omp_extension_contents()).unwrap();
    assert!(!omp_extension_outdated(&path));

    let stale = omp_extension_contents().replace("/.local/bin/poke-hook", "/old/path/poke-hook");
    std::fs::write(&path, stale).unwrap();
    assert!(omp_extension_outdated(&path));

    std::fs::write(&path, "export default function () {}\n").unwrap();
    assert!(!omp_extension_outdated(&path));
}

#[test]
fn omp_snapshot_path_uses_ts_ext() {
    let dir = std::env::temp_dir().join("pokepoke-omp-snapshot-test");
    let path = plan_snapshot_path(&dir, "omp", 1745332800);
    assert_eq!(
        path,
        dir.join("snapshots/omp-1745332800.ts"),
        "omp snapshot 应为 .ts 扩展名"
    );
}

// ==================== agent dir 解析 / 共用目录冲突 ====================
//
// env 读写 + serial:PI_CODING_AGENT_DIR / OMP_PROFILE 是进程级全局状态。

/// 保存/恢复一组 env(测试内失败也要还原)。
struct EnvGuard {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
        let keys = vec!["PI_CODING_AGENT_DIR", "OMP_PROFILE", "PI_PROFILE"];
        let saved = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect::<Vec<_>>();
        // SAFETY: 单线程(#[serial])单测;Drop 中还原。
        unsafe {
            for k in &keys {
                std::env::remove_var(k);
            }
            for (k, v) in pairs {
                if let Some(v) = v {
                    std::env::set_var(k, v);
                }
            }
        }
        Self { keys: saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            for (k, v) in self.keys.drain(..) {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

#[test]
#[serial]
fn omp_extension_path_defaults_to_home_omp_agent_dir() {
    let home = std::env::var("HOME").unwrap_or_default();
    let _guard = EnvGuard::set(&[]);
    let path = super::super::paths::omp_extension_path();
    assert_eq!(
        path,
        std::path::PathBuf::from(home).join(".omp/agent/extensions/pokepoke/index.ts")
    );
}

#[test]
#[serial]
fn omp_extension_path_follows_pi_coding_agent_dir() {
    let _guard = EnvGuard::set(&[("PI_CODING_AGENT_DIR", Some("/tmp/omp-custom-agent"))]);
    assert_eq!(
        super::super::paths::omp_extension_path(),
        std::path::PathBuf::from("/tmp/omp-custom-agent/extensions/pokepoke/index.ts")
    );
}

#[test]
#[serial]
fn omp_extension_path_prefers_profile_over_env() {
    // omp 侧 profile 存在时忽略 PI_CODING_AGENT_DIR(见 dirs.ts)。
    let _guard = EnvGuard::set(&[
        ("PI_CODING_AGENT_DIR", Some("/tmp/omp-custom-agent")),
        ("OMP_PROFILE", Some("work")),
    ]);
    let home = std::env::var("HOME").unwrap_or_default();
    assert_eq!(
        super::super::paths::omp_extension_path(),
        std::path::PathBuf::from(home)
            .join(".omp/profiles/work/agent/extensions/pokepoke/index.ts")
    );
}

#[test]
#[serial]
fn omp_profile_falls_back_to_pi_profile_env() {
    let _guard = EnvGuard::set(&[("PI_PROFILE", Some("work"))]);
    let home = std::env::var("HOME").unwrap_or_default();
    assert_eq!(
        super::super::paths::omp_agent_dir(),
        std::path::PathBuf::from(home).join(".omp/profiles/work/agent")
    );
}

#[test]
#[serial]
fn omp_shared_agent_dir_conflict_detects_pi_dir() {
    let _guard = EnvGuard::set(&[]);
    assert!(
        omp_shared_agent_dir_conflict().is_none(),
        "默认落点 ~/.omp/agent 与 ~/.pi/agent 不同,不应判冲突"
    );
}

#[test]
#[serial]
fn omp_shared_agent_dir_conflict_when_env_points_at_pi_dir() {
    // PI_CODING_AGENT_DIR 指到 pi 的 agent dir 时,omp 会去加载 pi 变体文件;
    // 此时投放 omp 变体会覆盖 pi 变体 -> 必须拒绝(install 报错、不写文件)。
    let home = std::env::var("HOME").unwrap_or_default();
    let pi_dir = std::path::PathBuf::from(home).join(".pi/agent");
    let _guard = EnvGuard::set(&[("PI_CODING_AGENT_DIR", pi_dir.to_str())]);
    let reason = omp_shared_agent_dir_conflict();
    assert!(
        reason.is_some(),
        "omp agent dir == pi agent dir 应判为冲突并拒绝投放"
    );
    assert!(reason.unwrap().contains("same agent dir"));
}

// ==================== 卸载守卫(2026-09-24 hello 真机回归)====================
//
// 现场:`--uninstall-omp` 在「omp 与 pi 同 agent dir」状态下把 pi 变体文件删掉了
// (install 会拒绝、uninstall 却照删 → 不对称)。以下三条锁住新守卫。

/// 建一个 <root>/<agent>/extensions/pokepoke/index.ts,内容由调用方给。
fn seed_extension(
    root: &std::path::Path,
    agent: &str,
    body: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let ext_dir = root.join(agent).join("extensions/pokepoke");
    std::fs::create_dir_all(&ext_dir).unwrap();
    let ext = ext_dir.join("index.ts");
    std::fs::write(&ext, body).unwrap();
    (root.join(agent), ext)
}

#[test]
fn omp_uninstall_refuses_shared_agent_dir_and_keeps_pi_variant() {
    let dir = tempdir().unwrap();
    // omp 解析到 pi 的 agent dir:目录里放着 pi 变体
    let (shared_dir, ext) = seed_extension(dir.path(), ".pi/agent", "// PI VARIANT SENTINEL\n");
    let reason = omp_uninstall_refusal(&shared_dir, &shared_dir, &ext)
        .expect("共用 agent dir 时必须拒绝卸载");
    assert!(
        reason.contains("same agent dir"),
        "unexpected reason: {reason}"
    );
    assert!(ext.exists(), "守卫拒绝时不得删除文件");
}

#[test]
fn omp_uninstall_refuses_foreign_extension_content() {
    let dir = tempdir().unwrap();
    let (omp_dir, ext) = seed_extension(dir.path(), ".omp/agent", "// my own extension\n");
    let pi_dir = dir.path().join(".pi/agent");
    let reason =
        omp_uninstall_refusal(&omp_dir, &pi_dir, &ext).expect("非本接入投放的内容必须拒绝卸载");
    assert!(
        reason.contains("not a PokePoke-managed omp extension"),
        "unexpected reason: {reason}"
    );
    assert!(ext.exists(), "守卫拒绝时不得删除文件");
}

#[test]
fn omp_uninstall_allows_pokepoke_managed_variant() {
    let dir = tempdir().unwrap();
    // 真内容:带 marker + --source=omp(与 --install-omp 投放的一致)
    let (omp_dir, ext) = seed_extension(dir.path(), ".omp/agent", &omp_extension_contents());
    let pi_dir = dir.path().join(".pi/agent");
    assert!(
        omp_uninstall_refusal(&omp_dir, &pi_dir, &ext).is_none(),
        "本接入投放的 omp 变体必须可卸载"
    );
}
