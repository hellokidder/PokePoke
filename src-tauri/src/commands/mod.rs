pub mod diagnostics;
pub mod focus;
pub mod integration;
pub mod quiet_mode;
pub mod sounds;
pub mod updater;

pub use focus::{capture_ghostty_focused_terminal_id, cwds_compatible};
pub use integration::{cc_integration_status, emit_cc_integration_updated, HookDriftState};
pub use updater::{PendingUpdate, PendingUpdateState};

use crate::popup::{self, PopupList};
use crate::sessions::{Session, SessionStore};
use crate::settings::{Settings, SettingsStore};
use crate::tray;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
pub fn get_sessions(store: State<'_, Arc<Mutex<SessionStore>>>) -> Vec<Session> {
    store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_all()
        .to_vec()
}

#[tauri::command]
pub fn get_session_by_id(
    id: String,
    store: State<'_, Arc<Mutex<SessionStore>>>,
) -> Option<Session> {
    let s = store.lock().unwrap_or_else(|p| p.into_inner());
    s.get_all().iter().find(|s| s.id == id).cloned()
}

#[tauri::command]
pub fn remove_session(
    id: String,
    store: State<'_, Arc<Mutex<SessionStore>>>,
    popup_list: State<'_, PopupList>,
    app: AppHandle,
) {
    {
        let mut s = store.lock().unwrap_or_else(|p| p.into_inner());
        s.remove_session(&id);
    }
    popup::close_popup(
        &app,
        &id,
        &popup_list,
        popup::PopupCloseReason::RemoveSession,
    );
    let _ = app.emit("sessions-updated", ());
}

#[tauri::command]
pub fn close_popup_window(id: String, popup_list: State<'_, PopupList>, app: AppHandle) {
    crate::log_info!(
        "notify",
        "close_popup_window from frontend session_id={}",
        id
    );
    popup::close_popup(
        &app,
        &id,
        &popup_list,
        popup::PopupCloseReason::FrontendDismiss,
    );
}

#[tauri::command]
pub fn get_settings(store: State<'_, Arc<Mutex<SettingsStore>>>) -> Settings {
    store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .settings
        .clone()
}

#[tauri::command]
pub fn save_settings(
    settings: Settings,
    store: State<'_, Arc<Mutex<SettingsStore>>>,
    app: AppHandle,
) {
    let new_tray_style = settings.tray_icon_style;
    let prev_tray_style = {
        let mut s = store.lock().unwrap_or_else(|p| p.into_inner());
        let prev = s.settings.tray_icon_style;
        s.update(settings);
        prev
    };
    if prev_tray_style != new_tray_style {
        crate::tray::apply_tray_icon_style(&app, new_tray_style);
    }
    let _ = app.emit("settings-updated", ());
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    tray::open_settings_window(&app);
    focus::auto_hide_panel(&app, "open_settings_window");
}

#[tauri::command]
pub fn close_settings_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.destroy();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle, store: State<'_, Arc<Mutex<SessionStore>>>) {
    crate::log_info!("lifecycle", "quit_app invoked from panel");
    store.lock().unwrap_or_else(|p| p.into_inner()).flush();
    crate::log_info!("app", "exiting reason=quit_app_command");
    app.exit(0);
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    crate::log_debug!("panel", "hide_panel invoked from frontend");
    if app.get_webview_window("panel").is_some() {
        crate::log_debug!("panel", "hide_panel: calling hide_session_panel()");
        crate::tray::hide_session_panel(&app);
    } else {
        crate::log_debug!("panel", "hide_panel: panel window not found");
    }
}

#[tauri::command]
pub fn log_from_frontend(level: String, tag: String, message: String) {
    crate::logger::log_from_frontend(&level, &tag, &message);
}

/// 前端 About / Diagnostics 显示的版本串。
///
/// - 有 `POKE_BUILD_HASH`(build.rs 本地构建时注入)→ `"1.0.1 (a1b2c3d4)"`
///   或 `"1.0.1 (a1b2c3d4-dirty)"`,方便自查现在跑的是哪个 commit。
/// - 没有(CI 打包 / 无 git 环境)→ 回退到纯 `CARGO_PKG_VERSION`,跟以前
///   `@tauri-apps/api/app` 的 `getVersion()` 行为一致。updater 比对走远端
///   `latest.json.version`,不读这个,不会受影响。
#[tauri::command]
pub fn app_version() -> String {
    let base = env!("CARGO_PKG_VERSION");
    match option_env!("POKE_BUILD_HASH") {
        Some(hash) if !hash.is_empty() => format!("{base} ({hash})"),
        _ => base.to_string(),
    }
}

/// 弹出全屏彩带覆盖层(激活/校验成功时由前端调用)。
#[tauri::command]
pub fn celebrate(app: AppHandle) {
    crate::confetti::show_confetti_overlay(&app);
}

/// 彩带播放结束后,前端主动关闭覆盖层窗口。
#[tauri::command]
pub fn close_confetti_overlay(app: AppHandle) {
    crate::confetti::close_confetti_overlay(&app);
}
