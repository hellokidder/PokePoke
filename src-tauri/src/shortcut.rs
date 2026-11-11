//! popup 全局快捷键路由(0.0.3+)。
//!
//! 设计要点:
//! - `⌘1` ~ `⌘9` 对应屏上第 1 ~ 第 9 个 **session** popup(updater popup 不参与编号)。
//! - `⌘Enter` 是 `⌘1` 的别名 —— vim/tmux 工作流里习惯回车确认。
//! - 前缀修饰键可在设置里改(默认 `CmdOrCtrl` = macOS 上的 ⌘),数字主键固定 1~9;
//!   同一前缀 + `Z` 打开静谧模式设置弹窗。
//! - **popup_list 空时全部反注册**,把 `⌘Enter` 等高频组合键还给其他 app。
//! - 每次 popup 增/删触发 `popup-shortcuts-updated` event(在 popup.rs 里 emit),
//!   本模块监听后重新 apply(unregister_all → register 新集合)。
//! - 跨屏全局编号:`popup_at_visual_index` 不区分屏,按 popup_list 插入序数。
//!
//! 不做的事:
//! - 不做 50ms debounce —— popup 增删本身就慢于 ms 量级,直接 register/unregister 安全
//!   - **TODO(将来撞再加)**:`docs/briefs/20260519-popup-shortcut.md` Risks 段写过这条风险 ——
//!     极端场景下多 agent 同时启动 / 同时 emit popup 会触发短时间内 unregister_all →
//!     register → unregister_all 多轮,这期间 `⌘Enter` 在 iTerm2 之类宿主里短暂被释放/抢占,
//!     用户那 0.5s 正好按到的话会感受到行为闪动。当前实测无问题,若用户反馈再补 debounce。
//! - 不做"用户主动关闭快捷键"开关 —— YAGNI,等真有反馈再加
//! - 不做 Windows/Linux 特异处理 —— Tauri global-shortcut 跨平台,首要场景是 macOS

use crate::popup::{self, PopupList};
use crate::quiet_mode;
use crate::settings::SettingsStore;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

/// 解析 settings 里存的完整 shortcut 字符串(如 `CmdOrCtrl+Semicolon` /
/// `Control+Shift+KeyK`)。末尾 token 视为 key `Code`,其余是 modifiers。
/// 校验规则:
/// - 必须含至少一个 `SUPER` 或 `CONTROL`(不允许纯 Shift/Alt 全局热键,防止误伤输入)
/// - 末尾必须是合法 `Code`
/// 失败返回 `None`,调用方回落到 `default_quiet_mode_shortcut()`。
pub fn parse_full_shortcut(raw: &str) -> Option<Shortcut> {
    let tokens: Vec<&str> = raw
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.len() < 2 {
        return None;
    }
    let (key_tok, mod_toks) = tokens.split_last()?;
    let mut mods = Modifiers::empty();
    for part in mod_toks {
        match *part {
            "CmdOrCtrl" | "Command" | "Super" | "Meta" => mods |= Modifiers::SUPER,
            "Control" | "Ctrl" => mods |= Modifiers::CONTROL,
            "Alt" | "Option" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            _ => return None,
        }
    }
    if !mods.intersects(Modifiers::SUPER | Modifiers::CONTROL) {
        return None;
    }
    let code = Code::from_str(key_tok).ok()?;
    Some(Shortcut::new(Some(mods), code))
}

/// 从 settings 读静谧模式热键;解析失败(旧数据 / 非法)时回落默认。
pub fn expected_quiet_mode_shortcut(app: &AppHandle) -> Shortcut {
    let raw = app
        .try_state::<Arc<Mutex<SettingsStore>>>()
        .map(|store| {
            let s = store.lock().unwrap_or_else(|p| p.into_inner());
            s.settings.quiet_mode_shortcut.clone()
        })
        .unwrap_or_else(crate::settings::default_quiet_mode_shortcut);
    parse_full_shortcut(&raw)
        .or_else(|| parse_full_shortcut(&crate::settings::default_quiet_mode_shortcut()))
        .expect("default quiet mode shortcut must parse")
}

/// 返回 true 表示已消费该快捷键。
pub fn handle_quiet_mode_press(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) -> bool {
    if event.state() != ShortcutState::Pressed {
        return false;
    }
    let expected = expected_quiet_mode_shortcut(app);
    if shortcut.mods != expected.mods || shortcut.key != expected.key {
        return false;
    }
    quiet_mode::toggle_quiet_mode_picker(app);
    true
}

/// 常驻全局快捷键(与 popup 列表无关)。`apply` 里 `unregister_all` 后会重注册。
pub fn register_persistent_shortcuts(app: &AppHandle) {
    let gs = app.global_shortcut();
    let sc = expected_quiet_mode_shortcut(app);
    if let Err(e) = gs.register(sc) {
        crate::log_warn!(
            "shortcut",
            "register quiet mode {:?} failed (likely in use): {}",
            sc,
            e,
        );
    }
}

#[cfg(test)]
fn should_block_popup_shortcut_for_plan(_is_pro: bool) -> bool {
    false
}

/// 把 settings 里的 `popup_shortcut_modifiers` 字符串解析成 Tauri `Modifiers`。
/// 未知 token 忽略;全空时回落到 `SUPER`(⌘),保持老行为。
pub fn parse_popup_shortcut_modifiers(raw: &str) -> Modifiers {
    let mut mods = Modifiers::empty();
    for part in raw.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        match part {
            "CmdOrCtrl" | "Command" | "Super" => mods |= Modifiers::SUPER,
            "Control" | "Ctrl" => mods |= Modifiers::CONTROL,
            "Alt" | "Option" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            _ => {}
        }
    }
    // 兜底:必须含 SUPER 或 CONTROL,否则回落 SUPER。
    // 防止用户误存"Shift"这类会被普通输入触发的组合。
    if !mods.intersects(Modifiers::SUPER | Modifiers::CONTROL) {
        Modifiers::SUPER
    } else {
        mods
    }
}

/// popup 角标 / 设置面板展示用:把 `CmdOrCtrl+Shift` 之类转成 `⌘⇧`。
pub fn format_popup_shortcut_mod_prefix(raw: &str) -> String {
    let mut out = String::new();
    for part in raw.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        let glyph = match part {
            "CmdOrCtrl" | "Command" | "Super" => "\u{2318}",
            "Control" | "Ctrl" => "\u{2303}",
            "Alt" | "Option" => "\u{2325}",
            "Shift" => "\u{21e7}",
            _ => continue,
        };
        out.push_str(glyph);
    }
    if out.is_empty() {
        "\u{2318}".to_string()
    } else {
        out
    }
}

fn expected_modifiers(app: &AppHandle) -> Modifiers {
    app.try_state::<Arc<Mutex<SettingsStore>>>()
        .map(|store| {
            let settings = store.lock().unwrap_or_else(|p| p.into_inner());
            parse_popup_shortcut_modifiers(&settings.settings.popup_shortcut_modifiers)
        })
        .unwrap_or(Modifiers::SUPER)
}

/// 全局快捷键事件入口。lib.rs 在 plugin `with_handler` 里只做 popup_list 是否就绪
/// 的判断,真正业务逻辑在这里:
///   1. 只在按下时响应(避免 keyup 重复)
///   2. shortcut → visual_index
///   3. visual_index → session_id(若无对应 popup 则 log + 返回)
///   4. session_id → 当前 label(0.0.3 unique-label,label 带 token,得反查)
///   5. emit `popup-shortcut-trigger` 给那个 popup webview,前端 handleClick 完成跳转
pub fn handle_press(
    app: &AppHandle,
    shortcut: &Shortcut,
    event: ShortcutEvent,
    popup_list: &PopupList,
) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let expected_mods = expected_modifiers(app);
    let Some(n) = shortcut_to_visual_index(shortcut, expected_mods) else {
        return;
    };
    let Some(session_id) = popup::session_popup_at_visual_index(popup_list, n) else {
        crate::log_info!(
            "shortcut",
            "ignored {:?}+{} (no session popup at index {})",
            expected_mods,
            n + 1,
            n,
        );
        return;
    };
    crate::log_info!(
        "shortcut",
        "trigger session_id={} via index {} ({:?}+{})",
        session_id,
        n,
        expected_mods,
        n + 1,
    );
    let Some(label) = popup::find_session_popup_label(popup_list, &session_id) else {
        // 极端时序:visual_index 找到 session_id 后,该 session 的 popup 被关掉了
        crate::log_info!(
            "shortcut",
            "trigger lost (popup gone between visual_index lookup and emit) session_id={}",
            session_id,
        );
        return;
    };
    let _ = app.emit_to(&label, "popup-shortcut-trigger", ());
}

/// 把 `Shortcut` 反查到 visual index(0-based)。
/// `⌘Enter` 和 `⌘1` 都映射到 index 0(Cmd+Enter 是 Cmd+1 的别名,见模块顶注释)。
/// pub:lib.rs 的 with_handler 闭包要用。
pub fn shortcut_to_visual_index(shortcut: &Shortcut, expected_mods: Modifiers) -> Option<usize> {
    if shortcut.mods != expected_mods {
        return None;
    }
    match shortcut.key {
        Code::Enter => Some(0),
        Code::Digit1 => Some(0),
        Code::Digit2 => Some(1),
        Code::Digit3 => Some(2),
        Code::Digit4 => Some(3),
        Code::Digit5 => Some(4),
        Code::Digit6 => Some(5),
        Code::Digit7 => Some(6),
        Code::Digit8 => Some(7),
        Code::Digit9 => Some(8),
        _ => None,
    }
}

/// 给 index(0-based)算对应的 Shortcut。index >= 9 返回 None(只支持 ⌘1~⌘9)。
fn index_to_shortcut(n: usize, mods: Modifiers) -> Option<Shortcut> {
    let code = match n {
        0 => Code::Digit1,
        1 => Code::Digit2,
        2 => Code::Digit3,
        3 => Code::Digit4,
        4 => Code::Digit5,
        5 => Code::Digit6,
        6 => Code::Digit7,
        7 => Code::Digit8,
        8 => Code::Digit9,
        _ => return None,
    };
    Some(Shortcut::new(Some(mods), code))
}

/// `Shortcut::new` 不是 const fn,所以用普通函数代替 const。重复调用代价 ~0。
fn enter_shortcut(mods: Modifiers) -> Shortcut {
    Shortcut::new(Some(mods), Code::Enter)
}

/// 按当前 popup_list 决定 register 哪些 shortcut。
/// - count == 0 → unregister_all,把组合键还给其他 app
/// - count > 0 → register `前缀+Enter` + `前缀+1` ~ `前缀+min(count, 9)`
///
/// 重复调用幂等:`unregister_all` 后重 register 新集合,不在意上次状态。
pub fn apply(app: &AppHandle, popup_list: &PopupList) {
    let count = popup::session_popup_count(popup_list).min(9);
    let mods = expected_modifiers(app);
    let gs = app.global_shortcut();

    // unregister_all 是 Tauri 推荐的"重置"动作,失败也只 warn,不阻塞后续 register
    if let Err(e) = gs.unregister_all() {
        crate::log_warn!("shortcut", "unregister_all error (ignored): {}", e);
    }

    if count == 0 {
        crate::log_info!("shortcut", "popup_list empty, all shortcuts released");
        register_persistent_shortcuts(app);
        return;
    }

    let mut to_register: Vec<Shortcut> = (0..count)
        .filter_map(|n| index_to_shortcut(n, mods))
        .collect();
    to_register.push(enter_shortcut(mods));

    for sc in &to_register {
        if let Err(e) = gs.register(*sc) {
            crate::log_warn!(
                "shortcut",
                "register {:?} failed (likely in use by another app): {}",
                sc,
                e,
            );
        }
    }
    crate::log_info!(
        "shortcut",
        "registered {} shortcuts for {} session popup(s) with mods {:?}",
        to_register.len(),
        count,
        mods,
    );
    register_persistent_shortcuts(app);
}

/// 在 setup hook 里调一次:监听 `popup-shortcuts-updated` / `settings-updated`,
/// 每次触发重 apply。popup.rs 在 show/close 后会 emit 前者。
pub fn install_listener(app: &AppHandle, popup_list: PopupList) {
    let app_handle = app.clone();
    let pl = popup_list.clone();
    let reapply = move || apply(&app_handle, &pl);

    app.listen("popup-shortcuts-updated", move |_event| {
        reapply();
    });

    let app_for_settings = app.clone();
    let pl_for_settings = popup_list.clone();
    app.listen("settings-updated", move |_event| {
        apply(&app_for_settings, &pl_for_settings);
    });

    // 启动时先 apply 一次,handle popup_list 已有内容的情况(虽然启动 reset 一般为空)
    apply(app, &popup_list);
}

#[cfg(test)]
mod tests {
    use super::{
        enter_shortcut, format_popup_shortcut_mod_prefix, index_to_shortcut, parse_full_shortcut,
        parse_popup_shortcut_modifiers, shortcut_to_visual_index,
        should_block_popup_shortcut_for_plan,
    };
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

    #[test]
    fn enter_and_digit1_both_map_to_index_zero() {
        let mods = Modifiers::SUPER;
        assert_eq!(
            shortcut_to_visual_index(&enter_shortcut(mods), mods),
            Some(0)
        );
        assert_eq!(
            shortcut_to_visual_index(&Shortcut::new(Some(mods), Code::Digit1), mods),
            Some(0),
            "⌘Enter 与 ⌘1 同语义,都指向 visual index 0",
        );
    }

    #[test]
    fn digit_keys_2_through_9_map_to_index_1_through_8() {
        let mods = Modifiers::SUPER;
        for n in 0..9_usize {
            let sc = index_to_shortcut(n, mods).unwrap();
            let recovered = shortcut_to_visual_index(&sc, mods).unwrap();
            assert_eq!(recovered, n, "index {} round-trip mismatch", n);
        }
    }

    #[test]
    fn index_above_8_is_unsupported() {
        assert!(
            index_to_shortcut(9, Modifiers::SUPER).is_none(),
            "上限是 ⌘9"
        );
        assert!(index_to_shortcut(100, Modifiers::SUPER).is_none());
    }

    #[test]
    fn non_matching_modifier_is_ignored() {
        let sc = Shortcut::new(Some(Modifiers::CONTROL), Code::Digit1);
        assert_eq!(shortcut_to_visual_index(&sc, Modifiers::SUPER), None);
    }

    #[test]
    fn control_modifier_round_trip() {
        let mods = Modifiers::CONTROL;
        let sc = index_to_shortcut(2, mods).unwrap();
        assert_eq!(shortcut_to_visual_index(&sc, mods), Some(2));
        assert_eq!(shortcut_to_visual_index(&sc, Modifiers::SUPER), None);
    }

    #[test]
    fn non_digit_non_enter_key_is_ignored() {
        let sc = Shortcut::new(Some(Modifiers::SUPER), Code::KeyA);
        assert_eq!(shortcut_to_visual_index(&sc, Modifiers::SUPER), None);
    }

    #[test]
    fn parse_popup_shortcut_modifiers_maps_tokens() {
        assert_eq!(
            parse_popup_shortcut_modifiers("CmdOrCtrl+Shift"),
            Modifiers::SUPER | Modifiers::SHIFT
        );
        assert_eq!(
            parse_popup_shortcut_modifiers("Control+Alt"),
            Modifiers::CONTROL | Modifiers::ALT
        );
        assert_eq!(
            parse_popup_shortcut_modifiers("garbage"),
            Modifiers::SUPER,
            "未知 token 回落 SUPER",
        );
    }

    #[test]
    fn format_popup_shortcut_mod_prefix_renders_glyphs() {
        assert_eq!(format_popup_shortcut_mod_prefix("CmdOrCtrl"), "\u{2318}");
        assert_eq!(
            format_popup_shortcut_mod_prefix("CmdOrCtrl+Shift"),
            "\u{2318}\u{21e7}"
        );
        assert_eq!(
            format_popup_shortcut_mod_prefix("Control+Alt"),
            "\u{2303}\u{2325}"
        );
    }

    #[test]
    fn popup_shortcuts_are_not_plan_gated() {
        assert!(!should_block_popup_shortcut_for_plan(false));
        assert!(!should_block_popup_shortcut_for_plan(true));
    }

    #[test]
    fn parse_full_shortcut_default_is_cmd_semicolon() {
        let sc = parse_full_shortcut("CmdOrCtrl+Semicolon").expect("default parses");
        assert_eq!(sc.mods, Modifiers::SUPER);
        assert_eq!(sc.key, Code::Semicolon);
    }

    #[test]
    fn parse_full_shortcut_accepts_multi_modifier() {
        let sc = parse_full_shortcut("Control+Shift+KeyK").expect("parses");
        assert_eq!(sc.mods, Modifiers::CONTROL | Modifiers::SHIFT);
        assert_eq!(sc.key, Code::KeyK);
    }

    #[test]
    fn parse_full_shortcut_rejects_pure_shift_or_alt() {
        assert!(parse_full_shortcut("Shift+KeyK").is_none());
        assert!(parse_full_shortcut("Alt+KeyK").is_none());
        assert!(parse_full_shortcut("Shift+Alt+KeyK").is_none());
    }

    #[test]
    fn parse_full_shortcut_rejects_bad_input() {
        assert!(parse_full_shortcut("").is_none());
        assert!(parse_full_shortcut("CmdOrCtrl").is_none(), "缺 key");
        assert!(parse_full_shortcut("CmdOrCtrl+NotAKey").is_none());
        assert!(
            parse_full_shortcut("Garbage+KeyK").is_none(),
            "未知 modifier"
        );
    }

    #[test]
    fn parse_popup_shortcut_modifiers_pure_shift_falls_back_to_super() {
        assert_eq!(parse_popup_shortcut_modifiers("Shift"), Modifiers::SUPER);
        assert_eq!(
            parse_popup_shortcut_modifiers("Shift+Alt"),
            Modifiers::SUPER,
            "无 SUPER/CONTROL 组合视为非法,回落 SUPER",
        );
    }
}
