//! DeepSeek Harness 精准回跳:激活 App + 通过 host 半插件的本地 socket 请求
//! 渲染进程切到指定对话。
//!
//! DSH 没有 per-session deep link(`dsh://open` 只把主窗口拉前台),所以这里
//! 两步走:
//!   1. `osascript … activate`(bundle id `com.deepseek.dsh`)把窗口拉前台;
//!   2. 连 `~/.pokepoke/dsh-<pid>.sock` 发 `{"action":"open","sessionId":…}`,
//!      等插件渲染半执行 `uiWorkspace.openSession` 后回 ack。
//!
//! 失败降级:没有 socket / 连不上 / 超时 → 只激活窗口(`AppActivatedOnly`),
//! 绝不再退回 Finder(见 `FocusTarget::Finder` 的 doc)。

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

use crate::log_info;

/// 回跳结果分型:日志与 `open_session_source` 的 reason 都按它区分
/// "真跳到对话" 和 "只激活了窗口",避免把 activate-only 误报成精确命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DshFocusOutcome {
    Precise,
    AppActivatedOnly,
    Failed,
}

/// DSH session id 形如 `session-<uuid>`;严格校验字符集,防异常值拼进 JSON/路径。
pub(crate) fn is_valid_dsh_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// host 半插件缓存的渲染进程状态(由 client 半每 500ms + focus/blur 事件上报)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DshFocusState {
    pub showing: Option<String>,
    pub focused: bool,
    pub visible: bool,
}

/// "用户此刻是否正看着这条 DSH 对话":showing 命中 + 页面 focused + visible。
/// 与终端 focus gate 同语义 —— DSH 是 GUI/Web 宿主,没有 tty 可查,只能问插件。
pub(crate) fn state_matches_session(state: &DshFocusState, session_id: &str) -> bool {
    state.focused && state.visible && state.showing.as_deref() == Some(session_id)
}

pub(crate) fn dsh_session_is_focused(socket: &str, session_id: &str) -> bool {
    query_state(socket)
        .map(|state| state_matches_session(&state, session_id))
        .unwrap_or(false)
}

/// 向 host 半插件现查状态。超时必须短:popup gate 在 staging worker 上跑,
/// focus watcher 每 500ms 一个 tick,不能被卡死的 DSH 拖住。
pub(crate) fn query_state(socket: &str) -> Option<DshFocusState> {
    let mut stream = UnixStream::connect(socket).ok()?;
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    stream.write_all(b"{\"action\":\"state\"}\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value["ok"].as_bool() != Some(true) {
        return None;
    }
    Some(DshFocusState {
        showing: value["showing"].as_str().map(str::to_string),
        focused: value["focused"].as_bool().unwrap_or(false),
        visible: value["visible"].as_bool().unwrap_or(false),
    })
}

pub(crate) fn focus_dsh_session(session_id: &str, nav_socket: Option<&str>) -> DshFocusOutcome {
    if !is_valid_dsh_session_id(session_id) {
        log_info!(
            "focus",
            "focus_dsh_session: reject invalid session_id len={}",
            session_id.len()
        );
        return DshFocusOutcome::Failed;
    }

    let activated = activate_dsh();
    let Some(socket) = nav_socket.filter(|s| !s.is_empty()) else {
        log_info!(
            "focus",
            "focus_dsh_session: no nav socket, activate_only activated={}",
            activated
        );
        return if activated {
            DshFocusOutcome::AppActivatedOnly
        } else {
            DshFocusOutcome::Failed
        };
    };

    // activate 是异步的:给窗口一点时间真正变成 frontmost,渲染半的
    // `document.hasFocus()` 才会放行 `openSession`(与 Vibe Island 的
    // viewer-not-focused 守卫一致)。
    std::thread::sleep(Duration::from_millis(150));

    match request_open(session_id, socket) {
        Some(true) => {
            log_info!(
                "focus",
                "focus_dsh_session: precise session_id={}",
                session_id
            );
            DshFocusOutcome::Precise
        }
        other => {
            log_info!(
                "focus",
                "focus_dsh_session: socket request failed result={:?} activated={} session_id={}",
                other,
                activated,
                session_id
            );
            if activated {
                DshFocusOutcome::AppActivatedOnly
            } else {
                DshFocusOutcome::Failed
            }
        }
    }
}

/// 激活 DSH 主窗口。优先 bundle id(免本地化名漂移),失败再按应用名。
fn activate_dsh() -> bool {
    let by_id = Command::new("osascript")
        .arg("-e")
        .arg("tell application id \"com.deepseek.dsh\" to activate")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if by_id {
        return true;
    }
    Command::new("open")
        .arg("-a")
        .arg("DeepSeek Harness")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 发一条 open 请求并等 ack。`None` = 连不上 / 写失败;`Some(false)` = 插件拒绝。
fn request_open(session_id: &str, socket: &str) -> Option<bool> {
    let mut stream = UnixStream::connect(socket).ok()?;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    // 插件侧 OPEN_TIMEOUT 是 8s(含渲染半最多 1.5s 等焦点 + 4s 等 retain),
    // 这里留余量,避免 PokePoke 先放弃而插件仍在跑。
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

    let request = serde_json::json!({ "action": "open", "sessionId": session_id });
    stream.write_all(format!("{}\n", request).as_bytes()).ok()?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    if reader.read_line(&mut line).ok()? == 0 {
        return Some(false);
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    Some(value["ok"].as_bool().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dsh_session_ids() {
        assert!(is_valid_dsh_session_id(
            "session-bed025d2-2c09-4387-96f5-bcba51d2dd92"
        ));
        assert!(is_valid_dsh_session_id("session-7e4d6e4a"));
    }

    #[test]
    fn request_open_roundtrips_over_unix_socket() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("dsh-focus-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let listener = UnixListener::bind(&dir).expect("bind fake dsh socket");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream.try_clone().expect("clone"))
                .read_line(&mut line)
                .expect("read request");
            let request: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(request["action"], "open");
            assert_eq!(request["sessionId"], "session-test-1");
            stream.write_all(b"{\"ok\":true}\n").expect("write ack");
        });

        let result = request_open("session-test-1", dir.to_str().expect("path"));
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&dir);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn focus_state_requires_same_visible_focused_conversation() {
        let state = DshFocusState {
            showing: Some("session-a".into()),
            focused: true,
            visible: true,
        };
        assert!(state_matches_session(&state, "session-a"));
        assert!(!state_matches_session(&state, "session-b"));
        assert!(!state_matches_session(
            &DshFocusState {
                focused: false,
                ..state.clone()
            },
            "session-a"
        ));
        assert!(!state_matches_session(
            &DshFocusState {
                visible: false,
                ..state.clone()
            },
            "session-a"
        ));
    }

    #[test]
    fn query_state_roundtrips_over_unix_socket() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let path = std::env::temp_dir().join(format!("dsh-state-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind fake dsh socket");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream.try_clone().expect("clone"))
                .read_line(&mut line)
                .expect("read request");
            let request: serde_json::Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(request["action"], "state");
            stream
                .write_all(
                    b"{\"ok\":true,\"showing\":\"session-a\",\"focused\":true,\"visible\":true}\n",
                )
                .expect("write state");
        });

        let state = query_state(path.to_str().expect("path")).expect("state");
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&path);
        assert_eq!(state.showing.as_deref(), Some("session-a"));
        assert!(state.focused && state.visible);
    }

    #[test]
    fn rejects_injection_and_empty() {
        assert!(!is_valid_dsh_session_id(""));
        assert!(!is_valid_dsh_session_id("session-\"}"));
        assert!(!is_valid_dsh_session_id("session-a b"));
        assert!(!is_valid_dsh_session_id(&"a".repeat(129)));
    }
}
