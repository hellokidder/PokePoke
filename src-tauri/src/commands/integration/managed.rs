use crate::popup::PopupList;
use crate::session_lifecycle::remove_session_with_cleanup;
use crate::sessions::{Session, SessionStore};
use crate::settings::SettingsStore;
use crate::tray;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

use super::drift::HookDriftState;
use super::status::{cc_integration_status, emit_cc_integration_updated, repair_cc_binary_path};
use super::{
    after_integration_change, clear_integration_disconnected, install_arg_for_source,
    mark_source_auto_connected, notify_source_for_integration, run_poke_hook_json,
    uninstall_arg_for_source,
};

/// 列出某 managed integration 在 session store 中对应的 session id(纯函数,可单测)。
pub(crate) fn session_ids_for_integration(sessions: &[Session], integration: &str) -> Vec<String> {
    let Some(notify_source) = notify_source_for_integration(integration) else {
        return Vec::new();
    };
    sessions
        .iter()
        .filter(|s| s.source.as_deref() == Some(notify_source))
        .map(|s| s.id.clone())
        .collect()
}

pub fn clear_sessions_for_integration(
    app: &AppHandle,
    session_store: &std::sync::Arc<std::sync::Mutex<SessionStore>>,
    popup_list: &PopupList,
    integration: &str,
) -> usize {
    let ids: Vec<String> = {
        let store = session_store.lock().unwrap_or_else(|p| p.into_inner());
        session_ids_for_integration(&store.sessions, integration)
    };
    for id in &ids {
        remove_session_with_cleanup(
            app,
            session_store,
            popup_list,
            id,
            crate::popup::PopupCloseReason::IntegrationDisconnect,
        );
    }
    if !ids.is_empty() {
        let _ = app.emit("sessions-updated", ());
        crate::log_info!(
            "lifecycle",
            "cleared {} session(s) for disconnected integration {}",
            ids.len(),
            integration,
        );
    }
    ids.len()
}

fn mark_integration_disconnected(
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    source: &str,
) {
    if let Ok(mut g) = settings_store.lock() {
        if !g.settings.disconnected_sources.iter().any(|s| s == source) {
            g.settings.disconnected_sources.push(source.to_string());
        }
        if !g
            .settings
            .auto_connected_sources
            .iter()
            .any(|s| s == source)
        {
            g.settings.auto_connected_sources.push(source.to_string());
        }
        g.save();
    }
}

pub(crate) fn disconnect_integration_fully(
    app: &AppHandle,
    source: &str,
    settings_store: &std::sync::Arc<std::sync::Mutex<SettingsStore>>,
    session_store: &std::sync::Arc<std::sync::Mutex<SessionStore>>,
    popup_list: &PopupList,
) -> serde_json::Value {
    let Some(uninstall_arg) = uninstall_arg_for_source(source) else {
        return serde_json::json!({
            "status": "error",
            "message": format!("unknown source: {}", source),
        });
    };

    let result = run_poke_hook_json(&[uninstall_arg]);
    let ok = result
        .get("status")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == "ok");
    let mut sessions_removed = 0usize;
    if ok {
        mark_integration_disconnected(settings_store, source);
        sessions_removed = clear_sessions_for_integration(app, session_store, popup_list, source);
        crate::log_info!(
            "lifecycle",
            "disconnected integration {} (cleared {} sessions)",
            source,
            sessions_removed,
        );
    }
    after_integration_change(app, source);
    if ok {
        return serde_json::json!({
            "status": "ok",
            "message": result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("disconnected"),
            "sessions_removed": sessions_removed,
        });
    }
    result
}

#[tauri::command]
pub fn connect_managed_integration(
    app: AppHandle,
    source: String,
    settings_store: State<'_, Arc<Mutex<SettingsStore>>>,
) -> serde_json::Value {
    if !crate::agent_presence::is_agent_present(&source) {
        return serde_json::json!({
            "status": "error",
            "message": format!("{} is not installed on this Mac", source),
        });
    }
    let Some(install_arg) = install_arg_for_source(&source) else {
        return serde_json::json!({
            "status": "error",
            "message": format!("unknown source: {}", source),
        });
    };

    let result = run_poke_hook_json(&[install_arg]);
    let ok = result
        .get("status")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == "ok");
    if ok {
        mark_source_auto_connected(&settings_store, &source);
        clear_integration_disconnected(&settings_store, &source);
        if let Some(state) = app.try_state::<Arc<Mutex<HookDriftState>>>() {
            state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear_source(&source);
        }
    }
    after_integration_change(&app, &source);
    result
}

#[tauri::command]
pub fn disconnect_managed_integration(
    app: AppHandle,
    source: String,
    settings_store: State<'_, Arc<Mutex<SettingsStore>>>,
    session_store: State<'_, Arc<Mutex<SessionStore>>>,
    popup_list: State<'_, PopupList>,
) -> serde_json::Value {
    disconnect_integration_fully(&app, &source, &settings_store, &session_store, &popup_list)
}

/// 写入 `~/.cursor/hooks.json`，启用 PokePoke 的 Cursor 全局 hook 接入。
#[tauri::command]
pub fn install_cursor_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--install-cursor"]);
    // 托盘菜单状态依赖这份 JSON，成功与否都刷新一次；失败时菜单项仍然能
    // 让用户看到真实状态（"Cursor: Not connected"）。
    tray::refresh_tray_menu(&app);
    result
}

/// 从 `~/.cursor/hooks.json` 移除 PokePoke 条目（保留用户其他 hook）。
#[tauri::command]
pub fn uninstall_cursor_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--uninstall-cursor"]);
    tray::refresh_tray_menu(&app);
    result
}

/// 投放 opencode plugin,启用 PokePoke 的 opencode 接入。
#[tauri::command]
pub fn install_opencode_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--install-opencode"]);
    tray::refresh_tray_menu(&app);
    result
}

/// 移除 opencode plugin。
#[tauri::command]
pub fn uninstall_opencode_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--uninstall-opencode"]);
    tray::refresh_tray_menu(&app);
    result
}

/// 投放 pi 扩展,启用 PokePoke 的 pi 接入。
#[tauri::command]
pub fn install_pi_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--install-pi"]);
    tray::refresh_tray_menu(&app);
    result
}

/// 移除 pi 扩展(删整个 pokepoke/ 目录)。
#[tauri::command]
pub fn uninstall_pi_integration(app: AppHandle) -> serde_json::Value {
    let result = run_poke_hook_json(&["--uninstall-pi"]);
    tray::refresh_tray_menu(&app);
    result
}

#[tauri::command]
pub fn repair_cc_integration(app: AppHandle) -> serde_json::Value {
    let bin = repair_cc_binary_path();
    if !bin.exists() {
        return serde_json::json!({
            "ok": false,
            "status": cc_integration_status(),
            "message": "poke-hook binary not found for repair"
        });
    }

    let repair_ok = Command::new(&bin)
        .arg("--install")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if repair_ok {
        if let Some(settings_store) = app.try_state::<Arc<Mutex<SettingsStore>>>() {
            clear_integration_disconnected(&settings_store, "cc");
            mark_source_auto_connected(&settings_store, "cc");
        }
    }

    tray::refresh_tray_menu(&app);
    emit_cc_integration_updated(&app);

    serde_json::json!({
        "ok": repair_ok,
        "status": cc_integration_status(),
    })
}
