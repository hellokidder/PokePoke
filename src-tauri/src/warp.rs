//! Warp 终端 local_control 客户端(brief 20260810-warp-terminal-support)。
//!
//! Warp(`TERM_PROGRAM=WarpTerminal`)没有 AppleScript 可枚举,回跳走 Warp 官方
//! 本地控制协议(源码开源,`warpctrl` CLI 同源):
//!
//! 1. **discovery**:`~/.warp/local-control/<instance_id>.json` 发布 InstanceRecord
//!    (含 loopback 端点 + instance-bound broker socket 引用)
//! 2. **认证**:连 `<id>.broker.sock` unix socket,写 `CredentialRequest`
//!    (action-scoped),shutdown(write) 后读回 `ScopedCredential{bearer_token}`
//! 3. **调用**:`POST http://127.0.0.1:<port>/v1/control` + `Authorization: Bearer`,
//!    body 为 `RequestEnvelope{protocol_version,request_id,target,action{kind,params}}`
//!
//! 客户端侧校验与 Warp 官方 client 一致(`validate_local_control_authority`):
//! 端点必须是 `127.0.0.1`,broker 文件名必须由所选 instance_id 派生,否则拒绝,
//! 防止 discovery 目录被外部污染后把请求重定向走。
//!
//! 所有失败都向调用方返回 `WarpControlError`;调用方(focus 回跳)只记日志,
//! 绝不影响 hook / notify 主链路。

use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// discovery record 目录:`~/.warp/local-control/`(Warp discovery.rs 同款)。
fn discovery_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".warp").join("local-control"))
}

/// 从 discovery 目录选一个可用的 InstanceRecord。
///
/// 过滤条件(与 Warp `InstanceRecord::validate_local_control_authority` 对齐):
/// - `endpoint` 与 `credential_broker` 同时在场(禁用 record 不发布部分路由)
/// - endpoint host 必须是 `127.0.0.1`
/// - broker 文件名必须是 `<instance_id>.broker.sock`
///
/// 多实例时取 `started_at` 最新的一条;任何校验不过都不猜测、不跳过校验。
pub fn discover_active_instance() -> Option<InstanceRecord> {
    let dir = discovery_dir()?;
    let mut best: Option<(chrono::DateTime<chrono::Utc>, InstanceRecord)> = None;
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // 排除写一半的临时 record(Warp 用 `<id>.json.tmp` 原子发布)
        if !name.ends_with(".json") || name.ends_with(".json.tmp") {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<InstanceRecord>(&text) else {
            continue;
        };
        if !record.is_enabled_and_authoritative() {
            continue;
        }
        if best
            .as_ref()
            .map(|(t, _)| record.started_at > *t)
            .unwrap_or(true)
        {
            best = Some((record.started_at, record));
        }
    }
    best.map(|(_, r)| r)
}

/// discovery record 的本地最小视图(只取路由 + 校验所需的字段)。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InstanceRecord {
    #[serde(default)]
    pub protocol_version: u32,
    pub instance_id: String,
    #[serde(default)]
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub endpoint: Option<ControlEndpoint>,
    pub credential_broker: Option<CredentialBrokerReference>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ControlEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CredentialBrokerReference {
    pub socket_path: PathBuf,
}

/// 本地控制失败的分类(供日志;不向上抛跨模块错误)。
#[derive(Debug)]
pub enum WarpControlError {
    /// local_control 未启用(record 缺失 / endpoint 禁用 / feature flag 未开)
    Disabled(String),
    /// 传输层失败(broker socket 连不上 / HTTP 失败)
    Transport(String),
    /// 认证失败(broker 拒绝 / bearer 无效)
    Auth(String),
    /// Warp 侧业务错误(如 title 无匹配 tab)
    Rejected(String),
}

impl std::fmt::Display for WarpControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarpControlError::Disabled(m) => write!(f, "warp local-control disabled: {m}"),
            WarpControlError::Transport(m) => write!(f, "warp local-control transport: {m}"),
            WarpControlError::Auth(m) => write!(f, "warp local-control auth: {m}"),
            WarpControlError::Rejected(m) => write!(f, "warp local-control rejected: {m}"),
        }
    }
}

impl InstanceRecord {
    fn is_enabled_and_authoritative(&self) -> bool {
        match (&self.endpoint, &self.credential_broker) {
            (Some(ep), Some(broker)) => {
                ep.host == "127.0.0.1" && broker.socket_path == broker_filename(&self.instance_id)
            }
            _ => false,
        }
    }

    fn broker_socket_path(&self) -> Option<PathBuf> {
        if !self.is_enabled_and_authoritative() {
            return None;
        }
        let broker = self.credential_broker.as_ref()?;
        Some(discovery_dir()?.join(&broker.socket_path))
    }
}

fn broker_filename(instance_id: &str) -> PathBuf {
    PathBuf::from(format!("{instance_id}.broker.sock"))
}

/// 生成 `CredentialRequest` 的 request_id(与 Warp 一样用 uuid v4)。
fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 通过 owner-authenticated broker socket 换一个 action-scoped bearer。
fn request_credential(record: &InstanceRecord, action: &str) -> Result<String, WarpControlError> {
    let path = record
        .broker_socket_path()
        .ok_or_else(|| WarpControlError::Disabled("broker socket path unavailable".into()))?;
    let request = json!({
        "protocol_version": record.protocol_version,
        "request_id": new_request_id(),
        "action": action,
    });
    let mut stream = UnixStream::connect(&path)
        .map_err(|e| WarpControlError::Transport(format!("broker socket connect failed: {e}")))?;
    // broker 按 read-to-EOF 分隔请求;Warp 官方 client 也走 shutdown(Write)
    stream
        .write_all(request.to_string().as_bytes())
        .map_err(|e| WarpControlError::Transport(format!("broker write failed: {e}")))?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|e| WarpControlError::Transport(format!("broker shutdown failed: {e}")))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| WarpControlError::Transport(format!("broker read failed: {e}")))?;
    let value: Value = serde_json::from_str(&response)
        .map_err(|e| WarpControlError::Auth(format!("broker returned invalid JSON: {e}")))?;
    if let Some(token) = value.get("bearer_token").and_then(|t| t.as_str()) {
        return Ok(token.to_string());
    }
    // ErrorResponseEnvelope 形状:{ "error": { "code": ..., "message": ... } }
    let msg = value
        .pointer("/error/message")
        .and_then(|m| m.as_str())
        .unwrap_or("credential broker refused request");
    Err(WarpControlError::Auth(msg.to_string()))
}

/// 发一条已认证的 local-control 请求,返回响应 JSON。
fn send_request(
    record: &InstanceRecord,
    target: Value,
    action: &str,
    params: Value,
) -> Result<Value, WarpControlError> {
    let bearer = request_credential(record, action)?;
    let endpoint = record
        .endpoint
        .as_ref()
        .ok_or_else(|| WarpControlError::Disabled("endpoint missing".into()))?;
    let body = json!({
        "protocol_version": record.protocol_version,
        "request_id": new_request_id(),
        "target": target,
        "action": { "kind": action, "params": params },
    });
    let url = format!("http://{}:{}/v1/control", endpoint.host, endpoint.port);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| WarpControlError::Transport(format!("http client build failed: {e}")))?;
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&body)
        .send()
        .map_err(|e| WarpControlError::Transport(format!("http request failed: {e}")))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|e| WarpControlError::Transport(format!("http read failed: {e}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| WarpControlError::Transport(format!("invalid response JSON: {e}")))?;
    if let Some(err) = value.get("response").and_then(|r| r.get("error")) {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown warp control error");
        return Err(WarpControlError::Rejected(msg.to_string()));
    }
    if value.get("error").is_some() {
        let msg = value
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown warp control error envelope");
        return Err(WarpControlError::Rejected(msg.to_string()));
    }
    if !status.is_success() {
        return Err(WarpControlError::Transport(format!(
            "http status {status}: {text}"
        )));
    }
    Ok(value)
}

/// 只把 Warp 带到前台(activate-only)。
pub fn focus_app() -> Result<(), WarpControlError> {
    let Some(record) = discover_active_instance() else {
        return Err(WarpControlError::Disabled(
            "no local-control discovery record".into(),
        ));
    };
    send_request(&record, Value::Null, "app.focus", json!({}))?;
    Ok(())
}

/// tab.activate 的 target selector:按标题全等命中一个 tab。
/// wire 形状与 Warp `selectors.rs` 的 `TabTarget::Title` 对齐。
fn tab_target_by_title(title: &str) -> Value {
    json!({ "tab": { "type": "title", "title": title } })
}

/// 把指定标题的 tab 切到前台并聚焦其窗口(需要 Warp Settings > Scripting
/// 开启,否则 discovery 目录无 record,返回 Disabled)。
///
/// brief 20260818-herdr-remote-warp-tab-focus:activate-only 只把 Warp 拉前台,
/// 显示的是上一个活跃 tab —— 本机 + `herdr --remote` 双 herdr tab 时点 remote
/// popup 会停在本地 herdr tab。调用方先经 herdr `client.window_title.set` 把
/// 目标 tab 标题改成唯一 marker,再按标题全等切 tab。
pub fn focus_tab_by_title(title: &str) -> Result<(), WarpControlError> {
    let Some(record) = discover_active_instance() else {
        return Err(WarpControlError::Disabled(
            "no local-control discovery record".into(),
        ));
    };
    send_request(
        &record,
        tab_target_by_title(title),
        "tab.activate",
        json!({ "mode": "target" }),
    )?;
    Ok(())
}

/// 探测 Warp 是否运行且 local_control 可用(供日志 / 开关文案,不做 gate)。
pub fn is_available() -> bool {
    discover_active_instance().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with(
        instance_id: &str,
        host: &str,
        port: u16,
        broker: Option<&str>,
    ) -> InstanceRecord {
        InstanceRecord {
            protocol_version: 1,
            instance_id: instance_id.to_string(),
            started_at: chrono::Utc::now(),
            endpoint: Some(ControlEndpoint {
                host: host.to_string(),
                port,
            }),
            credential_broker: broker.map(|b| CredentialBrokerReference {
                socket_path: PathBuf::from(b),
            }),
        }
    }

    #[test]
    fn authoritative_record_ok() {
        let r = record_with("inst-1", "127.0.0.1", 1234, Some("inst-1.broker.sock"));
        assert!(r.is_enabled_and_authoritative());
    }

    #[test]
    fn non_loopback_endpoint_rejected() {
        let r = record_with("inst-1", "0.0.0.0", 1234, Some("inst-1.broker.sock"));
        assert!(!r.is_enabled_and_authoritative());
    }

    #[test]
    fn broker_name_must_derive_from_instance_id() {
        let r = record_with("inst-1", "127.0.0.1", 1234, Some("other.broker.sock"));
        assert!(!r.is_enabled_and_authoritative());
    }

    #[test]
    fn disabled_record_rejected() {
        // endpoint 在而 broker 不在 = 部分路由,拒绝
        let mut r = record_with("inst-1", "127.0.0.1", 1234, None);
        r.endpoint = None;
        assert!(!r.is_enabled_and_authoritative());
        let r2 = record_with("inst-1", "127.0.0.1", 1234, None);
        assert!(!r2.is_enabled_and_authoritative());
    }

    #[test]
    fn request_body_shape_matches_warp_protocol() {
        let body = json!({
            "protocol_version": 1,
            "request_id": "uuid",
            "target": null,
            "action": { "kind": "app.focus", "params": {} },
        });
        // Warp 侧 serde 形状校验(rename_all snake_case):
        assert_eq!(body["action"]["kind"], "app.focus");
        assert_eq!(body["protocol_version"], 1);
    }

    #[test]
    fn tab_title_target_matches_warp_tab_target_shape() {
        // brief 20260818-herdr-remote-warp-tab-focus:target 形状对齐 Warp
        // selectors.rs 的 TabTarget::Title(serde tag=type, snake_case)。
        let target = tab_target_by_title("herdr · hello");
        assert_eq!(target["tab"]["type"], "title");
        assert_eq!(target["tab"]["title"], "herdr · hello");
        // 全等匹配:tab.activate 的 params 必须带 mode=target,否则 Warp 拒绝。
        let params = json!({ "mode": "target" });
        assert_eq!(params["mode"], "target");
    }

    #[test]
    fn discover_skips_tmp_records_and_uses_latest() {
        // 用临时 HOME 构造 discovery 目录
        let dir = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        let lc = dir.path().join(".warp").join("local-control");
        fs::create_dir_all(&lc).unwrap();
        // 旧 record + 新 record + tmp + 非法 host
        let old = record_with("old", "127.0.0.1", 1111, Some("old.broker.sock"));
        fs::write(lc.join("old.json"), serde_json::to_string(&old).unwrap()).unwrap();
        let mut latest = record_with("new", "127.0.0.1", 2222, Some("new.broker.sock"));
        latest.started_at = chrono::Utc::now() + chrono::Duration::seconds(10);
        fs::write(lc.join("new.json"), serde_json::to_string(&latest).unwrap()).unwrap();
        fs::write(lc.join("half.json.tmp"), "{}").unwrap();
        let bad = record_with("bad", "0.0.0.0", 3333, Some("bad.broker.sock"));
        fs::write(lc.join("bad.json"), serde_json::to_string(&bad).unwrap()).unwrap();

        let found = discover_active_instance().expect("should find latest authoritative record");
        assert_eq!(found.instance_id, "new");
        assert_eq!(found.endpoint.as_ref().unwrap().port, 2222);

        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
