//! `/notify` title 解析:codex 桌面端 thread title + 兜底 hook 上报 title。
//! 从 `http_server.rs` 抽出(brief 20260727-split-http-server-rs 第一阶段),行为零变化。

use super::NotifyRequest;
use super::CODEX_TITLE_CACHE;

/// codex 桌面端 thread title 查询(带进程级缓存,见 `state.rs` 的
/// `CODEX_TITLE_CACHE`)。codex hook payload 只带 thread UUID,真实 title 在
/// codex 自己的 session_index.jsonl 的 `thread_name` 字段;对话刚开可能还没
/// 生成 title → None,caller 兜底 hook 上报的 workspace 目录名。
pub(super) fn codex_thread_title(external_session_id: &str) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home)
        .join(".codex")
        .join("session_index.jsonl");
    CODEX_TITLE_CACHE.thread_name(&path, external_session_id)
}

/// codex 桌面端会话(ChatGPT.app,无 tty)的 title 解析:优先用 codex 对话真实 title
/// (thread_name),查不到兜底 hook 上报的 title(workspace 目录名)。
///
/// codex CLI(有 tty)不走此路径,保持 workspace basename —— 判据与 surface.rs 的
/// CodexDesktop 分支一致(source=codex && tty 为空)。
pub(super) fn resolve_title_for_notify(req: &NotifyRequest) -> String {
    let is_codex_desktop = req.source.as_deref() == Some("codex")
        && req
            .terminal_tty
            .as_deref()
            .map(|t| t.is_empty())
            .unwrap_or(true);
    if is_codex_desktop {
        if let Some(ext) = req.external_session_id.as_deref() {
            if let Some(name) = codex_thread_title(ext) {
                return name;
            }
        }
    }
    req.title.clone()
}
