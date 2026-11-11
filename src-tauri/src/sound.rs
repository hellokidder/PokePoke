use crate::custom_sounds;
use crate::sessions::SessionStatus;
use crate::settings::SettingsStore;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

// Trait 抽象点（Layer 1 测试自动化新增）：让 http_server 的 integration
// 测试可以注入 spy SoundPlayer，避免真正调 afplay。
// 生产路径走 AppSoundPlayer，是 play_alert_with_settings 的薄包装。
pub trait SoundPlayer: Send + Sync {
    fn play_alert(&self, status: SessionStatus);
}

pub struct AppSoundPlayer {
    settings: Arc<Mutex<SettingsStore>>,
}

impl AppSoundPlayer {
    pub fn new(settings: Arc<Mutex<SettingsStore>>) -> Self {
        Self { settings }
    }
}

impl SoundPlayer for AppSoundPlayer {
    fn play_alert(&self, status: SessionStatus) {
        play_alert_with_settings(&self.settings, status);
    }
}

/// 内置提示音 id（不含 `builtin:` 前缀），与 `src-tauri/sounds/<name>.mp3` 一一对应。
pub const BUNDLED_SOUND_NAMES: &[&str] = &["waiting", "completed", "failed"];

static BUNDLED_SOUNDS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 在 app setup 时注入打包资源目录；未注入时回落到源码目录（dev / 单测）。
pub fn init_bundled_sounds_dir(dir: PathBuf) {
    let _ = BUNDLED_SOUNDS_DIR.set(dir);
}

fn bundled_sounds_dir() -> PathBuf {
    BUNDLED_SOUNDS_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/sounds")))
}

fn builtin_fallback_for_status(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Pending => "builtin:waiting",
        SessionStatus::Idle | SessionStatus::Running => "builtin:completed",
        SessionStatus::LastFailed => "builtin:failed",
    }
}

/// Play the alert sound for a reminder status based on current settings.
pub fn play_alert_with_settings(settings_store: &Arc<Mutex<SettingsStore>>, status: SessionStatus) {
    let fallback = builtin_fallback_for_status(status.clone());
    let sound = {
        let s = settings_store.lock().unwrap_or_else(|p| p.into_inner());
        s.settings.sound_for_status(status).to_string()
    };

    if play_sound_str(&sound) {
        return;
    }

    if custom_sounds::parse_custom_id(&sound).is_some() {
        crate::log_warn!(
            "sound",
            "custom sound missing, falling back to {fallback} for configured {sound}"
        );
        let _ = play_sound_str(fallback);
    } else if sound != "mute" {
        // system:/builtin: 解析失败(macOS 标准系统音几乎恒在,缺多为打包资源
        // 丢失)。静默会让人误以为"没设提醒",补一条 WARN 便于诊断。
        crate::log_warn!(
            "sound",
            "unresolved sound value {sound} (missing file?), no fallback for non-custom"
        );
    }
}

/// Play a configured sound value for preview purposes.
pub fn play_sound_value(sound: &str) {
    play_sound_str(sound);
}

/// Play a named system sound for preview purposes.
pub fn play_sound_by_name(name: &str) {
    play_sound_str(&format!("system:{name}"));
}

fn resolve_sound_path(sound: &str) -> Option<String> {
    if sound == "mute" {
        return None;
    }

    if let Some(name) = sound.strip_prefix("builtin:") {
        let path = bundled_sounds_dir().join(format!("{name}.mp3"));
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
        return None;
    }

    if let Some(name) = sound.strip_prefix("system:") {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        if std::path::Path::new(&path).is_file() {
            return Some(path);
        }
        return None;
    }

    if custom_sounds::parse_custom_id(sound).is_some() {
        return custom_sounds::resolve_custom_sound_path(sound)
            .map(|p| p.to_string_lossy().into_owned());
    }

    None
}

/// Returns true when a playable path was resolved and playback was spawned.
fn play_sound_str(sound: &str) -> bool {
    let Some(path) = resolve_sound_path(sound) else {
        return false;
    };

    let path_for_log = path.clone();
    std::thread::spawn(move || {
        warm_up_output_device();
        match std::process::Command::new("afplay").arg(&path).output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => crate::log_warn!(
                "sound",
                "afplay failed path={} exit={:?}",
                path_for_log,
                out.status.code(),
            ),
            Err(err) => crate::log_warn!(
                "sound",
                "afplay spawn failed path={} err={}",
                path_for_log,
                err,
            ),
        }
    });
    true
}

/// 真实音效前推一段 ~100ms 近静音 pre-roll,唤醒空闲的输出设备。
///
/// macOS 输出设备在几秒无音频后进入低功耗休眠(蓝牙 A2DP 最严重);`afplay`
/// 从文件第一个采样点直接起播,设备「醒来」期间头 ~100ms 采样被直接丢弃,
/// 短提示音听感就是「开头没了」。pre-roll 让真实声音落在已激活的流上。
/// 资产缺失时静默跳过,退化为旧行为(不 panic、不报错)。
fn warm_up_output_device() {
    let preroll = bundled_sounds_dir().join("preroll.wav");
    if !preroll.is_file() {
        return;
    }
    let _ = std::process::Command::new("afplay").arg(&preroll).status();
}

/// List available system sound names (without extension).
pub fn list_system_sounds() -> Vec<String> {
    let dir = std::path::Path::new("/System/Library/Sounds");
    let mut sounds: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".aiff").map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    sounds.sort();
    sounds
}

/// List built-in sound ids (without `builtin:` prefix).
pub fn list_bundled_sounds() -> Vec<String> {
    BUNDLED_SOUND_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::resolve_sound_path;

    #[test]
    fn resolve_sound_path_returns_system_sound_path() {
        assert_eq!(
            resolve_sound_path("system:Glass").as_deref(),
            Some("/System/Library/Sounds/Glass.aiff"),
        );
    }

    #[test]
    fn resolve_sound_path_returns_builtin_sound_path() {
        let path = resolve_sound_path("builtin:waiting").expect("path");
        assert!(path.ends_with("sounds/waiting.mp3"));
    }

    #[test]
    fn resolve_sound_path_returns_none_for_mute() {
        assert_eq!(resolve_sound_path("mute"), None);
    }

    #[test]
    fn resolve_sound_path_returns_none_for_unknown_format() {
        assert_eq!(resolve_sound_path("custom:missing-id"), None);
        assert_eq!(resolve_sound_path("weird:value"), None);
    }
}
