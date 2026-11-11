// Write-then-verify L1(`docs/briefs/20260507-write-then-verify-l1.md`)
//
// install/uninstall 写完外部配置后立刻调本模块,确认宿主能加载;失败由
// 调用方触发回滚 + 返回 error JSON。
//
// 三宿主能力差异:
// - **codex**:有 `codex login status` 这种轻量子命令会真实加载 config.toml
//   + hooks.json,坏 schema 时 stderr 出现 `Error loading configuration`。
//   exit code 不能用(login 状态本身也会 exit=1),只看 stderr 文本判定。
//   覆盖**我方写错 + 上游 schema 漂移**两类。
// - **cc / cursor**:没找到纯 schema 验证 CLI(`claude doctor` 是 TUI;
//   `--bare` 跳过 hooks;cursor 是 GUI 包装)。退化为 serde 镜像 schema —
//   只挡我方 typo / 程序员手抖,**不挡上游 schema 漂移**。L2 + 真启动测试
//   是 cc/cursor 上游漂移的补位手段。

use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// codex CLI 探测的判定结果。
enum CodexProbeOutcome {
    /// codex 真实加载 config 没报 schema 错(可能 not logged in,但 schema 通过)
    Ok,
    /// stderr 出现 `Error loading configuration` → schema 坏
    SchemaError(String),
    /// codex CLI 不存在 / 启动失败 / 其他不可判定状态 → 调用方降级到 schema mirror
    Inconclusive,
}

/// 找 codex binary。先 PATH,再 几个常见 fnm/nvm 路径。
fn find_codex_binary() -> Option<String> {
    let candidates = [
        "codex".to_string(),
        format!(
            "{}/.local/share/fnm/node-versions",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];
    // 用 PATH lookup `which codex` 风格
    if let Ok(output) = Command::new("/usr/bin/which").arg("codex").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    let _ = candidates;
    None
}

/// `codex login status` 的硬超时。正常该子命令是轻量本地加载(读 config.toml +
/// hooks.json),不该慢;但 codex CLI 未来若加网络校验、或卡在凭证刷新,裸
/// `.output()` 会无限阻塞 —— 而 `probe_codex_cli` 经 `cmd_install_codex` →
/// `bootstrap.rs` 首启在 **Tauri setup 同步路径** 上跑,挂住 = 首启窗口永不出现。
/// 10s 覆盖慢机,又不让挂死拖垮启动。对齐 `osascript.rs` 的 spawn+poll+kill 思路。
const CODEX_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CODEX_PROBE_POLL: Duration = Duration::from_millis(20);

/// 跑 `codex login status`,最多等 `CODEX_PROBE_TIMEOUT`。返回 `(stdout, stderr)`。
///
/// 不复用 `crate::osascript::run_command_with_timeout`:那个只捕 stdout、stderr
/// 直接 `Stdio::null()`;codex 探测要看 stderr 里的 `Error loading configuration`,
/// 双流都必须抓。超时 / spawn 失败 → `None`(调用方降级到 `Inconclusive` → schema mirror)。
fn run_codex_login_status(codex_bin: &str) -> Option<(String, String)> {
    let mut cmd = Command::new(codex_bin);
    cmd.args(["login", "status"]);
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // exit code 不用:login 状态本身也会 exit=1,只看 stderr 文本。
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut s) = stdout_pipe.take() {
                    let _ = s.read_to_string(&mut stdout);
                }
                if let Some(mut s) = stderr_pipe.take() {
                    let _ = s.read_to_string(&mut stderr);
                }
                return Some((stdout, stderr));
            }
            Ok(None) => {
                if start.elapsed() >= CODEX_PROBE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!(
                        "verify_codex: `codex login status` timed out after {}ms, falling back to schema mirror",
                        CODEX_PROBE_TIMEOUT.as_millis()
                    );
                    return None;
                }
                std::thread::sleep(CODEX_PROBE_POLL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// 跑 `codex login status`,根据输出判定 config schema 是否可加载。
/// 不在乎 login 状态本身,只看有没有 schema error 字样。
fn probe_codex_cli() -> CodexProbeOutcome {
    let Some(codex_bin) = find_codex_binary() else {
        return CodexProbeOutcome::Inconclusive;
    };
    let Some((stdout, stderr)) = run_codex_login_status(&codex_bin) else {
        // 超时 / spawn 失败 → 无法判定,降级到 schema mirror。
        return CodexProbeOutcome::Inconclusive;
    };
    let combined = format!("{}{}", stdout, stderr);
    if combined.contains("Error loading configuration") || combined.contains("Error loading config")
    {
        // 截短后传给上层,避免日志过长
        let snippet = combined
            .lines()
            .find(|l| l.contains("Error loading"))
            .unwrap_or("Error loading configuration")
            .trim()
            .chars()
            .take(300)
            .collect::<String>();
        CodexProbeOutcome::SchemaError(snippet)
    } else {
        CodexProbeOutcome::Ok
    }
}

/// codex 写后回环验证。先尝试 CLI 探测,降级到 schema mirror。
pub fn verify_codex_writes(codex_dir: &Path) -> Result<(), String> {
    match probe_codex_cli() {
        CodexProbeOutcome::Ok => return Ok(()),
        CodexProbeOutcome::SchemaError(msg) => {
            return Err(format!("codex CLI probe rejected config: {}", msg));
        }
        CodexProbeOutcome::Inconclusive => {
            eprintln!("verify_codex: CLI probe inconclusive, falling back to schema mirror");
        }
    }
    // Fallback: schema mirror on hooks.json
    let hooks_path = codex_dir.join("hooks.json");
    if hooks_path.exists() {
        verify_codex_hooks_schema(&hooks_path)?;
    }
    Ok(())
}

fn verify_codex_hooks_schema(hooks_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(hooks_path).map_err(|e| format!("read hooks.json: {}", e))?;
    let v: Value =
        serde_json::from_str(&content).map_err(|e| format!("parse hooks.json: {}", e))?;
    // codex hooks.json 与 cc settings.json["hooks"] 结构相同 → 复用同一校验
    let hooks = v
        .get("hooks")
        .ok_or_else(|| "codex hooks.json missing top-level `hooks` key".to_string())?;
    validate_event_hooks_shape(hooks, "codex")
}

/// kimi config.toml 写后回环验证。优先跑 `kimi doctor config <path>`(kimi 专属
/// schema 校验子命令,会真实加载 config 并报告 providers/models/hooks 错误;
/// 真机验证 exit 0=valid / exit 1=invalid,exit code 可靠不像 codex login status)。
/// kimi 不在或超时则降级到 TOML 解析镜像(只挡我方写坏 + TOML 语法)。
pub fn verify_kimi_writes(config_path: &Path) -> Result<(), String> {
    if let Some(kimi_bin) = find_kimi_binary() {
        match probe_kimi_doctor(&kimi_bin, config_path) {
            KimiProbeOutcome::Ok => return Ok(()),
            KimiProbeOutcome::SchemaError(msg) => {
                return Err(format!("kimi doctor config rejected: {}", msg));
            }
            KimiProbeOutcome::Inconclusive => {
                eprintln!("verify_kimi: CLI probe inconclusive, falling back to TOML mirror");
            }
        }
    }
    verify_kimi_config_schema(config_path)
}

/// kimi CLI 探测的判定结果。
enum KimiProbeOutcome {
    /// `kimi doctor config` exit 0
    Ok,
    /// exit != 0,config schema 坏(stderr 含 ERROR / Invalid / Validation)
    SchemaError(String),
    /// kimi 不存在 / 启动失败 / 超时 -> 降级到 TOML 镜像
    Inconclusive,
}

const KIMI_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const KIMI_PROBE_POLL: Duration = Duration::from_millis(20);

/// 找 kimi binary。先 `<kimi_home>/bin/kimi`(默认安装位;GUI 启动 PokePoke 时
/// PATH 里通常没有 kimi),再 PATH `which kimi`。honor `KIMI_CODE_HOME`。
fn find_kimi_binary() -> Option<String> {
    let kimi_home = std::env::var("KIMI_CODE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".kimi-code")
        });
    let local = kimi_home.join("bin").join("kimi");
    if local.is_file() {
        return Some(local.to_string_lossy().into_owned());
    }
    if let Ok(output) = Command::new("/usr/bin/which").arg("kimi").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    None
}

/// 跑 `kimi doctor config <path>`,最多等 `KIMI_PROBE_TIMEOUT`。
/// exit 0 -> Ok;exit != 0 -> SchemaError(stderr 摘要);spawn/超时 -> Inconclusive。
fn probe_kimi_doctor(kimi_bin: &str, config_path: &Path) -> KimiProbeOutcome {
    let mut cmd = Command::new(kimi_bin);
    cmd.args(["doctor", "config"]).arg(config_path);
    let start = Instant::now();
    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return KimiProbeOutcome::Inconclusive,
    };
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut s) = stdout_pipe.take() {
                    let _ = s.read_to_string(&mut stdout);
                }
                if let Some(mut s) = stderr_pipe.take() {
                    let _ = s.read_to_string(&mut stderr);
                }
                if status.success() {
                    return KimiProbeOutcome::Ok;
                }
                let combined = format!("{}{}", stdout, stderr);
                let snippet = combined
                    .lines()
                    .find(|l| {
                        l.contains("ERROR") || l.contains("Invalid") || l.contains("Validation")
                    })
                    .unwrap_or("config invalid")
                    .trim()
                    .chars()
                    .take(300)
                    .collect::<String>();
                return KimiProbeOutcome::SchemaError(snippet);
            }
            Ok(None) => {
                if start.elapsed() >= KIMI_PROBE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!(
                        "verify_kimi: `kimi doctor config` timed out after {}ms, falling back to TOML mirror",
                        KIMI_PROBE_TIMEOUT.as_millis()
                    );
                    return KimiProbeOutcome::Inconclusive;
                }
                std::thread::sleep(KIMI_PROBE_POLL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return KimiProbeOutcome::Inconclusive;
            }
        }
    }
}

/// TOML 解析镜像:只挡我方写坏 + TOML 语法,不挡 kimi 上游 schema 漂移(那是
/// `kimi doctor config` 探测的职责)。`[[hooks]]` 每条至少要有 event + command。
fn verify_kimi_config_schema(config_path: &Path) -> Result<(), String> {
    if !config_path.exists() {
        // uninstall 后文件可能整个被删 / 从未创建
        return Ok(());
    }
    let content =
        fs::read_to_string(config_path).map_err(|e| format!("read config.toml: {}", e))?;
    let doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("parse config.toml: {}", e))?;
    if let Some(arr) = doc.get("hooks").and_then(|h| h.as_array_of_tables()) {
        for (i, t) in arr.iter().enumerate() {
            if t.get("event").and_then(|v| v.as_str()).is_none() {
                return Err(format!("hooks[{i}] missing `event` string"));
            }
            if t.get("command").and_then(|v| v.as_str()).is_none() {
                return Err(format!("hooks[{i}] missing `command` string"));
            }
        }
    }
    Ok(())
}

/// cc settings.json 写后 schema 镜像校验。`hooks` 字段是可选的。
pub fn verify_cc_writes(settings_path: &Path) -> Result<(), String> {
    if !settings_path.exists() {
        // uninstall 后可能 settings 还在但 hooks 字段空 → 文件不存在不算错
        return Ok(());
    }
    let content =
        fs::read_to_string(settings_path).map_err(|e| format!("read settings.json: {}", e))?;
    if content.trim().is_empty() {
        return Ok(());
    }
    let v: Value =
        serde_json::from_str(&content).map_err(|e| format!("parse settings.json: {}", e))?;
    let Some(hooks) = v.get("hooks") else {
        return Ok(());
    };
    validate_event_hooks_shape(hooks, "cc")
}

/// cursor hooks.json 写后 schema 镜像校验。结构与 cc/codex 不同:
/// `{"version": 1, "hooks": {EVENT: [{"command": str, "timeout"?: int}]}}`
pub fn verify_cursor_writes(hooks_path: &Path) -> Result<(), String> {
    if !hooks_path.exists() {
        // uninstall 把空 hooks 文件留下来或删掉都 ok
        return Ok(());
    }
    let content =
        fs::read_to_string(hooks_path).map_err(|e| format!("read cursor hooks.json: {}", e))?;
    if content.trim().is_empty() {
        return Ok(());
    }
    let v: Value =
        serde_json::from_str(&content).map_err(|e| format!("parse cursor hooks.json: {}", e))?;
    if v.get("version").and_then(|x| x.as_u64()) != Some(1) {
        return Err("cursor hooks.json missing `version: 1`".to_string());
    }
    let Some(hooks) = v.get("hooks") else {
        return Ok(());
    };
    let map = hooks
        .as_object()
        .ok_or_else(|| "cursor hooks must be object".to_string())?;
    for (event, arr) in map {
        let entries = arr.as_array().ok_or_else(|| {
            format!(
                "cursor hooks.{} must be array, got {}",
                event,
                type_name(arr)
            )
        })?;
        for (i, entry) in entries.iter().enumerate() {
            let obj = entry.as_object().ok_or_else(|| {
                format!(
                    "cursor hooks.{}[{}] must be object, got {}",
                    event,
                    i,
                    type_name(entry)
                )
            })?;
            if !obj.get("command").is_some_and(|c| c.is_string()) {
                return Err(format!(
                    "cursor hooks.{}[{}] missing `command` string",
                    event, i
                ));
            }
        }
    }
    Ok(())
}

/// grok `~/.grok/hooks/pokepoke.json` 写后 schema 镜像校验。
pub fn verify_grok_writes(hooks_path: &Path) -> Result<(), String> {
    if !hooks_path.exists() {
        return Ok(());
    }
    let content =
        fs::read_to_string(hooks_path).map_err(|e| format!("read grok hook file: {}", e))?;
    if content.trim().is_empty() {
        return Ok(());
    }
    if !content.contains("PokePoke grok integration") {
        return Err("grok hook file missing PokePoke marker".to_string());
    }
    if !content.contains("--source=grok") {
        return Err("grok hook file missing poke-hook invocation".to_string());
    }
    let v: Value =
        serde_json::from_str(&content).map_err(|e| format!("parse grok hook file: {}", e))?;
    let Some(hooks) = v.get("hooks") else {
        return Err("grok hook file missing `hooks` object".to_string());
    };
    validate_event_hooks_shape(hooks, "grok")
}

/// opencode plugin 写后校验。opencode plugin 是 JS 文件,没有 schema CLI 可探测
/// (opencode 是 Bun 运行时,加载失败只在 opencode 启动时报),退化为最小内容镜像:
/// 确认文件存在、非空、含 PokePoke 标记 + `--source=opencode`。只挡我方 typo /
/// 写空,不挡 opencode 上游 plugin API 漂移(与 cc/cursor 同为镜像级)。
pub fn verify_opencode_writes(plugin_path: &Path) -> Result<(), String> {
    if !plugin_path.exists() {
        // uninstall 删文件是合法终态
        return Ok(());
    }
    let content =
        fs::read_to_string(plugin_path).map_err(|e| format!("read opencode plugin: {}", e))?;
    if content.trim().is_empty() {
        return Err("opencode plugin is empty".to_string());
    }
    if !content.contains("PokePoke opencode integration") {
        return Err("opencode plugin missing PokePoke marker".to_string());
    }
    if !content.contains("--source=opencode") {
        return Err("opencode plugin missing poke-hook invocation".to_string());
    }
    Ok(())
}

/// pi 扩展落点校验:文件存在、非空、含 PokePoke 标记与 `--source=pi` 调用、
/// 且 `__POKE_HOOK_BIN__` 占位符已被实际 hook 路径替换(install 时漏替换会被拦下)。
pub fn verify_pi_writes(ext_path: &Path) -> Result<(), String> {
    if !ext_path.exists() {
        // uninstall 删目录是合法终态
        return Ok(());
    }
    let content = fs::read_to_string(ext_path).map_err(|e| format!("read pi extension: {}", e))?;
    if content.trim().is_empty() {
        return Err("pi extension is empty".to_string());
    }
    if !content.contains("PokePoke pi integration") {
        return Err("pi extension missing PokePoke marker".to_string());
    }
    if !content.contains("--source=pi") {
        return Err("pi extension missing poke-hook invocation".to_string());
    }
    if content.contains("__POKE_HOOK_BIN__") {
        return Err("pi extension has unresolved hook path placeholder".to_string());
    }
    Ok(())
}

/// omp 扩展落点校验:文件存在、非空、含 PokePoke omp 标记与 `--source=omp` 调用、
/// 且 `__POKE_HOOK_BIN__` 占位符已被实际 hook 路径替换(install 时漏替换会被拦下)。
pub fn verify_omp_writes(ext_path: &Path) -> Result<(), String> {
    if !ext_path.exists() {
        // uninstall 删目录是合法终态
        return Ok(());
    }
    let content = fs::read_to_string(ext_path).map_err(|e| format!("read omp extension: {}", e))?;
    if content.trim().is_empty() {
        return Err("omp extension is empty".to_string());
    }
    if !content.contains("PokePoke omp integration") {
        return Err("omp extension missing PokePoke marker".to_string());
    }
    if !content.contains("--source=omp") {
        return Err("omp extension missing poke-hook invocation".to_string());
    }
    if content.contains("__POKE_HOOK_BIN__") {
        return Err("omp extension has unresolved hook path placeholder".to_string());
    }
    Ok(())
}

/// cc/codex 共用的 hooks shape:
/// `{EVENT: [{"matcher"?: str, "hooks": [{"type": "command", "command": str}]}]}`
fn validate_event_hooks_shape(hooks: &Value, source_label: &str) -> Result<(), String> {
    let map = hooks.as_object().ok_or_else(|| {
        format!(
            "{} hooks must be object, got {}",
            source_label,
            type_name(hooks)
        )
    })?;
    for (event, groups) in map {
        let arr = groups.as_array().ok_or_else(|| {
            format!(
                "{} hooks.{} must be array, got {}",
                source_label,
                event,
                type_name(groups)
            )
        })?;
        for (gi, group) in arr.iter().enumerate() {
            let group_obj = group.as_object().ok_or_else(|| {
                format!(
                    "{} hooks.{}[{}] must be object, got {}",
                    source_label,
                    event,
                    gi,
                    type_name(group)
                )
            })?;
            let inner = group_obj
                .get("hooks")
                .and_then(|h| h.as_array())
                .ok_or_else(|| {
                    format!(
                        "{} hooks.{}[{}].hooks must be array",
                        source_label, event, gi
                    )
                })?;
            for (hi, entry) in inner.iter().enumerate() {
                let entry_obj = entry.as_object().ok_or_else(|| {
                    format!(
                        "{} hooks.{}[{}].hooks[{}] must be object, got {}",
                        source_label,
                        event,
                        gi,
                        hi,
                        type_name(entry)
                    )
                })?;
                if entry_obj.get("type").and_then(|t| t.as_str()) != Some("command") {
                    return Err(format!(
                        "{} hooks.{}[{}].hooks[{}] missing `type: \"command\"`",
                        source_label, event, gi, hi
                    ));
                }
                if !entry_obj.get("command").is_some_and(|c| c.is_string()) {
                    return Err(format!(
                        "{} hooks.{}[{}].hooks[{}] missing `command` string",
                        source_label, event, gi, hi
                    ));
                }
            }
        }
    }
    Ok(())
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_tmp(name: &str, content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), content).unwrap();
        dir
    }

    #[test]
    fn cc_verify_passes_on_valid_hooks_shape() {
        let dir = write_tmp(
            "settings.json",
            &json!({
                "hooks": {
                    "SessionStart": [{
                        "hooks": [{"type": "command", "command": "/path/to/hook"}]
                    }]
                }
            })
            .to_string(),
        );
        assert!(verify_cc_writes(&dir.path().join("settings.json")).is_ok());
    }

    #[test]
    fn cc_verify_passes_when_no_hooks_key() {
        let dir = write_tmp("settings.json", &json!({"model": "opus"}).to_string());
        assert!(verify_cc_writes(&dir.path().join("settings.json")).is_ok());
    }

    #[test]
    fn cc_verify_passes_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        // 文件不存在视为 ok(uninstall 可能整个删掉)
        assert!(verify_cc_writes(&dir.path().join("settings.json")).is_ok());
    }

    #[test]
    fn cc_verify_rejects_hooks_as_string() {
        let dir = write_tmp("settings.json", r#"{"hooks": "broken"}"#);
        let err = verify_cc_writes(&dir.path().join("settings.json")).unwrap_err();
        assert!(err.contains("must be object"), "err={}", err);
    }

    #[test]
    fn cc_verify_rejects_event_value_as_object() {
        // SessionStart 应该是 array,不是单个 object
        let dir = write_tmp(
            "settings.json",
            r#"{"hooks": {"SessionStart": {"hooks": []}}}"#,
        );
        let err = verify_cc_writes(&dir.path().join("settings.json")).unwrap_err();
        assert!(err.contains("must be array"), "err={}", err);
    }

    #[test]
    fn cc_verify_rejects_command_entry_missing_type() {
        let dir = write_tmp(
            "settings.json",
            r#"{"hooks": {"Stop": [{"hooks": [{"command": "/path"}]}]}}"#,
        );
        let err = verify_cc_writes(&dir.path().join("settings.json")).unwrap_err();
        assert!(err.contains("`type: \"command\"`"), "err={}", err);
    }

    #[test]
    fn cc_verify_rejects_invalid_json() {
        let dir = write_tmp("settings.json", "{not json");
        let err = verify_cc_writes(&dir.path().join("settings.json")).unwrap_err();
        assert!(err.contains("parse settings.json"), "err={}", err);
    }

    #[test]
    fn cursor_verify_passes_on_valid_shape() {
        let dir = write_tmp(
            "hooks.json",
            &json!({
                "version": 1,
                "hooks": {
                    "sessionStart": [{"command": "/path", "timeout": 8}]
                }
            })
            .to_string(),
        );
        assert!(verify_cursor_writes(&dir.path().join("hooks.json")).is_ok());
    }

    #[test]
    fn cursor_verify_passes_with_just_version_no_hooks() {
        let dir = write_tmp("hooks.json", r#"{"version": 1}"#);
        assert!(verify_cursor_writes(&dir.path().join("hooks.json")).is_ok());
    }

    #[test]
    fn cursor_verify_rejects_missing_version() {
        let dir = write_tmp("hooks.json", r#"{"hooks": {}}"#);
        let err = verify_cursor_writes(&dir.path().join("hooks.json")).unwrap_err();
        assert!(err.contains("version: 1"), "err={}", err);
    }

    #[test]
    fn cursor_verify_rejects_wrong_version() {
        let dir = write_tmp("hooks.json", r#"{"version": 2, "hooks": {}}"#);
        let err = verify_cursor_writes(&dir.path().join("hooks.json")).unwrap_err();
        assert!(err.contains("version: 1"), "err={}", err);
    }

    #[test]
    fn cursor_verify_rejects_command_missing() {
        let dir = write_tmp(
            "hooks.json",
            r#"{"version": 1, "hooks": {"stop": [{"timeout": 8}]}}"#,
        );
        let err = verify_cursor_writes(&dir.path().join("hooks.json")).unwrap_err();
        assert!(err.contains("missing `command`"), "err={}", err);
    }

    #[test]
    fn codex_schema_mirror_rejects_string_hooks_root() {
        let dir = write_tmp("hooks.json", r#"{"hooks": "./hooks.json"}"#);
        let err = verify_codex_hooks_schema(&dir.path().join("hooks.json")).unwrap_err();
        assert!(err.contains("must be object"), "err={}", err);
    }

    #[test]
    fn codex_schema_mirror_passes_on_valid_shape() {
        let dir = write_tmp(
            "hooks.json",
            &json!({
                "hooks": {
                    "SessionStart": [{
                        "hooks": [{"type": "command", "command": "/path"}]
                    }]
                }
            })
            .to_string(),
        );
        assert!(verify_codex_hooks_schema(&dir.path().join("hooks.json")).is_ok());
    }

    const VALID_PI_EXTENSION: &str = r#"// PokePoke pi integration - auto-generated by `poke-hook --install-pi`.
const POKE_HOOK = "/usr/local/bin/pokehook --source=pi";
"#;

    #[test]
    fn pi_verify_passes_on_valid_extension() {
        let dir = write_tmp("index.ts", VALID_PI_EXTENSION);
        assert!(verify_pi_writes(&dir.path().join("index.ts")).is_ok());
    }

    #[test]
    fn pi_verify_passes_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(verify_pi_writes(&dir.path().join("index.ts")).is_ok());
    }

    #[test]
    fn pi_verify_rejects_empty_file() {
        let dir = write_tmp("index.ts", "   \n");
        let err = verify_pi_writes(&dir.path().join("index.ts")).unwrap_err();
        assert!(err.contains("empty"), "err={}", err);
    }

    #[test]
    fn pi_verify_rejects_unresolved_placeholder() {
        let dir = write_tmp(
            "index.ts",
            r#"// PokePoke pi integration
const POKE_HOOK = "__POKE_HOOK_BIN__ --source=pi";
"#,
        );
        let err = verify_pi_writes(&dir.path().join("index.ts")).unwrap_err();
        assert!(err.contains("placeholder"), "err={}", err);
    }

    #[test]
    fn pi_verify_rejects_missing_source_arg() {
        // 有 PokePoke marker 但缺 --source=pi 调用 -> 拒绝
        let dir = write_tmp(
            "index.ts",
            r#"// PokePoke pi integration
const POKE_HOOK = "/usr/local/bin/pokehook";
"#,
        );
        let err = verify_pi_writes(&dir.path().join("index.ts")).unwrap_err();
        assert!(err.contains("missing poke-hook invocation"), "err={}", err);
    }

    #[test]
    fn pi_verify_rejects_missing_marker() {
        let dir = write_tmp(
            "index.ts",
            r#"const POKE_HOOK = "/usr/local/bin/pokehook --source=pi";"#,
        );
        let err = verify_pi_writes(&dir.path().join("index.ts")).unwrap_err();
        assert!(err.contains("missing PokePoke marker"), "err={}", err);
    }

    #[test]
    fn kimi_verify_passes_on_valid_config_with_hooks() {
        let dir = write_tmp(
            "config.toml",
            "default_model = \"x\"\n[[hooks]]\nevent = \"SessionStart\"\ncommand = \"/bin/true\"\n",
        );
        assert!(verify_kimi_config_schema(&dir.path().join("config.toml")).is_ok());
    }

    #[test]
    fn kimi_verify_passes_when_no_hooks_key() {
        let dir = write_tmp("config.toml", "default_model = \"x\"\n");
        assert!(verify_kimi_config_schema(&dir.path().join("config.toml")).is_ok());
    }

    #[test]
    fn kimi_verify_passes_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(verify_kimi_config_schema(&dir.path().join("config.toml")).is_ok());
    }

    #[test]
    fn kimi_verify_rejects_malformed_toml() {
        let dir = write_tmp("config.toml", "default_model = \n[[hooks\n");
        let err = verify_kimi_config_schema(&dir.path().join("config.toml")).unwrap_err();
        assert!(err.contains("parse config.toml"), "err={}", err);
    }

    #[test]
    fn kimi_verify_rejects_hook_missing_event() {
        let dir = write_tmp("config.toml", "[[hooks]]\ncommand = \"/bin/true\"\n");
        let err = verify_kimi_config_schema(&dir.path().join("config.toml")).unwrap_err();
        assert!(err.contains("missing `event`"), "err={}", err);
    }

    #[test]
    fn kimi_verify_rejects_hook_missing_command() {
        let dir = write_tmp("config.toml", "[[hooks]]\nevent = \"Stop\"\n");
        let err = verify_kimi_config_schema(&dir.path().join("config.toml")).unwrap_err();
        assert!(err.contains("missing `command`"), "err={}", err);
    }
}
