//! Orca CLI 探测的「运行时可用性前置门 + 失败分类」。
//!
//! 2026-10-13 从 `orca.rs` 抽出(同目录 `orca_herdr.rs` 同样拆法):既让 `orca.rs`
//! 回到 §4a 体量阈值内,也让"为什么这么探测"集中在一处。
//!
//! ## 前置门为什么必要
//!
//! 实测(Orca 1.4.215,本机):`orca --version` 正常,但 `orca terminal list --json`
//! 返回
//! `{"ok":false,"error":{"code":"runtime_unavailable","message":"Could not read Orca
//! runtime metadata at .../orca-runtime.json. Start the Orca app first."}}`
//! —— **Orca app 未运行时 CLI 永远查不到终端**。而 CLI 是 Electron
//! (`ELECTRON_RUN_AS_NODE`) 冷启动,实测每次 55–85ms;原来的"失败就换参数重试"
//! 会把每个 herdr 宿主的 notify 白烧两次启动(~165ms),日志还误报成"flag 不可用"。
//!
//! Orca 自己的判据(其 CLI `out/cli/runtime/metadata.js::readMetadata`):
//! 元数据文件存在 + 可解析 + 含 `unix`/`named-pipe` transport + 有 `authToken`。
//! 这里镜像同一判据(读文件 ~µs,**不启动进程**),并把"参数不兼容"与
//! "运行时不可用"分开 —— 只有前者值得去掉 flag 回退重试。

use super::{resolve_orca_cli, OrcaCliEnvelope};
use crate::commands::focus::cli_timeout::{command_output_with_timeout, FOCUS_CLI_TIMEOUT};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Orca CLI 的结构化错误(`{"ok":false,"error":{"code":...,"message":...}}`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrcaCliError {
    pub code: String,
    pub message: String,
}

impl OrcaCliError {
    /// 是否"参数不兼容"类失败(CLI 跑起来了但拒绝了我们的 flag)。
    /// 只有这类才值得去掉 flag 重试;`runtime_unavailable` / `runtime_timeout` /
    /// `incompatible_runtime` 等重试一律无意义。
    fn is_invalid_argument(&self) -> bool {
        self.code == "invalid_argument"
    }
}

/// 一次 Orca CLI 探测的结果。
pub(super) enum OrcaProbe<T> {
    Ok(T),
    /// 参数被 CLI 拒绝(契约漂移,例如旧版没有 `--include-visual-layouts`)。
    InvalidArgument(OrcaCliError),
    /// 运行时不可用 / 连接失败 / 超时 / 输出不可解析 —— 重试无意义。
    Unavailable,
}

/// Orca 的 userData 目录。镜像其 CLI `getDefaultUserDataPath`
/// (dev / 多实例用 `ORCA_USER_DATA_PATH` 指向别的目录,必须一致才判得准)。
fn orca_user_data_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ORCA_USER_DATA_PATH") {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").ok()?;
        return Some(PathBuf::from(home).join("Library/Application Support/orca"));
    }
    if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(PathBuf::from(appdata).join("orca"));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty());
    match base {
        Some(b) => Some(PathBuf::from(b).join("orca")),
        None => Some(PathBuf::from(std::env::var("HOME").ok()?).join(".config/orca")),
    }
}

/// 元数据是否满足 Orca 自己的"运行时可用"条件(见上方模块段注释)。
/// 纯读文件,不启动进程。
fn orca_runtime_available() -> bool {
    let Some(dir) = orca_user_data_dir() else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(dir.join("orca-runtime.json")) else {
        return false;
    };
    let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    metadata_has_unix_transport_and_token(&metadata)
}

/// `findTransport(metadata, 'unix', 'named-pipe') && metadata.authToken`
/// (兼容新版 `transports[]` 与旧版单体 `transport` 字段)。
fn metadata_has_unix_transport_and_token(metadata: &serde_json::Value) -> bool {
    let has_transport = metadata
        .get("transports")
        .and_then(|t| t.as_array())
        .map(|items| items.iter().any(is_unix_like_transport))
        .unwrap_or(false)
        || metadata
            .get("transport")
            .map(is_unix_like_transport)
            .unwrap_or(false);
    let has_token = metadata
        .get("authToken")
        .and_then(|t| t.as_str())
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    has_transport && has_token
}

fn is_unix_like_transport(transport: &serde_json::Value) -> bool {
    matches!(
        transport.get("kind").and_then(|k| k.as_str()),
        Some("unix") | Some("named-pipe")
    )
}

/// 元数据在、但连接类失败(app 崩溃后残留元数据 / 半死状态)时的短冷却。
/// 只挡"肯定重试也没用"的失败;Orca 重新起来后最晚 `ORCA_UNREACHABLE_TTL`
/// 恢复探测(bounded,用户不可感知)。
const ORCA_UNREACHABLE_TTL: Duration = Duration::from_secs(10);
static ORCA_UNREACHABLE_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

fn orca_recently_unreachable() -> bool {
    let guard = ORCA_UNREACHABLE_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.map(|until| Instant::now() < until).unwrap_or(false)
}

fn mark_orca_unreachable() {
    let mut guard = ORCA_UNREACHABLE_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = Some(Instant::now() + ORCA_UNREACHABLE_TTL);
}

fn clear_orca_unreachable() {
    let mut guard = ORCA_UNREACHABLE_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// 从 CLI 输出里取结构化错误。CLI 失败时把 `{"ok":false,"error":{...}}` 打到
/// **stdout**(实测 1.4.215;退出码 1)。
/// 样本(2026-10-13 真机采集,已作为单测夹具):
/// - `runtime_unavailable`:“Could not read Orca runtime metadata at ... Start the Orca app first.”
/// - `invalid_argument`:“Unknown flag --x for command: terminal list” + `data.validFlags`
fn parse_orca_cli_error(stdout: &[u8]) -> Option<OrcaCliError> {
    let value: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    let error = value.get("error")?;
    Some(OrcaCliError {
        code: error
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string(),
        message: error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// 探测一次 Orca CLI(带运行时可用性前置门 + 失败分类)。
pub(super) fn run_orca_json_checked<T: for<'de> Deserialize<'de>>(args: &[&str]) -> OrcaProbe<T> {
    // 前置门:运行时元数据不满足 Orca 自己的判据 → 直接跳过,连进程都不起。
    if !orca_runtime_available() || orca_recently_unreachable() {
        crate::log_debug!("focus", "orca skip (runtime unavailable) args={:?}", args);
        return OrcaProbe::Unavailable;
    }

    let cli = resolve_orca_cli();
    let mut cmd = Command::new(&cli);
    cmd.args(args).arg("--json");
    let Some(output) = command_output_with_timeout(cmd, FOCUS_CLI_TIMEOUT, "orca/json") else {
        mark_orca_unreachable();
        return OrcaProbe::Unavailable;
    };

    if !output.status.success() {
        let failure = parse_orca_cli_error(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = failure
            .as_ref()
            .map(|f| f.message.as_str())
            .unwrap_or_else(|| stderr.trim());
        crate::log_info!(
            "focus",
            "orca cli={} args={:?} exit={} code={} message={}",
            cli.display(),
            args,
            output.status,
            failure
                .as_ref()
                .map(|f| f.code.as_str())
                .unwrap_or("<unparsed>"),
            detail,
        );
        return match failure {
            Some(error) if error.is_invalid_argument() => OrcaProbe::InvalidArgument(error),
            // 运行时不可用 / 超时 / 其它:重试无意义,进冷却避免每个 notify 都重启 Electron。
            _ => {
                mark_orca_unreachable();
                OrcaProbe::Unavailable
            }
        };
    }

    match serde_json::from_slice::<OrcaCliEnvelope<T>>(&output.stdout) {
        Ok(envelope) => {
            if !envelope.ok {
                // 契约上 ok=false 应伴随非零退出码(实测两种失败都是 1),但保持防御:
                // 同样按错误码分类,避免"退出码 0 + ok=false"时静默吞掉契约漂移。
                let failure = parse_orca_cli_error(&output.stdout);
                crate::log_info!(
                    "focus",
                    "orca cli={} args={:?} ok=false code={}",
                    cli.display(),
                    args,
                    failure
                        .as_ref()
                        .map(|f| f.code.as_str())
                        .unwrap_or("<unparsed>")
                );
                return match failure {
                    Some(error) if error.is_invalid_argument() => OrcaProbe::InvalidArgument(error),
                    _ => {
                        mark_orca_unreachable();
                        OrcaProbe::Unavailable
                    }
                };
            }
            match envelope.result {
                Some(result) => {
                    clear_orca_unreachable();
                    OrcaProbe::Ok(result)
                }
                None => {
                    crate::log_info!(
                        "focus",
                        "orca cli={} args={:?} ok=true but no result",
                        cli.display(),
                        args
                    );
                    OrcaProbe::Unavailable
                }
            }
        }
        Err(err) => {
            crate::log_info!(
                "focus",
                "orca cli={} args={:?} json parse failed: {}",
                cli.display(),
                args,
                err
            );
            mark_orca_unreachable();
            OrcaProbe::Unavailable
        }
    }
}

pub(super) fn run_orca_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> Option<T> {
    match run_orca_json_checked(args) {
        OrcaProbe::Ok(result) => Some(result),
        OrcaProbe::InvalidArgument(_) | OrcaProbe::Unavailable => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::OrcaTerminalListResult;
    use super::*;

    // 下列两条错误样本均为真机 Orca 1.4.215 的**原始输出**(遵循"解析外部数据
    // 必须真实样本验证"的约定)。

    /// `orca terminal list --json`,Orca app 未运行时(退出码 1)。
    const REAL_RUNTIME_UNAVAILABLE: &str = r#"{
  "id": "local",
  "ok": false,
  "error": {
    "code": "runtime_unavailable",
    "message": "Could not read Orca runtime metadata at /Users/someone/Library/Application Support/orca/orca-runtime.json. Start the Orca app first."
  }
}"#;

    /// `orca terminal list --bogus --json`(退出码 1,带 validFlags)。
    const REAL_INVALID_ARGUMENT: &str = r#"{
  "id": "local",
  "ok": false,
  "error": {
    "code": "invalid_argument",
    "message": "Unknown flag --totally-bogus-flag for command: terminal list",
    "data": {
      "validFlags": ["environment", "help", "include-visual-layouts", "json", "limit", "pairing-code", "worktree"]
    }
  }
}"#;

    #[test]
    fn parses_real_cli_error_payloads() {
        let runtime =
            parse_orca_cli_error(REAL_RUNTIME_UNAVAILABLE.as_bytes()).expect("应解析出错误");
        assert_eq!(runtime.code, "runtime_unavailable");
        assert!(
            !runtime.is_invalid_argument(),
            "运行时不可用不属于「参数不兼容」,不得触发换参数重试"
        );

        let invalid = parse_orca_cli_error(REAL_INVALID_ARGUMENT.as_bytes()).expect("应解析出错误");
        assert_eq!(invalid.code, "invalid_argument");
        assert!(invalid.is_invalid_argument(), "参数不兼容才需要回退重试");

        // 成功输出 / 非 JSON 输出不应被当成错误。
        assert!(parse_orca_cli_error(br#"{"ok":true,"result":{}}"#).is_none());
        assert!(parse_orca_cli_error(b"not json").is_none());
    }

    #[test]
    fn metadata_availability_matches_orca_cli_contract() {
        let unix = serde_json::json!({
            "transports": [
                {"kind": "websocket", "url": "ws://127.0.0.1:1"},
                {"kind": "unix", "path": "/tmp/orca.sock"}
            ],
            "authToken": "t"
        });
        assert!(metadata_has_unix_transport_and_token(&unix));

        let legacy = serde_json::json!({
            "transport": {"kind": "named-pipe", "path": "x"},
            "authToken": "t"
        });
        assert!(
            metadata_has_unix_transport_and_token(&legacy),
            "旧版单体 transport 字段仍须识别"
        );

        let ws_only = serde_json::json!({
            "transports": [{"kind": "websocket", "url": "ws://127.0.0.1:1"}],
            "authToken": "t"
        });
        assert!(
            !metadata_has_unix_transport_and_token(&ws_only),
            "仅 websocket 不算可用(镜像 Orca 的 findTransport('unix','named-pipe'))"
        );

        let no_token = serde_json::json!({
            "transports": [{"kind": "unix", "path": "/tmp/orca.sock"}]
        });
        assert!(
            !metadata_has_unix_transport_and_token(&no_token),
            "缺 authToken"
        );

        let empty_token = serde_json::json!({
            "transports": [{"kind": "unix", "path": "/tmp/orca.sock"}],
            "authToken": ""
        });
        assert!(!metadata_has_unix_transport_and_token(&empty_token));
    }

    /// 前置门核心:元数据不满足判据时**连进程都不该启动**。
    /// 用会 touch 哨兵的假 CLI 证明它没被调用。
    #[test]
    #[serial_test::serial]
    fn orca_probe_skips_cli_when_runtime_metadata_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sentinel = dir.path().join("spawned");
        let fake_cli = write_fake_cli(dir.path(), &sentinel, r#"{"ok":true,"result":{}}"#);
        let empty_user_data = dir.path().join("empty-user-data");
        std::fs::create_dir_all(&empty_user_data).expect("mkdir");

        std::env::set_var("ORCA_USER_DATA_PATH", &empty_user_data);
        std::env::set_var("ORCA_CLI_COMMAND", &fake_cli);
        clear_orca_unreachable();
        let probed = run_orca_json::<OrcaTerminalListResult>(&["terminal", "list"]);
        std::env::remove_var("ORCA_USER_DATA_PATH");
        std::env::remove_var("ORCA_CLI_COMMAND");
        clear_orca_unreachable();

        assert!(probed.is_none(), "运行时不可用时应直接返回 None");
        assert!(
            !sentinel.exists(),
            "元数据缺失时绝不能启动 Orca CLI(Electron 冷启动 55–85ms)"
        );
    }

    /// 反向对照:元数据满足判据时,CLI 会被调用且结果被解析。
    #[test]
    #[serial_test::serial]
    fn orca_probe_calls_cli_when_runtime_metadata_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sentinel = dir.path().join("spawned");
        let fake_cli = write_fake_cli(
            dir.path(),
            &sentinel,
            r#"{"ok":true,"result":{"terminals":[],"visualLayouts":[]}}"#,
        );
        let user_data = dir.path().join("user-data");
        std::fs::create_dir_all(&user_data).expect("mkdir");
        std::fs::write(
            user_data.join("orca-runtime.json"),
            r#"{"transports":[{"kind":"unix","path":"/tmp/orca.sock"}],"authToken":"t"}"#,
        )
        .expect("写元数据");

        std::env::set_var("ORCA_USER_DATA_PATH", &user_data);
        std::env::set_var("ORCA_CLI_COMMAND", &fake_cli);
        clear_orca_unreachable();
        let probed = run_orca_json::<OrcaTerminalListResult>(&["terminal", "list"]);
        std::env::remove_var("ORCA_USER_DATA_PATH");
        std::env::remove_var("ORCA_CLI_COMMAND");
        clear_orca_unreachable();

        assert!(sentinel.exists(), "元数据可用时必须真的调用 CLI");
        assert!(probed.is_some(), "正常 envelope 应被解析出来");
    }

    /// 造一个"记录自己被调用"的假 CLI。
    fn write_fake_cli(dir: &std::path::Path, sentinel: &std::path::Path, json: &str) -> PathBuf {
        let path = dir.join("fake-orca");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ntouch '{}'\necho '{}'\n",
                sentinel.display(),
                json
            ),
        )
        .expect("写假 CLI");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        path
    }
}
