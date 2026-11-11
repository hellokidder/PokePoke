//! Cursor GUI + ChatGPT 桌面端会话跳转。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! `focus_cursor_workspace`(cursor CLI 复用窗口)+ `focus_codex_desktop`
//! (codex:// deep link + activate)+ `is_valid_codex_thread_id`(UUID 校验)。

use std::process::Command;

use super::terminal_id_hint;

/// Focus a Cursor editor window for the given workspace path.
/// Uses `cursor` CLI which reuses the existing window instead of
/// opening the GUI composer dialog (which `open -a Cursor` tends to do).
pub(super) fn focus_cursor_workspace(path: &str) {
    let _ = Command::new("cursor").arg(path).output();
}

/// ChatGPT 桌面端会话跳回:两步 -- `open codex://threads/<thread_id>` 切对话 +
/// `osascript ... activate` 拉前台。
///
/// ChatGPT.app(= `com.openai.codex`,Electron)注册了 `codex://` scheme,内部
/// 路由 `codex://threads/<id>` 就是 "Open in app" 动作(openThread),切到那条
/// 具体对话。但实测 `open` 对**已运行**的 app 只传 URL、不激活前台(前台不变),
/// 所以必须补一步 activate 把 app 拉到前台,否则用户点了 app 不切前台、看不到。
///
/// thread UUID 来自 codex hook payload 的 session_id(见 pokehook::external_session_id),
/// 与 `~/.codex/sessions/` rollout 的 session_meta.id 同值 -- 验证过 `session_index.jsonl`
/// 的 id 一致。严格校验为 UUID 字符集(`[0-9a-fA-F-]`),防异常值拼出意料外的 URL。
///
/// app 未在跑:`open` 会启动 app;能否恢复到指定 thread 取决于 app 的 deep link
/// 处理,不保证。失败不打开 Finder(没意义,见 FocusTarget::Finder 说明),只记日志。
pub(super) fn focus_codex_desktop(thread_id: &str) -> bool {
    if !is_valid_codex_thread_id(thread_id) {
        crate::log_info!(
            "focus",
            "focus_codex_desktop: reject invalid thread_id hint={}",
            terminal_id_hint(Some(thread_id))
        );
        return false;
    }
    let url = format!("codex://threads/{}", thread_id);
    crate::log_info!(
        "focus",
        "focus_codex_desktop thread_id={} -> open {} + activate",
        terminal_id_hint(Some(thread_id)),
        url
    );
    // 1. deep link 让 app 内部切到该对话(openThread)。
    let opened = match Command::new("open").arg(&url).status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            crate::log_info!("focus", "focus_codex_desktop: open exit={}", status);
            false
        }
        Err(err) => {
            crate::log_info!("focus", "focus_codex_desktop: open failed err={}", err);
            false
        }
    };
    // 2. open 对已运行的 ChatGPT.app 只传 URL 不激活前台(实测前台不变),
    //    显式 activate 把 app 拉到前台。bundle id 比应用名稳(免本地化名漂移)。
    let _ = Command::new("osascript")
        .arg("-e")
        .arg("tell application id \"com.openai.codex\" to activate")
        .output();
    opened
}

/// codex thread UUID 校验:只允许 `[0-9a-fA-F-]`(标准 UUID 字符集),非空且
/// 长度上限 128 拒绝超长输入。codex 的 session_id 是 UUID(如
/// `019f5fbb-1872-7172-a623-e51fa7f31d06`),这组字符覆盖且排除一切会破坏
/// `codex://threads/<id>` URL 语义的字符。
fn is_valid_codex_thread_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}
