//! Ghostty 终端跳转 + capture + herdr anchor tab 兜底。
//!
//! 从 `commands/focus.rs` 抽出(2026-07-27,brief 20260727-split-focus-rs)。
//! 三层跳转:`focus_ghostty`(L1 id verify / L2 cwd first-match / L3 activate-only)
//! + capture 枚举(SessionStart 异步抓 stable terminal id + cwd)+ herdr anchor
//! (`focus_ghostty_herdr_anchor_*`,CC 跑在 herdr pane 时定位 name=="herdr" 的
//! Ghostty tab)。`is_ghostty_herdr_tab_focused_for_socket` 供 popup 聚焦抑制。

use super::applescript::{
    applescript_escape, is_app_running, run_applescript_bool, run_applescript_str,
};
use super::ghostty_capture::cwds_compatible;
use super::{clear_stale_terminal_id, log_path_hint, terminal_id_hint};
use crate::sessions::SessionStore;
use std::sync::{Arc, Mutex};

pub(super) fn focus_ghostty_for_session(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    terminal_id: &str,
    terminal_id_captured_cwd: Option<&str>,
    workspace_path: Option<&str>,
    log_context: &str,
) -> FocusGhosttyResult {
    let r = focus_ghostty(Some(terminal_id), terminal_id_captured_cwd, workspace_path);
    crate::log_debug!(
        "focus",
        "{} focus_ghostty terminal_id_hint={} -> focused={} invalidate_id={}",
        log_context,
        terminal_id_hint(Some(terminal_id)),
        r.focused,
        r.invalidate_id
    );
    if r.invalidate_id {
        clear_stale_terminal_id(store, session_id, terminal_id);
    }
    r
}

/// 段 A herdr anchor tab 兜底:CC 跑在 herdr pane 里时,外层 Ghostty tab 的
/// name 一直是字面量 `"herdr"`(实测 herdr 0.7.3 不会自动把 cc-hex8 写进 title)。
/// 于是 pokepoke 段 A 的 title-key 匹配和 cwd first-match 都没依据 —— 前者拿不到
/// 前 8 位 hex,后者会撞到别的同 workspace tab。
///
/// 这里加一层 herdr 专属 anchor lookup:如果 session 有 `herdr_pane_id` 但段 A
/// 没落到"叫 herdr 的那个 tab",就 osascript 找 `name is "herdr"` 的 terminal
/// 并聚焦。多 herdr 实例(unique 找不到)先不管,skip 让段 B 独立生效。
///
/// **不写回 `terminal_id` 到 session** —— Ghostty tab 生命周期短,重启就换 UUID;
/// anchor 每次点击都重新 osascript 查,便宜(单次枚举)且总是当下正确。
///
/// 返回:是否命中并聚焦成功。false 时 caller 保持段 A 结果不变,段 B 仍会尝试。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FocusGhosttyResult {
    pub focused: bool,
    pub invalidate_id: bool,
}

fn ghostty_l1_verify_basis<'a>(
    terminal_id_captured_cwd: Option<&'a str>,
    workspace_path: Option<&'a str>,
) -> &'a str {
    terminal_id_captured_cwd
        .filter(|cwd| !cwd.is_empty())
        .or(workspace_path)
        .unwrap_or("")
}

/// L1 cwd 校验:复用 capture 模块的 cwd 兼容判定(cwds_compatible 随实现拆到
/// ghostty_capture.rs)。
fn ghostty_l1_cwd_matches(
    current_cwd: &str,
    terminal_id_captured_cwd: Option<&str>,
    workspace_path: Option<&str>,
) -> bool {
    cwds_compatible(
        current_cwd,
        ghostty_l1_verify_basis(terminal_id_captured_cwd, workspace_path),
    )
}

/// Ghostty session 跳转。三层兜底:
///   L1 terminal_id Some  → focus by id,**先验 cwd 跟 capture cwd snapshot 兼容**
///                         才用(2026-05-25 加固:历史脏数据 + capture race 都
///                         可能让 stored id 撞到无关 tab,稳定跳错。verify 不过
///                         直接当 miss,signal invalidate)
///   L2 workspace_path Some → focus by working directory first-match
///   L3 都没有 / 都失败    → activate Ghostty(切前台不选 tab)
///
/// `invalidate_id` 在以下时机为 true:
///   - L1 id 在 Ghostty 里查不到(tab 关了 / Ghostty 重启 id 重发)
///   - L1 id 找到了但 cwd 不再跟 capture cwd snapshot 兼容
/// 这两种 stored id 都是脏的,caller 应清掉。
/// L1 verify 后的动作判定。纯函数,把"osascript 查询结果 → 该做什么"的语义
/// 收在一起便于单测;osascript 调用留在 `focus_ghostty` 只负责执行。
///
/// - `Invalidate`:stored id 在 Ghostty 里已不存在(tab 关了 / Ghostty 重启 id 重发)
///   → caller 清掉脏 id,走 L2。
/// - `TrustId`:tab 存在但 `working directory` 属性空(shell integration 未上报
///   OSC7,cwd 整体不可用)→ 不做 cwd 校验,直接 focus by id。id 是 capture 在
///   agent 活跃时绑定的稳定锡点,绑定关系本身可信(see 2026-06-25 capture 放宽)。
/// - `CwdCheck`:tab 存在且 cwd 有值 → 走原 cwd verify,由 caller 用
///   `ghostty_l1_cwd_matches` 决定 focus 还是当 stale。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum L1VerifyOutcome {
    Invalidate,
    TrustId,
    CwdCheck,
}

/// `exists` = verify 脚本里 id 是否 match 到 terminal;`current_cwd` = 该 tab
/// 当前的 working directory(可能为空)。两者都来自 verify 脚本的 "exists\x1ecwd"。
fn l1_verify_outcome(exists: bool, current_cwd: &str) -> L1VerifyOutcome {
    if !exists {
        L1VerifyOutcome::Invalidate
    } else if current_cwd.trim().is_empty() {
        L1VerifyOutcome::TrustId
    } else {
        L1VerifyOutcome::CwdCheck
    }
}

/// Ghostty `focus` 后焦点异步结算的等待时长(秒)。`focus` 返回不代表 split 已切到,
/// 太短会读回旧焦点误判失败,太长拖慢点击体感。0.08 对照 open-vibe-island 实测值。
pub(super) const GHOSTTY_FOCUS_SETTLE_DELAY: &str = "0.08";
/// `activate window` / `select tab` 之间的小停顿(秒),给 window server 跨 space 切换喘息。
pub(super) const GHOSTTY_WINDOW_ACTIVATION_DELAY: &str = "0.04";
/// focus + 读回验证的最大重试次数。Ghostty 偶发一次没切到,重试基本能落位。
pub(super) const GHOSTTY_FOCUS_ATTEMPTS: u8 = 3;

/// L1 内部用:按已转义的 terminal id 定位并聚焦该 tab。返回焦点是否**确实落到**目标 terminal。
///
/// 旧实现是 `activate; focus t; return true` —— 三个坑:
///   1. 只 activate app,没 `activate window` + `select tab`,多窗口/跨 space 时
///      app 到前台但停在别的 window/tab(症状:跳到 Ghostty 没切对 tab)。
///   2. `focus` 返回即 return true,但 Ghostty 异步更新 focused split,
///      焦点还没落位就报成功(症状:焦点没落位要手点一下)。
///   3. 完全不验证焦点是否真的切过去了。
///
/// 8c036f2 加固改成"先 window→tab→terminal 三级定位,再 repeat N 次 { activate window;
/// select tab; focus; delay; 读回验证 }"。但这版有个**引用错位**坑(2026-07-01 复现):
/// `repeat with aWindow in windows` 拿到的 `aWindow`/`aTab` 是按**当前 z-order 索引**
/// 的延迟解析引用(`item N of every window` / `item M of ... item N of every window`)。
/// `activate window targetWindow` 把目标窗口拉到前台后 windows 顺序立刻翻转,之后
/// `select tab targetTab` 复用旧引用会按**新** z-order 重新解析 → 指向另一个窗口的同
/// 索引 tab。实测:焦点在窗口 B,回跳窗口 A 的 tab,会把 B 的 selected tab 也换掉
/// (B 的 tab 数足够时不报错,而是错位选中 B 的同索引 tab)。
///
/// 修复:分两阶段,绝不跨 z-order 翻转复用引用。
///   Phase 1(z-order 未变):遍历只存 `targetWindow`。`activate window targetWindow`
///     解析该引用时 z-order 还没翻,正确;返回后 front window = 目标 window。
///   Phase 2(z-order 已稳定):在 `tabs of front window` 里**按 id 重新解析**
///     tab/terminal,再 `select tab` + `focus` + 读回验证。front window 始终是
///     目标窗口,引用不再错位。重试循环里每轮都重新解析,不存跨轮引用。
///
/// 读回 id 等于目标才算成功 —— 这是 focus **之后**的事实级验证,跟 L1 调用前的
/// "id/cwd verify"(focus 之前)互补。
/// `pub(super)`:ghostty_herdr.rs(段 C herdr client 宿主)也走这条路。
pub(super) fn focus_ghostty_by_id(escaped_id: &str) -> bool {
    let script = focus_ghostty_by_id_script(escaped_id);
    run_applescript_bool(&script)
}

/// 纯函数:生成 L1 跳转脚本。抽出仅为单测验证结构(单测跑不了真 osascript)——
/// 断言脚本不在 Phase 1 存 tab/terminal 引用、Phase 2 通过 `front window` 重新解析,
/// 防止 8c036f2 那版 stale-ref 回归。
pub(super) fn focus_ghostty_by_id_script(escaped_id: &str) -> String {
    format!(
        r#"tell application "Ghostty"
    try
        set targetWindow to missing value
        repeat with aWindow in windows
            repeat with aTab in tabs of aWindow
                repeat with aTerminal in terminals of aTab
                    if (id of aTerminal as text) is "{id}" then
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
                    if (id of aTerminal as text) is "{id}" then
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
                if (id of focused terminal of selected tab of front window as text) is "{id}" then
                    return true
                end if
            end try
        end repeat
        return false
    on error
        return false
    end try
end tell"#,
        id = escaped_id,
        attempts = GHOSTTY_FOCUS_ATTEMPTS,
        win_delay = GHOSTTY_WINDOW_ACTIVATION_DELAY,
        settle_delay = GHOSTTY_FOCUS_SETTLE_DELAY,
    )
}

/// 纯函数:生成 L2(cwd first-match)跳转脚本。结构镜像 `focus_ghostty_by_id_script`,
/// 同样分两阶段防引用错位(见该函数注释)。抽出仅为单测验证结构。
pub(super) fn focus_ghostty_l2_cwd_script(escaped_path: &str) -> String {
    format!(
        r#"tell application "Ghostty"
    try
        set targetWindow to missing value
        repeat with aWindow in windows
            repeat with aTab in tabs of aWindow
                repeat with aTerminal in terminals of aTab
                    if (working directory of aTerminal as text) is "{path}" then
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
        set theTab to missing value
        set theTerm to missing value
        repeat with aTab in tabs of front window
            repeat with aTerminal in terminals of aTab
                if (working directory of aTerminal as text) is "{path}" then
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
        return true
    on error
        return false
    end try
end tell"#,
        path = escaped_path,
        win_delay = GHOSTTY_WINDOW_ACTIVATION_DELAY,
        settle_delay = GHOSTTY_FOCUS_SETTLE_DELAY,
    )
}

pub(super) fn focus_ghostty(
    terminal_id: Option<&str>,
    terminal_id_captured_cwd: Option<&str>,
    workspace_path: Option<&str>,
) -> FocusGhosttyResult {
    let mut result = FocusGhosttyResult::default();
    if !is_app_running("Ghostty") {
        return result;
    }
    // L1 with verify
    if let Some(id) = terminal_id {
        let escaped = applescript_escape(id);
        // 先 query 现在这个 id 对应的 tab 是否存在 + 当前 cwd,再决定要不要 focus。
        // 返回 "exists\x1ecwd"(exists=1/0):区分"tab 真不存在"(invalidate)与
        // "tab 存在但 working directory 属性空"(Ghostty shell integration 未上报
        // OSC7 时 cwd 整体不可用,见 2026-06-25 capture 放宽)。后者不该当 stale ——
        // id 是 capture 时绑定的稳定锡点,cwd 不可用时信任 id 直接 focus。
        let verify_script = format!(
            r#"tell application "Ghostty"
    try
        set t to first terminal whose id is "{id}"
        return "1" & (ASCII character 30) & (working directory of t as text)
    on error
        return "0" & (ASCII character 30)
    end try
end tell"#,
            id = escaped
        );
        let raw = run_applescript_str(&verify_script).unwrap_or_default();
        let mut parts = raw.splitn(2, '\u{001e}');
        let exists = parts.next().unwrap_or("").trim() == "1";
        let current_cwd = parts.next().unwrap_or("").trim().to_string();
        match l1_verify_outcome(exists, &current_cwd) {
            L1VerifyOutcome::Invalidate => {
                crate::log_info!(
                    "focus",
                    "focus_ghostty L1 terminal_id_hint={} not found in Ghostty (tab closed?), invalidate + fall L2",
                    terminal_id_hint(Some(id))
                );
                result.invalidate_id = true;
            }
            L1VerifyOutcome::TrustId => {
                // tab 存在但 cwd 不可用(shell integration 没上报 OSC7):信任 stored id
                // 直接 focus,不做 cwd 校验。比走 L2(working directory 匹配,同样废)
                // / L3(activate-only 不选 tab)精准 —— id 是 capture 在 agent 活跃时
                // 绑定的,绑定关系本身可信。
                if focus_ghostty_by_id(&escaped) {
                    crate::log_info!(
                        "focus",
                        "focus_ghostty L1 by terminal_id_hint={} cwd_unavailable trust_id captured_cwd={} workspace_path={} ok",
                        terminal_id_hint(Some(id)),
                        log_path_hint(terminal_id_captured_cwd),
                        log_path_hint(workspace_path)
                    );
                    result.focused = true;
                    return result;
                }
                crate::log_info!(
                    "focus",
                    "focus_ghostty L1 by terminal_id_hint={} cwd_unavailable trust_id but focus failed, fall L2",
                    terminal_id_hint(Some(id))
                );
            }
            L1VerifyOutcome::CwdCheck => {
                let verify_cwd = ghostty_l1_verify_basis(terminal_id_captured_cwd, workspace_path);
                if ghostty_l1_cwd_matches(&current_cwd, terminal_id_captured_cwd, workspace_path) {
                    if focus_ghostty_by_id(&escaped) {
                        crate::log_info!(
                            "focus",
                            "focus_ghostty L1 by terminal_id_hint={} verified current_cwd={} captured_cwd={} workspace_path={} ok",
                            terminal_id_hint(Some(id)),
                            log_path_hint(Some(current_cwd.as_str())),
                            log_path_hint(terminal_id_captured_cwd),
                            log_path_hint(workspace_path)
                        );
                        result.focused = true;
                        return result;
                    }
                    crate::log_info!(
                        "focus",
                        "focus_ghostty L1 by terminal_id_hint={} verify ok but focus failed, fall L2",
                        terminal_id_hint(Some(id))
                    );
                } else {
                    crate::log_info!(
                        "focus",
                        "focus_ghostty L1 terminal_id_hint={} STALE: current_cwd={} verify_cwd={} captured_cwd={} session_workspace={} 不兼容,invalidate + fall L2",
                        terminal_id_hint(Some(id)),
                        log_path_hint(Some(current_cwd.as_str())),
                        log_path_hint(Some(verify_cwd)),
                        log_path_hint(terminal_id_captured_cwd),
                        log_path_hint(workspace_path)
                    );
                    result.invalidate_id = true;
                }
            }
        }
    }
    // L2:按 cwd first-match。同样做 window→tab 定位 + select tab + focus,
    // 否则多窗口下 `focus t` 切了 terminal 但 window/tab 没到前台(跳错 tab 的元凶之一)。
    // 注意 L2 本就是兜底,cwd 可能撞多个 tab,这里只保证"聚焦到第一个匹配的那个 tab
    // 确实落位",不解决撞车本身(撞车由阶段② title 主键治本)。
    //
    // 引用错位防护同 L1(见 focus_ghostty_by_id 注释):遍历只存 targetWindow,
    // `activate window` 翻转 z-order 后在 `tabs of front window` 里按 cwd 重新解析
    // tab/terminal,绝不跨翻转复用引用。
    if let Some(path) = workspace_path {
        let escaped = applescript_escape(path);
        let script = focus_ghostty_l2_cwd_script(&escaped);
        if run_applescript_bool(&script) {
            crate::log_info!("focus", "focus_ghostty L2 by cwd={} ok", path);
            result.focused = true;
            return result;
        }
        crate::log_info!("focus", "focus_ghostty L2 by cwd={} miss, fall L3", path);
    }
    // L3
    let ok = run_applescript_bool(
        r#"tell application "Ghostty" to activate
return true"#,
    );
    crate::log_info!("focus", "focus_ghostty L3 activate-only -> {}", ok);
    result.focused = ok;
    result
}

/// L1 verify / l1_verify_outcome 纯函数测试(拆分后保留在主体;
/// capture / 枚举相关测试随实现移到 ghostty_capture.rs)。
#[cfg(test)]
mod ghostty_l1_tests {
    use super::{
        ghostty_l1_cwd_matches, ghostty_l1_verify_basis, l1_verify_outcome, L1VerifyOutcome,
    };

    #[test]
    fn l1_verify_prefers_captured_cwd_over_drifted_workspace() {
        assert_eq!(
            ghostty_l1_verify_basis(Some("/Users/edy"), Some("/Users/edy/Documents/Kidder")),
            "/Users/edy"
        );
        assert!(ghostty_l1_cwd_matches(
            "/Users/edy",
            Some("/Users/edy"),
            Some("/Users/edy/Documents/Kidder")
        ));
    }

    #[test]
    fn l1_verify_falls_back_to_workspace_for_legacy_sessions() {
        assert_eq!(
            ghostty_l1_verify_basis(None, Some("/Users/edy/Documents/Kidder")),
            "/Users/edy/Documents/Kidder"
        );
        assert!(!ghostty_l1_cwd_matches(
            "/Users/edy",
            None,
            Some("/Users/edy/Documents/Kidder")
        ));
    }

    #[test]
    fn l1_verify_outcome_invalidates_when_tab_missing() {
        // id 在 Ghostty 里 match 不到任何 terminal(tab 关了 / Ghostty 重启 id 重发)
        // → invalidate,走 L2。
        assert_eq!(l1_verify_outcome(false, ""), L1VerifyOutcome::Invalidate);
        // cwd 有值但 exists=false 仍按"不存在"处理(exists 优先)。
        assert_eq!(
            l1_verify_outcome(false, "/Users/edy/proj"),
            L1VerifyOutcome::Invalidate
        );
    }

    #[test]
    fn l1_verify_outcome_trusts_id_when_cwd_unavailable() {
        // tab 存在但 working directory 空(shell integration 未上报 OSC7)→ 信任 id。
        // 这是 2026-06-25 修订的核心:cwd 不可用时不再当 stale,直接 focus by id。
        assert_eq!(l1_verify_outcome(true, ""), L1VerifyOutcome::TrustId);
        assert_eq!(l1_verify_outcome(true, "   "), L1VerifyOutcome::TrustId);
    }

    #[test]
    fn l1_verify_outcome_runs_cwd_check_when_cwd_available() {
        // tab 存在且 cwd 有值 → 走原 cwd verify(兼容 shell integration 正常的情况)。
        assert_eq!(
            l1_verify_outcome(true, "/Users/edy/proj"),
            L1VerifyOutcome::CwdCheck
        );
    }
}
