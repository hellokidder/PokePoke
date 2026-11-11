//! Terminal.app 终端跳转:按 tty 定位 window/tab + 跨 space 激活。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! 结构镜像 iterm2:`focus_terminal_app`(重试 + 三态)+ `focus_terminal_app_script`
//! (纯函数,单测验证结构)+ `focus_terminal_app_once` + `TerminalFocusOutcome`。

use super::applescript::run_applescript_str;

pub(super) fn focus_terminal_app(tty: &str) -> bool {
    // 同 focus_iterm2:set index 把 window 在所在 space 拉前,activate 让
    // macOS 跨 space 切到 Terminal.app + 那块屏。
    //
    // 2026-07-14:对齐 focus_iterm2 的 on-error 兜底 + 重试。Terminal.app 也会
    // 在 Apple Event 繁忙时抛异常,裸脚本同样会被吞成 false。包 try/on error,
    // 异常时兜底 activate(至少拉 Terminal.app 前台),osascript 失败重试 1 次。
    for attempt in 0..2 {
        match focus_terminal_app_once(tty) {
            Some(TerminalFocusOutcome::Hit) => return true,
            Some(TerminalFocusOutcome::ActivatedFallback) => {
                crate::log_info!(
                    "focus",
                    "focus_terminal_app({}) activated-fallback (set index raised, activate only) attempt={}",
                    tty,
                    attempt
                );
                return true;
            }
            Some(TerminalFocusOutcome::Miss) => return false,
            None => {
                crate::log_info!(
                    "focus",
                    "focus_terminal_app({}) osascript failed attempt={} (retry)",
                    tty,
                    attempt
                );
                continue;
            }
        }
    }
    false
}

/// `focus_terminal_app` 一次尝试的三态结果,语义镜像 `ItermFocusOutcome`。
enum TerminalFocusOutcome {
    Hit,
    ActivatedFallback,
    Miss,
}

/// 纯函数:生成 focus_terminal_app 的 AppleScript(包 try/on error + 三态 return),
/// 结构镜像 `focus_iterm2_script`。抽出为单测验证结构。
pub(super) fn focus_terminal_app_script(tty_dev: &str) -> String {
    format!(
        r#"tell application "Terminal"
    try
        set found to false
        repeat with w in windows
            repeat with t in tabs of w
                if tty of t is "{tty}" then
                    set index of w to 1
                    set selected tab of w to t
                    activate
                    set found to true
                    exit repeat
                end if
            end repeat
            if found then exit repeat
        end repeat
        if found then
            return "ok"
        else
            return "miss"
        end if
    on error errMsg
        activate
        return "activated"
    end try
end tell"#,
        tty = tty_dev
    )
}

fn focus_terminal_app_once(tty_dev: &str) -> Option<TerminalFocusOutcome> {
    let script = focus_terminal_app_script(tty_dev);
    let raw = run_applescript_str(&script)?;
    match raw.trim() {
        "ok" => Some(TerminalFocusOutcome::Hit),
        "activated" => Some(TerminalFocusOutcome::ActivatedFallback),
        _ => Some(TerminalFocusOutcome::Miss),
    }
}
