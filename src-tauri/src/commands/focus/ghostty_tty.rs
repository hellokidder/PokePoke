//! Ghostty 1.4+ 原生 `tty` 精准回跳(L0)。
//!
//! brief `20261029-ghostty-tty-focus.md`。Ghostty 1.4.0 起 sdef 的 terminal class
//! 暴露 `tty`(text,`/dev/ttysNNN`)与 `pid`,与 iTerm2 的 tty 匹配同构 —— focus
//! 不再依赖「capture 猜 focused tab / OSC title 消歧 / cwd first-match」。
//!
//! 独立文件而非续写 `ghostty.rs`:AGENTS.md §4a 的结构约定(新增适配走独立文件,
//! 且 `ghostty.rs` 已在 file-size ratchet 白名单里只许缩不许涨)。

use super::applescript::{applescript_escape, run_applescript_bool};
use super::ghostty::{
    GHOSTTY_FOCUS_ATTEMPTS, GHOSTTY_FOCUS_SETTLE_DELAY, GHOSTTY_WINDOW_ACTIVATION_DELAY,
};

/// 按原生 `tty` 属性直连定位并聚焦该 surface。
///
/// - tty 是 surface 级的,与 hook 上报的 `terminal_tty` 同源(`ps -o tty=` 实测一致),
///   split pane 也各自独立 → 同 cwd 多 tab / 多 split 不再撞车。
/// - tty 属于内层终端(herdr / cmux / tmux pane)时匹配不到任何 surface,
///   **自然 miss**,由 caller 继续走既有 herdr/cmux 短路或 L1/L2/L3。
/// - Ghostty < 1.4 的 sdef 没有该属性,整脚本编译失败返回 false,同样落到兜底路径
///   (不做版本门:用户在 1.4+,少一层探测状态机;代价是 1.3 上多点一次失败的
///   osascript,不影响回跳结果)。
pub(super) fn focus_ghostty_by_tty(tty: &str) -> bool {
    let escaped = applescript_escape(tty);
    run_applescript_bool(&focus_ghostty_by_tty_script(&escaped))
}

/// 纯函数:生成 tty 版 L0 跳转脚本。结构与 `ghostty::focus_ghostty_by_id_script`
/// 一致(Phase 1 只解析 `targetWindow`;Phase 2 在 `front window` 内按 tty 重新解析
/// tab/terminal,绝不跨 z-order 翻转复用引用),差异只有匹配键从 `id` 换成 `tty`。
pub(super) fn focus_ghostty_by_tty_script(escaped_tty: &str) -> String {
    format!(
        r#"tell application "Ghostty"
    try
        set targetWindow to missing value
        repeat with aWindow in windows
            repeat with aTab in tabs of aWindow
                repeat with aTerminal in terminals of aTab
                    if (tty of aTerminal as text) is "{tty}" then
                        set targetWindow to aWindow
                        exit repeat
                    end if
                end repeat
                if targetWindow is not missing value then exit repeat
            end repeat
            if targetWindow is not missing value then exit repeat
        end repeat

        if targetWindow is missing value then return false

        activate
        activate window targetWindow
        delay {win_delay}
        repeat {attempts} times
            set theTab to missing value
            set theTerm to missing value
            repeat with aTab in tabs of front window
                repeat with aTerminal in terminals of aTab
                    if (tty of aTerminal as text) is "{tty}" then
                        set theTab to aTab
                        set theTerm to aTerminal
                        exit repeat
                    end if
                end repeat
                if theTerm is not missing value then exit repeat
            end repeat
            if theTerm is missing value then return false
            select tab theTab
            delay {win_delay}
            focus theTerm
            delay {settle_delay}
            try
                if (tty of focused terminal of selected tab of front window as text) is "{tty}" then
                    return true
                end if
            end try
        end repeat
        return false
    on error
        return false
    end try
end tell"#,
        tty = escaped_tty,
        attempts = GHOSTTY_FOCUS_ATTEMPTS,
        win_delay = GHOSTTY_WINDOW_ACTIVATION_DELAY,
        settle_delay = GHOSTTY_FOCUS_SETTLE_DELAY,
    )
}

#[cfg(test)]
mod tests {
    use super::focus_ghostty_by_tty_script;

    #[test]
    fn l0_tty_script_does_not_store_tab_or_terminal_in_phase1() {
        // 与 `focus_ghostty_by_id_script` 同款 stale-ref 防护:Phase 1 只存 window。
        let s = focus_ghostty_by_tty_script("/dev/ttys016");
        let phase1 = s.split("activate\n").next().unwrap_or(&s);
        assert!(
            !phase1.contains("set targetTab to"),
            "L0 Phase 1 must not store targetTab (stale-ref bug): {phase1}"
        );
        assert!(
            !phase1.contains("set targetTerminal to"),
            "L0 Phase 1 must not store targetTerminal (stale-ref bug): {phase1}"
        );
        assert!(phase1.contains("set targetWindow to aWindow"));
    }

    #[test]
    fn l0_tty_script_matches_by_tty_and_verifies_readback() {
        let s = focus_ghostty_by_tty_script("/dev/ttys016");
        assert!(
            s.contains("if (tty of aTerminal as text) is \"/dev/ttys016\" then"),
            "L0 must match by tty, got: {s}"
        );
        assert!(
            !s.contains("id of aTerminal"),
            "L0 must not fall back to id matching: {s}"
        );
        assert!(
            !s.contains("working directory of aTerminal"),
            "L0 must not fall back to cwd matching: {s}"
        );
        // Phase 2 在 front window 内按 tty 重新解析,并读回验证。
        let after_activate = s.split("activate window targetWindow").nth(1).unwrap_or("");
        assert!(
            after_activate.contains("tabs of front window"),
            "L0 Phase 2 must re-resolve via `tabs of front window`: {after_activate}"
        );
        assert!(after_activate.contains("select tab theTab"));
        assert!(after_activate.contains("focus theTerm"));
        assert!(after_activate.contains(
            "if (tty of focused terminal of selected tab of front window as text) is \"/dev/ttys016\" then"
        ));
    }
}
