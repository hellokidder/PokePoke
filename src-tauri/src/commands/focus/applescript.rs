//! AppleScript 辅助:osascript 调用包装 + 字符串转义 + app 运行检测。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! 各终端跳转模块(iterm2/terminal_app/ghostty/herdr)共享这些底层 osascript
//! 调用:`is_app_running`(查 app 是否在跑,避免误 launch)+ `run_applescript_bool/str`
//! (带超时的 osascript 执行,防主线程挂死)+ `applescript_escape`(字面量转义)。

/// Check if a macOS application is currently running (without launching it).
pub(super) fn is_app_running(app_name: &str) -> bool {
    crate::osascript::run_bool(
        &format!("application \"{}\" is running", app_name),
        "focus",
        "is_app_running",
    )
}

pub(super) fn run_applescript_bool(script: &str) -> bool {
    // 带超时:目标终端 App 卡住时不让 osascript 无限阻塞(见 osascript 模块注释,
    // 2026-07-01 主线程挂死事故)。op 名给个泛标签,精确定位靠各 caller 自己的 INFO。
    crate::osascript::run_bool(script, "focus", "applescript_bool")
}

/// 跑 osascript 取 stdout 字符串(trimmed),失败/超时返回 None。
pub(super) fn run_applescript_str(script: &str) -> Option<String> {
    let s = crate::osascript::run(
        script,
        crate::osascript::DEFAULT_TIMEOUT,
        "focus",
        "applescript_str",
    )?;
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 转义 AppleScript 字符串字面量(双引号 + 反斜杠)。
/// 用户 cwd / id 含特殊字符时防止 AppleScript 解析失败。
pub(super) fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
