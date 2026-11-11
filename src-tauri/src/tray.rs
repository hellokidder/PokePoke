use crate::quiet_mode::QuietModeState;
use crate::settings::TrayIconStyle;
use std::sync::Arc;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};

mod panel;
pub(crate) use panel::{hide_session_panel, prewarm_session_panel};

/// 根据 style 选 tray 图标 + 是否走 macOS template image 渲染。
/// - Colored: 彩色八爪鱼,debug build 走 hue-shift dev 图,release 走 prod 图。
/// - White: 固定白色八爪鱼(纯白像素 + 保留 alpha),`icon_as_template(false)`
///   让 macOS 不反色,深色菜单栏直接显示白色——这是用户的明确视觉选择,
///   不是 template 自适应模式(template 在浅色菜单栏会变黑反差不够,
///   实测被用户拒绝)。不区分 dev/prod。
fn pick_tray_icon(style: TrayIconStyle) -> (&'static [u8], bool) {
    match style {
        TrayIconStyle::Colored => {
            #[cfg(debug_assertions)]
            let bytes: &[u8] = include_bytes!("../icons/tray-default-dev.png");
            #[cfg(not(debug_assertions))]
            let bytes: &[u8] = include_bytes!("../icons/tray-default.png");
            (bytes, false)
        }
        TrayIconStyle::White => (include_bytes!("../icons/tray-default-white.png"), false),
    }
}

pub fn create_tray(app: &AppHandle, style: TrayIconStyle) -> tauri::Result<()> {
    // Dev 模式 colored 走绿色章鱼、Release colored 紫色,方便一眼区分实例;
    // mono 模式不区分 dev/prod,统一交给 macOS template 反色。
    // 素材由 scripts/gen-dev-icons.swift 从源图生成。
    let (tray_bytes, is_template) = pick_tray_icon(style);

    let tray_icon = Image::from_bytes(tray_bytes).unwrap_or_else(|_| {
        Image::from_bytes(include_bytes!("../icons/32x32.png"))
            .expect("bundled fallback tray icon must decode")
    });

    let menu = build_tray_menu(app)?;

    TrayIconBuilder::with_id("main")
        .icon(tray_icon)
        .icon_as_template(is_template)
        .tooltip("Poke Poke")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => open_settings_window(app),
            "quiet_mode_end" => handle_quiet_mode_end(app),
            "quit" => {
                crate::log_info!("app", "exiting reason=tray_quit");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                panel::toggle_panel(tray.app_handle(), position.x, position.y);
            }
        })
        .build(app)?;

    Ok(())
}

/// 设置面板下拉切换 style 时调用,运行时换 tray 图标 + template flag,
/// 不重建整个 tray(MenuBuilder 调用一次足矣)。
pub fn apply_tray_icon_style(app: &AppHandle, style: TrayIconStyle) {
    let Some(tray) = app.tray_by_id("main") else {
        crate::log_warn!("tray", "apply_tray_icon_style: tray 'main' not found");
        return;
    };
    let (bytes, is_template) = pick_tray_icon(style);
    let icon = match Image::from_bytes(bytes) {
        Ok(img) => img,
        Err(e) => {
            crate::log_error!("tray", "apply_tray_icon_style: decode failed: {}", e);
            return;
        }
    };
    if let Err(e) = tray.set_icon(Some(icon)) {
        crate::log_error!("tray", "apply_tray_icon_style: set_icon failed: {}", e);
    }
    if let Err(e) = tray.set_icon_as_template(is_template) {
        crate::log_error!(
            "tray",
            "apply_tray_icon_style: set_icon_as_template failed: {}",
            e
        );
    }
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let settings_item = MenuItemBuilder::with_id("settings", "Settings\u{2026}").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let mut menu_builder = MenuBuilder::new(app);
    if let Some(state) = app.try_state::<Arc<QuietModeState>>() {
        if let Some(remaining) = state.remaining() {
            let quiet_label = format!(
                "Quiet mode: {}",
                crate::quiet_mode::format_menu_bar_countdown(remaining)
            );
            let quiet_status_item =
                MenuItemBuilder::with_id("quiet_mode_status", quiet_label.as_str())
                    .enabled(false)
                    .build(app)?;
            let quiet_end_item =
                MenuItemBuilder::with_id("quiet_mode_end", "End Quiet Mode").build(app)?;
            menu_builder = menu_builder
                .item(&quiet_status_item)
                .item(&quiet_end_item)
                .separator();
        }
    }

    menu_builder.item(&settings_item).item(&quit_item).build()
}

fn handle_quiet_mode_end(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<QuietModeState>>() else {
        return;
    };
    if !state.is_active() {
        return;
    }
    state.clear();
    crate::quiet_mode::notify_state_changed(app, &state);
    crate::log_info!("quiet_mode", "cancelled from tray menu");
}

pub fn refresh_tray_menu(app: &AppHandle) {
    if let Ok(menu) = build_tray_menu(app) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// 静谧模式:菜单栏 title 显示剩余时间 `MM:SS`(macOS 上为蓝色)。
pub fn sync_quiet_mode_title(app: &AppHandle) {
    let title = app
        .try_state::<Arc<QuietModeState>>()
        .and_then(|state| {
            state
                .remaining()
                .map(crate::quiet_mode::format_menu_bar_countdown)
        })
        .unwrap_or_default();

    let Some(tray) = app.tray_by_id("main") else {
        return;
    };

    #[cfg(target_os = "macos")]
    {
        let title_for_tray = title.clone();
        if let Err(e) = tray.with_inner_tray_icon(move |inner| {
            if title_for_tray.is_empty() {
                inner.set_title(None::<&str>);
                if let Some(item) = inner.ns_status_item() {
                    crate::tray_quiet_title::clear(&item);
                }
            } else if let Some(item) = inner.ns_status_item() {
                crate::tray_quiet_title::set_blue_title(&item, &title_for_tray);
            } else {
                inner.set_title(Some(title_for_tray.as_str()));
            }
        }) {
            crate::log_warn!("tray", "quiet mode title update failed: {}", e);
        }
    }

    #[cfg(not(target_os = "macos"))]
    if title.is_empty() {
        let _ = tray.set_title::<&str>(None);
    } else if let Err(e) = tray.set_title(Some(title.as_str())) {
        crate::log_warn!("tray", "set_title failed: {}", e);
    }
}

/// 静谧模式状态变化时:刷新 title 并重建托盘菜单顶栏。
pub fn sync_quiet_mode_display(app: &AppHandle) {
    sync_quiet_mode_title(app);
    refresh_tray_menu(app);
}

const SETTINGS_W: f64 = 620.0;
// 左侧 sidebar tab 布局(general/popup/license/about)
// 长 form 拆 4 个面板,窗高从 1040 压到 600 —— 4 个 tab 中最高的是"通用"(3 个
// section ~310)+ header 58 + footer ~44 ≈ 412,留余量到 600。
// 1.0.1: popup tab 加了 focus_suppression toggle row
// 之后共 5 段(≈ 5×90 + header + footer ≈ 552),600 太挤;调到 720 给余量。
// 宽度也从 560 → 620,让 5 段呼吸更松。
// **唯一例外**:移动端推送 tab 内容随 provider 数量增长,在它上面 overflow:auto。
// 其它 tab 仍是 overflow:hidden 不出滚动条的设计契约,沿用原 0.0.3 注释精神。
const SETTINGS_H: f64 = 760.0;

// 水平锁死 SETTINGS_W、竖直仅设下限:min_width == max_width = 宽度不可变,
// 高度可自由拉伸(下限为最短 tab 可展示高度,避免内容被挤压)。
const MIN_SETTINGS_H: f64 = 400.0;

/// Open a standalone settings window (singleton). If already open, focus it.
pub fn open_settings_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    create_settings_window(app);
}

// Settings 层级/激活策略设计说明（1.0.1 走一圈弯路后定稿）：
//
// 现状：普通 WebviewWindow，`always_on_top=false`，**不切 activation policy**，
// app 全程 Accessory。
//
// 曾试过的错路：
//   A. 老方案：打开 settings 时切 Regular，关闭切回 Accessory —— 让 settings
//      进 Dock/Cmd+Tab。副作用：Regular app 一被 window server 事件唤起
//      (popup 弹出 order_front_regardless)，AppKit 会 order-front 所有
//      Normal-level window，settings 被顺带抬到 popup 之上遮住通知内容。
//   B. 中间方案：settings 也做成 NSPanel + Floating —— 顺带修 A 的副作用。
//      副作用：Floating level 就是"永远浮在其它 app 之上"，用户点其它 app
//      settings 也不会让位，体感是恼人的"钉在最上层"。
//
// 正解（当前）：普通 NSWindow + 保持 Accessory 不动。
//   - Settings 在 Normal window level，用户切走 app 时按 z-order 让位 ✓
//   - Popup 在 Floating level，永远盖过 Settings（这是通知的期望行为） ✓
//   - 没有 activation policy 切换 → 没有 AppKit
//     order-front-all-normal-windows 副作用 ✓
//
// 代价：Settings 不进 Dock、不进 Cmd+Tab。Accessory 应用的通用取舍，session
// panel 一直如此，用户对入口的心智已经在 tray 上，不再是问题。
#[cfg(target_os = "macos")]
fn create_settings_window(app: &AppHandle) {
    let (mon_x, mon_y, mon_w, mon_h) = crate::monitors::cursor_monitor(app)
        .as_ref()
        .map(crate::monitors::monitor_logical_metrics)
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0));
    let x = mon_x + (mon_w - SETTINGS_W) / 2.0;
    let y = mon_y + (mon_h - SETTINGS_H) / 2.0;

    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("")
        .inner_size(SETTINGS_W, SETTINGS_H)
        .position(x, y)
        .decorations(false)
        .always_on_top(false)
        .resizable(true)
        .min_inner_size(SETTINGS_W, MIN_SETTINGS_H)
        .max_inner_size(SETTINGS_W, f64::MAX)
        .skip_taskbar(true)
        .shadow(false)
        .transparent(true)
        .build();

    if let Ok(window) = built {
        let _ = window.set_focus();
    }
}

#[cfg(not(target_os = "macos"))]
fn create_settings_window(app: &AppHandle) {
    let (mon_x, mon_y, mon_w, mon_h) = crate::monitors::cursor_monitor(app)
        .as_ref()
        .map(crate::monitors::monitor_logical_metrics)
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0));
    let x = mon_x + (mon_w - SETTINGS_W) / 2.0;
    let y = mon_y + (mon_h - SETTINGS_H) / 2.0;

    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("")
        .inner_size(SETTINGS_W, SETTINGS_H)
        .position(x, y)
        .decorations(false)
        .always_on_top(false)
        .resizable(true)
        .min_inner_size(SETTINGS_W, MIN_SETTINGS_H)
        .max_inner_size(SETTINGS_W, f64::MAX)
        .skip_taskbar(false)
        .shadow(false)
        .transparent(true)
        .build();

    if let Ok(window) = built {
        let _ = window.set_focus();
    }
}
