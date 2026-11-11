use crate::quiet_mode::{self, QuietModeState};
use crate::settings::SettingsStore;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn get_quiet_mode_status(state: State<'_, Arc<QuietModeState>>) -> quiet_mode::QuietModeStatus {
    state.status()
}

#[tauri::command]
pub fn start_quiet_mode(
    duration_mins: u32,
    app: AppHandle,
    state: State<'_, Arc<QuietModeState>>,
    settings_store: State<'_, Arc<Mutex<SettingsStore>>>,
) -> Result<quiet_mode::QuietModeStatus, String> {
    if duration_mins == 0 || duration_mins > 24 * 60 {
        return Err("duration out of range".into());
    }
    {
        let mut store = settings_store.lock().unwrap_or_else(|p| p.into_inner());
        store.settings.quiet_mode_duration_mins = duration_mins;
        store.save();
    }
    let end = state.start(Duration::from_secs(duration_mins as u64 * 60));
    let status = state.status();
    quiet_mode::notify_state_changed(&app, &state);
    quiet_mode::spawn_expiry_watcher(app.clone(), state.inner().clone(), end);
    quiet_mode::spawn_tray_title_ticker(app.clone(), state.inner().clone(), end);
    quiet_mode::close_quiet_mode_picker(&app);
    crate::log_info!("quiet_mode", "started for {} min", duration_mins);
    Ok(status)
}

#[tauri::command]
pub fn cancel_quiet_mode(
    app: AppHandle,
    state: State<'_, Arc<QuietModeState>>,
) -> quiet_mode::QuietModeStatus {
    state.clear();
    let status = state.status();
    quiet_mode::notify_state_changed(&app, &state);
    crate::log_info!("quiet_mode", "cancelled by user");
    status
}

#[tauri::command]
pub fn open_quiet_mode_picker(app: AppHandle) {
    quiet_mode::open_quiet_mode_picker(&app);
}

#[tauri::command]
pub fn close_quiet_mode_picker(app: AppHandle) {
    quiet_mode::close_quiet_mode_picker(&app);
}
