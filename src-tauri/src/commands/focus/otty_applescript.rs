//! Otty AppleScript 聚焦/抑制(Otty 字典兼容 Terminal.app `tabs`/`selected` 语义)。
//!
//! ⚠️ 所有 osascript 调用必须走带 5s 硬超时的 `crate::osascript::run`
//! (经下方 [`run_otty_bool`]),**禁止**裸 `Command::new("osascript").output()`
//! —— 2026-08-03 事故:Otty
//! AppleEvent 服务挂死时,裸调用会等满 Apple 默认超时(~2 分钟,-1712)才返回,
//! 拖死整条 herdr 回跳链。其余终端模块统一走 `applescript.rs::run_applescript_bool`,
//! 本模块是最后补上的同款防线。

/// 只读查询:front window 当前 selected tab 的 AppleScript id(不带 `t_` 前缀)。
/// 供哨兵差分诊断使用:与 bound tab 真实对比,零误报识别"判定契约损坏"
/// (用户在 bound tab 但判定恒 false)。查询失败/Otty 无窗口返回 None。
pub(crate) fn front_selected_tab_id() -> Option<String> {
    let script = r#"tell application "Otty"
    if (count of windows) is 0 then return ""
    return (id of selected tab of front window)
end tell"#;
    let out = crate::osascript::run(
        script,
        crate::osascript::DEFAULT_TIMEOUT,
        "focus",
        "otty_front_selected_tab",
    )?;
    let id = out.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// 统一入口:带 5s 硬超时跑 Otty AppleScript,结果按 stdout=="true" 判 bool。
/// 成功打 INFO(带调用上下文,便于对账);超时/失败细节由 `osascript::run` 打
/// WARN(`osascript TIMEOUT/FAILED op=...` 含 stderr),这里不重复。
fn run_otty_bool(script: &str, op: &str, ctx: &str) -> bool {
    let out = crate::osascript::run(script, crate::osascript::DEFAULT_TIMEOUT, "focus", op);
    match out.as_deref() {
        Some(s) if s.trim() == "true" => {
            crate::log_info!("focus", "otty {} {} ok=true", op, ctx);
            true
        }
        Some(_) => {
            crate::log_info!("focus", "otty {} {} ok=false", op, ctx);
            false
        }
        None => {
            crate::log_info!(
                "focus",
                "otty {} {} failed (timeout/exit, 细节见 osascript WARN)",
                op,
                ctx
            );
            false
        }
    }
}

/// 前窗口当前 selected tab 是否就是给定 tab(聚焦抑制/召回用)。
/// `tab_id_applescript` 必须是 AppleScript `id of tab` 形式(不带 `t_` 前缀)。
///
/// ⚠️ `window_id` 参数保留仅为调用点兼容,内部**必须忽略**:Otty CLI 的
/// window_id 是 `w_<UUID>`(如 `w_17618FDF-…`),而 AppleScript 的 `id of w`
/// 是标准 window 类的数字 id(实测 `9916`,Otty.sdef 未给 window 扩展任何属性),
/// 两套体系不可互匹配。历史实现(66be4d5)拿 CLI UUID 匹配 `id of w`,分支恒
/// false —— 日志佐证:2026-08-07 起 `otty_selected_tab_is` 9590 次 ok=false、
/// 0 次 ok=true,Otty 聚焦抑制与聚焦召回全部失效(2026-08-10 事故)。
///
/// 语义依据:"用户正在看 bound tab" ⟺ "bound tab 是 front window 的 selected
/// tab"。tab id 全局唯一(AppleScript `id of tab` 与 CLI `t_` 前缀后一致,已
/// 实测);用户看哪个窗口,哪个就是 front window,多窗口下 front-window 语义
/// 天然正确,无需窗口级消歧。
pub(super) fn selected_tab_is(tab_id_applescript: &str, _window_id: Option<&str>) -> bool {
    let tab_id_escaped = tab_id_applescript.replace('"', r#"\""#);
    let script = format!(
        r#"tell application "Otty"
    if (count of windows) is 0 then return false
    return (id of selected tab of front window) is "{id}"
end tell"#,
        id = tab_id_escaped,
    );
    run_otty_bool(
        &script,
        "otty_selected_tab_is",
        &format!("tab={}", tab_id_applescript),
    )
}

/// 前窗口当前 selected tab 的 tty 是否等于给定 tty(只读,聚焦抑制用)。
/// 用于 herdr-in-Otty:herdr UI client 的 tty 若等于 Otty 前窗口 selected tab
/// 的 tty,说明用户正看着承载 herdr 的 tab -> 抑制 popup。
pub(crate) fn otty_selected_tab_has_tty(tty_dev: &str) -> bool {
    let tty = tty_dev.trim();
    let script = format!(
        r#"tell application "Otty"
    if (count of windows) is 0 then return false
    return (tty of selected tab of front window) is "{tty}"
end tell"#,
        tty = tty.replace('"', r#"\""#)
    );
    run_otty_bool(
        &script,
        "otty_selected_tab_has_tty",
        &format!("tty={}", tty),
    )
}

/// 通过 AppleScript 聚焦 Otty 窗口中指定 tab_id 的 tab。
/// Otty 的 AppleScript 字典兼容 Terminal.app:`set selected of tab ... to true` +
/// `activate` 即可精准跳转。
pub(super) fn focus_otty_window_applescript(tab_id: &str) -> bool {
    // CLI tab_id 形如 `t_19fa3119c4c_1`(带类型前缀),但 Otty AppleScript 字典里
    // `id of tab` 返回不带 `t_` 前缀的 `19fa3119c4c_1`。比较前必须剥前缀对齐,
    // 否则 `id of t is "t_..."` 永远不匹配,聚焦恒返回 false(实测 verified)。
    let applescript_id = tab_id.strip_prefix("t_").unwrap_or(tab_id);
    let script = format!(
        r#"tell application "Otty"
    activate
    repeat with w in windows
        repeat with t in tabs of w
            if id of t is "{tab_id}" then
                set selected of t to true
                set index of w to 1
                return true
            end if
        end repeat
    end repeat
    return false
end tell"#,
        tab_id = applescript_id.replace('"', r#"\""#)
    );
    run_otty_bool(
        &script,
        "otty_focus_otty_window",
        &format!("tab_id={}", tab_id),
    )
}

/// herdr 跑在 Otty pane 里时,按 herdr UI client 的 tty 定位承载它的 Otty tab,
/// 选中 + 抬窗 + 激活。用于 herdr 段 A 的 host anchor(Ghostty/iTerm2/cmux 之外
/// 的 Otty 宿主)。
///
/// Otty AppleScript `tty of tab` 返回该 tab active pane 的 tty(`/dev/ttysXXX`),
/// 与 herdr client tty 比对定位。tty 全局唯一,不会误命中别的终端上的 herdr。
/// `activate` 放在命中分支内,避免 tty 未命中时把 Otty 误拉前台(herdr 在
/// Ghostty 但 Otty 也在跑的场景)。
pub(crate) fn focus_otty_herdr_anchor_by_tty(client_tty_dev: &str) -> bool {
    let tty = client_tty_dev.trim();
    let script = format!(
        r#"tell application "Otty"
    repeat with w in windows
        repeat with t in tabs of w
            if tty of t is "{tty}" then
                set selected of t to true
                set index of w to 1
                activate
                return true
            end if
        end repeat
    end repeat
    return false
end tell"#,
        tty = tty.replace('"', r#"\""#)
    );
    run_otty_bool(
        &script,
        "otty_focus_otty_herdr_anchor",
        &format!("tty={}", tty),
    )
}
