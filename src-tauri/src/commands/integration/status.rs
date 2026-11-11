use crate::settings::SettingsStore;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

use super::drift::HookDriftState;
use super::{
    check_arg_for_source, hook_bin_path, is_executable, run_poke_hook_json,
    MANAGED_INTEGRATION_ORDER,
};

const CC_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "Notification",
    "PermissionRequest",
    "Stop",
    "StopFailure",
];

#[derive(Debug, Clone, Serialize)]
pub struct CcIntegrationStatus {
    pub installed: bool,
    pub binary_executable: bool,
    pub settings_exists: bool,
    pub hooks_configured: bool,
    pub connected: bool,
    pub repair_available: bool,
    pub issue: String,
}

fn claude_settings_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".claude/settings.json")
}

fn env_fallback_path() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_default()
}

/// CC repair 路径选择(`~/.local/bin/poke-hook`,缺失时回退 current_exe)。
/// 2026-08-12 拆分:integration.rs 抽 mod 后由 `managed`(repair_cc_integration)使用,
/// 可见性 private → pub(crate)。
pub(crate) fn repair_cc_binary_path() -> std::path::PathBuf {
    let installed = hook_bin_path();
    if installed.exists() {
        installed
    } else {
        env_fallback_path()
    }
}

fn contains_poke_hook(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|h| {
            h["command"]
                .as_str()
                .is_some_and(|command| command.contains("poke-hook"))
        })
    })
}

fn cc_issue(installed: bool, binary_executable: bool, hooks_configured: bool) -> &'static str {
    if !installed {
        "binary_missing"
    } else if !binary_executable {
        "binary_not_executable"
    } else if !hooks_configured {
        "hooks_missing"
    } else {
        "healthy"
    }
}

pub fn cc_integration_status() -> CcIntegrationStatus {
    let hook_path = hook_bin_path();
    let settings_path = claude_settings_path();

    let installed = hook_path.exists();
    let binary_executable = installed && is_executable(&hook_path);
    let settings_exists = settings_path.exists();
    let hooks_configured = if settings_exists {
        let content = std::fs::read_to_string(&settings_path).unwrap_or_default();
        let settings: Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        CC_HOOK_EVENTS.iter().all(|event| {
            settings["hooks"][event]
                .as_array()
                .is_some_and(|arr| arr.iter().any(contains_poke_hook))
        })
    } else {
        false
    };
    let connected = installed && binary_executable && hooks_configured;
    let repair_available = repair_cc_binary_path().exists();

    CcIntegrationStatus {
        installed,
        binary_executable,
        settings_exists,
        hooks_configured,
        connected,
        repair_available,
        issue: cc_issue(installed, binary_executable, hooks_configured).into(),
    }
}

pub fn emit_cc_integration_updated(app: &AppHandle) {
    let _ = app.emit("cc-integration-updated", cc_integration_status());
}
/// 设置面板「接入端」Tab 用的单行状态。
#[derive(Debug, Clone, Serialize)]
pub struct ManagedIntegrationStatus {
    pub source: String,
    pub agent_present: bool,
    pub connected: bool,
    pub hooks_configured: bool,
    pub hooks_outdated: bool,
    pub installed: bool,
    pub issue: Option<String>,
    pub repair_available: bool,
    pub disconnected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_enabled: Option<bool>,
}

/// 诊断导出复用:返回 codex / cursor `--check-*` 的原始 JSON。
/// 调用方(diagnostics)负责字段白名单脱敏,不直接整段透传。
macro_rules! hook_check_json_fn {
    ($fn:ident, $flag:expr) => {
        pub(crate) fn $fn() -> serde_json::Value {
            run_poke_hook_json(&[$flag])
        }
    };
}
hook_check_json_fn!(codex_check_json, "--check-codex");
hook_check_json_fn!(cursor_check_json, "--check-cursor");
hook_check_json_fn!(opencode_check_json, "--check-opencode");
hook_check_json_fn!(pi_check_json, "--check-pi");
hook_check_json_fn!(kimi_check_json, "--check-kimi");
hook_check_json_fn!(grok_check_json, "--check-grok");

/// 设置面板行状态(CC 分支,注入检测结果,便于单测)。
pub(crate) fn managed_status_from_cc(
    cc: &CcIntegrationStatus,
    agent_present: bool,
    hooks_outdated: bool,
    disconnected: bool,
) -> ManagedIntegrationStatus {
    ManagedIntegrationStatus {
        source: "cc".to_string(),
        agent_present,
        connected: cc.connected && !disconnected,
        hooks_configured: cc.hooks_configured,
        hooks_outdated,
        installed: cc.installed,
        issue: if disconnected {
            None
        } else {
            Some(cc.issue.clone())
        },
        repair_available: cc.repair_available,
        disconnected,
        feature_enabled: None,
    }
}

/// 设置面板行状态(非 CC 分支,注入 poke-hook check JSON,便于单测)。
pub(crate) fn managed_status_from_hook_check(
    source: &str,
    agent_present: bool,
    check: &serde_json::Value,
    hooks_outdated_flag: bool,
    disconnected: bool,
    repair_available: bool,
) -> ManagedIntegrationStatus {
    let bool_field = |key: &str| check.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    ManagedIntegrationStatus {
        source: source.to_string(),
        agent_present,
        connected: bool_field("connected") && !disconnected,
        hooks_configured: bool_field("hooks_configured"),
        hooks_outdated: hooks_outdated_flag || bool_field("hooks_outdated"),
        installed: bool_field("installed"),
        issue: None,
        repair_available,
        disconnected,
        feature_enabled: check.get("feature_enabled").and_then(|v| v.as_bool()),
    }
}

fn managed_integration_status(
    source: &str,
    outdated_sources: &HashSet<String>,
    disconnected_sources: &HashSet<String>,
) -> ManagedIntegrationStatus {
    let agent_present = crate::agent_presence::is_agent_present(source);
    let disconnected = disconnected_sources.contains(source);
    if source == "cc" {
        return managed_status_from_cc(
            &cc_integration_status(),
            agent_present,
            outdated_sources.contains(source),
            disconnected,
        );
    }

    let check = check_arg_for_source(source)
        .map(|arg| run_poke_hook_json(&[arg]))
        .unwrap_or_else(|| serde_json::json!({}));

    managed_status_from_hook_check(
        source,
        agent_present,
        &check,
        outdated_sources.contains(source),
        disconnected,
        hook_bin_path().exists(),
    )
}

fn disconnected_source_set(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
) -> HashSet<String> {
    settings_store
        .lock()
        .map(|g| g.settings.disconnected_sources.iter().cloned().collect())
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_managed_integrations(
    drift: State<'_, Arc<Mutex<HookDriftState>>>,
    settings_store: State<'_, Arc<Mutex<SettingsStore>>>,
) -> Vec<ManagedIntegrationStatus> {
    let outdated: HashSet<String> = drift
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .outdated_sources()
        .into_iter()
        .collect();
    let disconnected = disconnected_source_set(&settings_store);
    MANAGED_INTEGRATION_ORDER
        .iter()
        .map(|source| managed_integration_status(source, &outdated, &disconnected))
        .collect()
}

#[tauri::command]
pub fn check_managed_integration(
    source: String,
    drift: State<'_, Arc<Mutex<HookDriftState>>>,
    settings_store: State<'_, Arc<Mutex<SettingsStore>>>,
) -> ManagedIntegrationStatus {
    let outdated: HashSet<String> = drift
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .outdated_sources()
        .into_iter()
        .collect();
    let disconnected = disconnected_source_set(&settings_store);
    managed_integration_status(&source, &outdated, &disconnected)
}

#[tauri::command]
pub fn check_cc_integration() -> serde_json::Value {
    serde_json::to_value(cc_integration_status())
        .unwrap_or_else(|_| serde_json::json!({"connected": false, "issue": "unknown"}))
}

#[tauri::command]
pub fn check_codex_integration() -> serde_json::Value {
    run_poke_hook_json(&["--check-codex"])
}

/// 查询 Cursor 全局 hook 接入状态(`~/.cursor/hooks.json`)。
///
/// 语义与 `check_cc_integration` 对称:不再需要传入项目路径。
/// 内部直接委托 `poke-hook --check-cursor`。
#[tauri::command]
pub fn check_cursor_integration() -> serde_json::Value {
    run_poke_hook_json(&["--check-cursor"])
}

/// 查询 opencode plugin 接入状态(`~/.config/opencode/plugin/pokepoke.js`)。
#[tauri::command]
pub fn check_opencode_integration() -> serde_json::Value {
    run_poke_hook_json(&["--check-opencode"])
}

/// 查询 pi 扩展接入状态(`~/.pi/agent/extensions/pokepoke/index.ts`)。
#[tauri::command]
pub fn check_pi_integration() -> serde_json::Value {
    run_poke_hook_json(&["--check-pi"])
}
