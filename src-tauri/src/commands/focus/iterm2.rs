//! iTerm2 终端跳转:按 tty 精确定位 tab + session + 跨 space 激活。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! `focus_iterm2`(重试 + 三态)+ `focus_iterm2_script`(纯函数,单测验证结构)
//! + `focus_iterm2_once`(跑脚本解析三态)+ `ItermFocusOutcome`(三态结果)。

use super::applescript::run_applescript_str;

/// `focus_iterm2` 一次尝试的 AppleScript 结果(三态,便于日志区分"精确命中"
/// vs "异常兜底激活" vs "没找到")。`run_bool` 只能给 bool,这里要三态所以用 `run`。
enum ItermFocusOutcome {
    /// `tty of s` 匹配上,`select s` + `select tab` + `activate` 全成功。
    Hit,
    /// 匹配上但 `select`/`select tab` 抛了运行时异常(iTerm2 正忙 / Apple Event
    /// 与 herdr 等同跑 App 撞车)-> on error 兜底 `activate`,至少把 iTerm2 拉前台。
    ActivatedFallback,
    /// 该 tty 在 iTerm2 里没 session(iTerm2 没开这个 tab / tty 是别的 App 的)。
    Miss,
}

/// 纯函数:生成 focus_iterm2 的 AppleScript(包 try/on error + 三态 return)。
/// 抽出仅为单测验证结构(单测跑不了真 osascript)--断言脚本含 try/on error
/// 兜底 + activate + 三态返回值,防止后人改回裸脚本吞异常。
pub(super) fn focus_iterm2_script(tty_dev: &str) -> String {
    format!(
        r#"tell application "iTerm2"
    try
        repeat with w in windows
            repeat with t in tabs of w
                repeat with s in sessions of t
                    if tty of s is "{tty}" then
                        select s
                        tell w to select t
                        tell w to select
                        activate
                        return "ok"
                    end if
                end repeat
            end repeat
        end repeat
        return "miss"
    on error errMsg
        -- select / select tab 偶发抛异常(iTerm2 正忙、与 herdr 等同跑 App 的
        -- Apple Event 撞车)。裸脚本会让 osascript 非零退出被吞成 false,用户
        -- 观感是"点了没反应"。兜底:至少 activate 把 iTerm2 拉到前台,herdr
        -- 窗口就在那,段 B 还能把 pane 切对。
        activate
        return "activated"
    end try
end tell"#,
        tty = tty_dev
    )
}

/// 跑一次 focus_iterm2 脚本,解析三态结果。脚本包了 try/on error:`select s`
/// 异常时落到 `activate` + return "activated"。返回 None 表示 osascript 自身失败
/// (spawn / 超时 / 非零退出且非 on-error 路径)。
fn focus_iterm2_once(tty_dev: &str) -> Option<ItermFocusOutcome> {
    let script = focus_iterm2_script(tty_dev);
    let raw = run_applescript_str(&script)?;
    match raw.trim() {
        "ok" => Some(ItermFocusOutcome::Hit),
        "activated" => Some(ItermFocusOutcome::ActivatedFallback),
        _ => Some(ItermFocusOutcome::Miss),
    }
}

pub(super) fn focus_iterm2(tty: &str) -> bool {
    // 多屏 multi-space 跳转关键三件:
    //   1. select s / select t - 选中 tab + session
    //   2. tell w to select - 让该 window 在自己所在 space 成为 frontmost
    //   3. activate - 激活 iTerm app,触发 macOS 跨 space 切到 window 所在屏
    // 之前只有 1 + 外部 `open -a iTerm`,在 popup 在主屏 / 终端在副屏的场景下,
    // open 拉的是当前 space 的 frontmost iTerm window,不是 popup 关联的那个。
    //
    // 2026-07-14(herdr-in-iTerm2 回跳偶发失败):`select s` 在 iTerm2 与 herdr
    // 等 AppleScript-heavy App 同跑时,Apple Event 撞车会抛运行时异常 -> 旧脚本
    // 裸奔无 on error -> osascript 非零退出被吞成 false,且 stderr 被丢弃,日志
    // 只能看到 false(146ms 快速返回 = 中途异常退出的特征)。三处加固:
    //   a. 脚本包 try/on error,异常时兜底 activate(至少拉 iTerm2 前台)
    //   b. osascript 失败时重试 1 次,覆盖瞬时竞态(iTerm2 下一拍就不忙了)
    //   c. 三态结果落日志,Hit / ActivatedFallback 都算成功(true),Miss 才 false
    for attempt in 0..2 {
        match focus_iterm2_once(tty) {
            Some(ItermFocusOutcome::Hit) => return true,
            Some(ItermFocusOutcome::ActivatedFallback) => {
                crate::log_info!(
                    "focus",
                    "focus_iterm2({}) activated-fallback (select raised, activate only) attempt={}",
                    tty,
                    attempt
                );
                return true;
            }
            Some(ItermFocusOutcome::Miss) => {
                // 没找到该 tty 的 session:重试也救不回(不是竞态,是真没有)。
                return false;
            }
            None => {
                // osascript 自身失败(spawn / 超时 / 非零退出非 on-error 路径):
                // 可能是 iTerm2 瞬时卡住,重试一次。
                crate::log_info!(
                    "focus",
                    "focus_iterm2({}) osascript failed attempt={} (retry)",
                    tty,
                    attempt
                );
                continue;
            }
        }
    }
    false
}
