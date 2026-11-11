use crate::custom_sounds::CustomSoundInfo;
use crate::settings::SettingsStore;
use std::sync::{Arc, Mutex};
use tauri::State;

#[tauri::command]
pub fn list_custom_sounds() -> Result<Vec<CustomSoundInfo>, String> {
    crate::custom_sounds::list_custom_sounds().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_custom_sound() -> Result<CustomSoundInfo, String> {
    let picked = rfd::FileDialog::new()
        .add_filter("Audio", &["wav", "mp3", "aiff", "aif", "m4a", "caf"])
        .pick_file()
        .ok_or_else(|| "cancelled".to_string())?;

    crate::custom_sounds::import_from_path(&picked).map_err(|e| {
        let code = e.to_string();
        crate::log_warn!(
            "custom_sound",
            "import rejected path={:?} reason={code}",
            picked
        );
        code
    })
}

#[tauri::command]
pub fn delete_custom_sound(
    id: String,
    settings: State<'_, Arc<Mutex<SettingsStore>>>,
) -> Result<(), String> {
    let store = settings.lock().unwrap_or_else(|p| p.into_inner());
    let s = &store.settings;
    let in_use = [
        s.sound_pending.as_str(),
        s.sound_idle.as_str(),
        s.sound_failed.as_str(),
    ];
    crate::custom_sounds::delete_custom_sound(&id, &in_use).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preview_sound(sound: String) {
    crate::sound::play_sound_value(&sound);
}

#[tauri::command]
pub fn list_system_sounds() -> Vec<String> {
    crate::sound::list_system_sounds()
}

#[tauri::command]
pub fn list_bundled_sounds() -> Vec<String> {
    crate::sound::list_bundled_sounds()
}
