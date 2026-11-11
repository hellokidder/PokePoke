//! Warp 回跳(brief 20260810-warp-terminal-support)。
//!
//! Warp 无 AppleScript 可枚举,回跳优先走官方 local_control 协议(见
//! `crate::warp`),协议不可用时降级系统级激活:
//!
//! 1. **local_control**:`app.focus` 把 Warp 带到前台(Warp 的 `WarpControlCli`
//!    feature flag 开启时可用)
//! 2. **`open -a Warp` 降级**:local_control 不可用(flag 未 rollout / 版本过旧)
//!    时用 Launch Services 激活。不依赖 Apple Events(TCC Automation 权限),
//!    对已运行实例就是 activate 语义
//! 3. **AppleScript 兜底**:`open -a` 后自查 frontmost 未命中再试
//!    `tell application "Warp" to activate`
//!
//! 每层激活后自查 frontmost(150ms 后),命中即止;全部失败只记日志不抛错。
//! 调用方保证只有 `terminal_app == "warpterminal"` 的 session 走这里,
//! 因此不会误吸 iTerm2/Ghostty ladder。
//!
//! > 注:title-key 精确回跳(OSC 2 改写 tab title + `tab.activate` 全等匹配)
//! > 曾随 20260810 会话下掉(local_control 未 rollout 时收益为负)。2026-08-18
//! > 起仅在 **herdr-remote 段 C** 以 marker 形式加回(见 focus/herdr.rs
//! > `focus_warp_tab_with_marker`,brief 20260818-herdr-remote-warp-tab-focus):
//! > herdr 自己的 `client.window_title.set` 写 marker + Warp `tab.activate`
//! > 按标题全等切 tab,解决双 herdr tab(本机 + remote)时 activate-only 停错
//! > tab 的问题;local_control 不可用(Settings > Scripting 未开)时不改标题、
//! > 直接退化 activate-only。普通 warpterminal 会话仍走 activate-only,
//! > 不引入标题改写。

use std::process::Command;
use std::thread;
use std::time::Duration;

use super::applescript::{is_app_running, run_applescript_bool};
use crate::osascript::frontmost_app_name;

/// Warp 主进程的可执行名(稳定版为 `stable`;preview/dev 渠道会不同),用于
/// frontmost 自查时宽容匹配 `Warp` / `stable` / 其它渠道名。
/// `pub(crate)`:popup/focus.rs 的 Warp 聚焦抑制共用同一名单。
pub(crate) fn frontmost_is_warp(name: Option<&str>) -> bool {
    matches!(
        name.unwrap_or(""),
        "Warp" | "stable" | "preview" | "dev" | "oss" | "warp"
    )
}

/// 激活后等 150ms 自查 frontmost,记录结果并返回是否命中 Warp。
fn verify_frontmost(tag: &str) -> bool {
    thread::sleep(Duration::from_millis(150));
    let front = frontmost_app_name();
    let hit = frontmost_is_warp(front.as_deref());
    crate::log_info!(
        "focus",
        "focus_warp_session {} frontmost={:?} hit={}",
        tag,
        front,
        hit
    );
    hit
}

/// 回跳主入口:local_control → `open -a Warp` → AppleScript,逐层降级 + 自查。
pub(super) fn focus_warp_session() -> bool {
    // 层 1:local_control(未来 flag rollout 后走这里)
    match crate::warp::focus_app() {
        Ok(()) => {
            let ok = verify_frontmost("local-control");
            if ok {
                return true;
            }
        }
        Err(e) => {
            crate::log_info!(
                "focus",
                "focus_warp_session local-control unavailable ({}); fallback open",
                e
            );
        }
    }
    if !is_app_running("Warp") {
        crate::log_info!("focus", "focus_warp_session Warp not running");
        return false;
    }
    // 层 2:Launch Services(无 TCC Automation 权限依赖;已运行实例 = activate)
    let open_ok = Command::new("/usr/bin/open")
        .args(["-a", "Warp"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    crate::log_info!("focus", "focus_warp_session open -a Warp -> {}", open_ok);
    if open_ok && verify_frontmost("open") {
        return true;
    }
    // 层 3:AppleScript 兜底
    let ok = run_applescript_bool("tell application \"Warp\" to activate\nreturn true");
    crate::log_info!("focus", "focus_warp_session osascript activate -> {}", ok);
    ok && verify_frontmost("osascript")
}

#[cfg(test)]
mod tests {
    use super::frontmost_is_warp;

    /// 名单与 popup/focus.rs 的 Warp 聚焦抑制共用,漏配任何一个渠道名都会
    /// 导致"回跳自查命中但抑制不命中"(或反之)的不一致。
    #[test]
    fn frontmost_is_warp_matches_all_channels() {
        for name in ["Warp", "stable", "preview", "dev", "oss", "warp"] {
            assert!(frontmost_is_warp(Some(name)), "{name} 应命中 Warp 家族");
        }
    }

    #[test]
    fn frontmost_is_warp_rejects_non_warp() {
        for name in [
            "iTerm2", "Ghostty", "Terminal", "WezTerm", "Cursor", "Finder", "",
        ] {
            assert!(!frontmost_is_warp(Some(name)), "{name} 不应命中 Warp 家族");
        }
        assert!(!frontmost_is_warp(None));
    }
}
