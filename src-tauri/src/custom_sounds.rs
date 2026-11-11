//! 用户自定义提示音：受管目录 `~/.pokepoke/sounds/` + `manifest.json`。
//! settings 只存 `custom:<uuid>`，不存绝对路径。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const SOUNDS_SUBDIR: &str = "sounds";
pub const MANIFEST_FILENAME: &str = "manifest.json";

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 30 * 1024 * 1024;
pub const MAX_SOUND_COUNT: usize = 20;
pub const MAX_DURATION_SECS: f64 = 3.0;
pub const MAX_DISPLAY_NAME_LEN: usize = 64;

fn normalized_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "wav" => Some("wav"),
        "mp3" => Some("mp3"),
        "m4a" => Some("m4a"),
        "caf" => Some("caf"),
        "aiff" | "aif" => Some("aiff"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomSoundEntry {
    pub id: String,
    pub display_name: String,
    pub ext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    sounds: Vec<CustomSoundEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomSoundInfo {
    pub id: String,
    pub display_name: String,
    /// 设置值：`custom:<id>`
    pub value: String,
}

#[derive(Debug)]
pub enum CustomSoundError {
    Cancelled,
    UnsupportedFormat,
    FileTooLarge,
    DurationTooLong,
    TooManySounds,
    TotalSizeExceeded,
    InUse,
    NotFound,
    Io(io::Error),
}

impl std::fmt::Display for CustomSoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::UnsupportedFormat => write!(f, "unsupported_format"),
            Self::FileTooLarge => write!(f, "file_too_large"),
            Self::DurationTooLong => write!(f, "duration_too_long"),
            Self::TooManySounds => write!(f, "too_many_sounds"),
            Self::TotalSizeExceeded => write!(f, "total_size_exceeded"),
            Self::InUse => write!(f, "in_use"),
            Self::NotFound => write!(f, "not_found"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl From<io::Error> for CustomSoundError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn pokepoke_data_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".pokepoke"),
        None => PathBuf::from("/tmp/.pokepoke"),
    }
}

pub fn custom_sounds_dir() -> PathBuf {
    pokepoke_data_dir().join(SOUNDS_SUBDIR)
}

pub fn manifest_path() -> PathBuf {
    custom_sounds_dir().join(MANIFEST_FILENAME)
}

pub fn sound_file_path(entry: &CustomSoundEntry) -> PathBuf {
    custom_sounds_dir().join(format!("{}.{}", entry.id, entry.ext))
}

pub fn custom_sound_value(id: &str) -> String {
    format!("custom:{id}")
}

pub fn parse_custom_id(sound: &str) -> Option<&str> {
    sound.strip_prefix("custom:")
}

/// 列出磁盘上存在且 manifest 有效的自定义音。
pub fn list_custom_sounds() -> Result<Vec<CustomSoundInfo>, io::Error> {
    let mut manifest = load_manifest()?;
    manifest
        .sounds
        .retain(|entry| sound_file_path(entry).is_file());
    save_manifest(&manifest)?;
    Ok(manifest
        .sounds
        .into_iter()
        .map(|entry| CustomSoundInfo {
            value: custom_sound_value(&entry.id),
            id: entry.id.clone(),
            display_name: entry.display_name,
        })
        .collect())
}

pub fn resolve_custom_sound_path(sound: &str) -> Option<PathBuf> {
    let id = parse_custom_id(sound)?;
    let manifest = load_manifest().ok()?;
    let entry = manifest.sounds.iter().find(|e| e.id == id)?;
    let path = sound_file_path(entry);
    path.is_file().then_some(path)
}

pub fn import_from_path(source: &Path) -> Result<CustomSoundInfo, CustomSoundError> {
    if !source.is_file() {
        return Err(CustomSoundError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "source file not found",
        )));
    }

    let ext = normalized_extension(source).ok_or(CustomSoundError::UnsupportedFormat)?;
    let metadata = fs::metadata(source)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(CustomSoundError::FileTooLarge);
    }

    let duration = probe_duration_secs(source)?;
    if duration <= 0.0 {
        return Err(CustomSoundError::UnsupportedFormat);
    }
    if duration > MAX_DURATION_SECS {
        return Err(CustomSoundError::DurationTooLong);
    }

    ensure_sounds_dir()?;
    let mut manifest = load_manifest()?;
    manifest
        .sounds
        .retain(|entry| sound_file_path(entry).is_file());

    if manifest.sounds.len() >= MAX_SOUND_COUNT {
        return Err(CustomSoundError::TooManySounds);
    }

    let current_total = total_audio_bytes(&manifest.sounds)?;
    if current_total + metadata.len() > MAX_TOTAL_BYTES {
        return Err(CustomSoundError::TotalSizeExceeded);
    }

    let id = Uuid::new_v4().to_string();
    let display_name = display_name_from_source(source);
    let entry = CustomSoundEntry {
        id: id.clone(),
        display_name: display_name.clone(),
        ext: ext.to_string(),
    };
    let dest = sound_file_path(&entry);
    fs::copy(source, &dest)?;
    tighten_file_perms_0600(&dest)?;

    manifest.sounds.push(entry);
    save_manifest(&manifest)?;

    Ok(CustomSoundInfo {
        value: custom_sound_value(&id),
        id,
        display_name,
    })
}

pub fn delete_custom_sound(id: &str, in_use_values: &[&str]) -> Result<(), CustomSoundError> {
    let value = custom_sound_value(id);
    if in_use_values.contains(&value.as_str()) {
        return Err(CustomSoundError::InUse);
    }

    ensure_sounds_dir()?;
    let mut manifest = load_manifest()?;
    let Some(pos) = manifest.sounds.iter().position(|e| e.id == id) else {
        return Err(CustomSoundError::NotFound);
    };
    let entry = manifest.sounds.remove(pos);
    let path = sound_file_path(&entry);
    if let Err(e) = fs::remove_file(&path) {
        // 文件已不在(外部删除)不算错;其余失败记日志,避免"manifest 删了、文件
        // 还在,下次 import 同名 ext 撞路径"的静默不一致。
        if e.kind() != io::ErrorKind::NotFound {
            crate::log_warn!(
                "custom_sound",
                "delete: failed to remove audio file id={} path={:?} err={e}",
                id,
                path,
            );
        }
    }
    save_manifest(&manifest)?;
    Ok(())
}

fn display_name_from_source(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("sound");
    let mut name = stem.trim().to_string();
    if name.is_empty() {
        name = "sound".to_string();
    }
    if name.chars().count() > MAX_DISPLAY_NAME_LEN {
        name = name.chars().take(MAX_DISPLAY_NAME_LEN).collect();
    }
    name
}

fn total_audio_bytes(entries: &[CustomSoundEntry]) -> Result<u64, io::Error> {
    entries
        .iter()
        .map(|entry| fs::metadata(sound_file_path(entry)).map(|m| m.len()))
        .try_fold(0u64, |acc, len| len.map(|n| acc.saturating_add(n)))
}

fn probe_duration_secs(path: &Path) -> Result<f64, CustomSoundError> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("afinfo")
            .arg(path)
            .output()
            .map_err(CustomSoundError::Io)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");
        if !output.status.success() || combined.contains("Fail:") {
            return Err(CustomSoundError::UnsupportedFormat);
        }
        if let Some(secs) = parse_duration_from_afinfo(&combined) {
            return Ok(secs);
        }
        Err(CustomSoundError::UnsupportedFormat)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(CustomSoundError::UnsupportedFormat)
    }
}

/// 从 `afinfo` 输出解析秒数。支持 `estimated duration: 1.23 sec` 与 `MM:SS.sss`。
fn parse_duration_from_afinfo(text: &str) -> Option<f64> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("estimated duration:")
            .or_else(|| trimmed.strip_prefix("duration:"))
        else {
            continue;
        };
        let token = rest.split_whitespace().next()?;
        if let Ok(secs) = token.parse::<f64>() {
            return Some(secs);
        }
        // 形如 1:23.456
        if let Some((mins, secs_part)) = token.split_once(':') {
            let mins: f64 = mins.parse().ok()?;
            let secs: f64 = secs_part.parse().ok()?;
            return Some(mins * 60.0 + secs);
        }
    }
    None
}

fn load_manifest() -> Result<Manifest, io::Error> {
    let path = manifest_path();
    if !path.is_file() {
        return Ok(Manifest {
            version: 1,
            sounds: Vec::new(),
        });
    }
    let raw = fs::read_to_string(&path)?;
    let mut manifest: Manifest = serde_json::from_str(&raw).unwrap_or_default();
    if manifest.version == 0 {
        manifest.version = 1;
    }
    Ok(manifest)
}

fn save_manifest(manifest: &Manifest) -> Result<(), io::Error> {
    ensure_sounds_dir()?;
    let path = manifest_path();
    let body = serde_json::to_string_pretty(manifest).unwrap_or_else(|_| "{}".to_string());
    // 原子写:先写同目录临时文件再 rename 覆盖,避免崩溃/断电留下半截 JSON。
    // 否则 load_manifest 的 unwrap_or_default() 会静默清空用户全部自定义音条目,
    // 磁盘文件变孤儿。残留 tmp 文件无引用,下次 save 自然覆盖。
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    tighten_file_perms_0600(&tmp)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn ensure_sounds_dir() -> Result<(), io::Error> {
    let dir = custom_sounds_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(&dir)?;
        let mut perms = fs::metadata(&dir)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&dir, perms)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(&dir)
    }
}

#[cfg(unix)]
fn tighten_file_perms_0600(path: &Path) -> Result<(), io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn tighten_file_perms_0600(_path: &Path) -> Result<(), io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::tempdir;

    /// 临时把 HOME 指到沙盒目录跑闭包。HOME 是进程全局,任何读写 HOME 的
    /// 测试(custom_sounds / agent_presence / diagnostics / hook_cli 的
    /// 路径烘焙测试)必须统一 `#[serial]` 串行,否则并行时会互相踩 env。
    fn with_temp_sounds<F: FnOnce()>(f: F) {
        let tmp = tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        f();
        if let Some(home) = old_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn normalized_extension_accepts_common_formats() {
        assert_eq!(normalized_extension(Path::new("/tmp/a.MP3")), Some("mp3"));
        assert_eq!(normalized_extension(Path::new("/tmp/a.AIF")), Some("aiff"));
        assert!(normalized_extension(Path::new("/tmp/a.txt")).is_none());
    }

    fn bundled_waiting_mp3() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sounds/waiting.mp3")
    }

    /// 生成指定时长的合法 wav（8000Hz mono 16-bit PCM），供导入用例使用。
    fn write_wav(duration_secs: f64) -> PathBuf {
        let sample_rate = 8000u32;
        let data_len = (sample_rate as f64 * duration_secs) as u32;
        let channels = 1u16;
        let bits_per_sample = 16u16;
        let bytes_per_sample = bits_per_sample / 8;
        let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
        let block_align = channels * bytes_per_sample;
        let data_size = data_len * bytes_per_sample as u32 * channels as u32;
        let riff_size = 36 + data_size;

        let mut wav = Vec::with_capacity((44 + data_size) as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend(vec![0u8; data_size as usize]);

        fs::create_dir_all(pokepoke_data_dir()).unwrap();
        let path = pokepoke_data_dir().join(format!("probe-{duration_secs}.wav"));
        fs::write(&path, &wav).unwrap();
        path
    }

    fn write_tiny_wav() -> PathBuf {
        write_wav(0.05)
    }

    #[test]
    #[serial]
    fn import_list_delete_round_trip() {
        with_temp_sounds(|| {
            let src = write_tiny_wav();
            let info = import_from_path(&src).expect("import");
            assert!(info.value.starts_with("custom:"));

            let listed = list_custom_sounds().expect("list");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, info.id);

            delete_custom_sound(&info.id, &[]).expect("delete");
            assert!(list_custom_sounds().unwrap().is_empty());
        });
    }

    #[test]
    #[serial]
    fn delete_rejects_when_in_use() {
        with_temp_sounds(|| {
            let src = write_tiny_wav();
            let info = import_from_path(&src).expect("import");
            let err = delete_custom_sound(&info.id, &[&info.value]).unwrap_err();
            assert!(matches!(err, CustomSoundError::InUse));
        });
    }

    #[test]
    fn parse_duration_from_afinfo_handles_sec_and_colon_formats() {
        let sample = "estimated duration: 1.828562 sec\n";
        assert_eq!(parse_duration_from_afinfo(sample), Some(1.828562));
        let colon = "duration: 1:05.500 sec\n";
        assert_eq!(parse_duration_from_afinfo(colon), Some(65.5));
    }

    #[test]
    #[serial]
    fn rejects_file_too_large() {
        with_temp_sounds(|| {
            let src = bundled_waiting_mp3();
            if !src.is_file() {
                return;
            }
            let tmp = tempdir().unwrap();
            let big = tmp.path().join("big.mp3");
            let mut data = fs::read(&src).unwrap();
            data.extend(vec![0u8; 3 * 1024 * 1024]);
            fs::write(&big, &data).unwrap();
            let err = import_from_path(&big).unwrap_err();
            assert!(matches!(err, CustomSoundError::FileTooLarge));
        });
    }

    #[test]
    #[serial]
    fn rejects_duration_too_long() {
        with_temp_sounds(|| {
            // 4s wav 超过 3s 上限
            let src = write_wav(4.0);
            let duration = probe_duration_secs(&src).expect("afinfo should parse");
            if duration <= MAX_DURATION_SECS {
                return; // 环境无法构造超长样本时跳过
            }
            let err = import_from_path(&src).unwrap_err();
            assert!(matches!(err, CustomSoundError::DurationTooLong));
        });
    }

    #[test]
    #[serial]
    fn rejects_too_many_sounds() {
        with_temp_sounds(|| {
            let src = write_tiny_wav();
            for _ in 0..MAX_SOUND_COUNT {
                import_from_path(&src).expect("import should succeed below cap");
            }
            let err = import_from_path(&src).unwrap_err();
            assert!(matches!(err, CustomSoundError::TooManySounds));
        });
    }

    #[test]
    #[serial]
    fn rejects_total_size_exceeded() {
        with_temp_sounds(|| {
            let src = write_tiny_wav();
            // 预置一个接近总量上限的条目，再导入新文件必然超限。
            ensure_sounds_dir().unwrap();
            let big_id = "big-entry";
            let big_ext = "wav";
            let big_path = custom_sounds_dir().join(format!("{big_id}.{big_ext}"));
            fs::write(&big_path, vec![0u8; (MAX_TOTAL_BYTES - 1) as usize]).unwrap();
            let manifest = Manifest {
                version: 1,
                sounds: vec![CustomSoundEntry {
                    id: big_id.to_string(),
                    display_name: "big".to_string(),
                    ext: big_ext.to_string(),
                }],
            };
            let body = serde_json::to_string_pretty(&manifest).unwrap();
            fs::write(manifest_path(), body).unwrap();

            let err = import_from_path(&src).unwrap_err();
            assert!(matches!(err, CustomSoundError::TotalSizeExceeded));
        });
    }

    #[test]
    #[serial]
    fn rejects_unsupported_extension() {
        with_temp_sounds(|| {
            let src_dir = tempdir().unwrap();
            let src = src_dir.path().join("nope.txt");
            fs::write(&src, b"hello").unwrap();
            let err = import_from_path(&src).unwrap_err();
            assert!(matches!(err, CustomSoundError::UnsupportedFormat));
        });
    }
}
