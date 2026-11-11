//! popup 窗口构建 + URL 生成 + 快捷键 event。
//!
//! 从 `popup.rs` 抽出:`tauri_panel!` 宏(PokePopupPanel/PokePopupEventHandler,
//! macOS NSPanel 子类)+ `build_popup_window` x2(macOS PanelBuilder /
//! 非 macOS WebviewWindowBuilder)+ `session_popup_url`/`update_popup_url`
//! (hash 参数生成)+ `emit_popup_shortcuts_updated`(⌘N 角标 event)+
//! `schedule_popup_visibility_retry`/`attach_popup_hover_key_handler`(macOS
//! 上屏 / 悬停辅助)。行为零变化;`popup.rs` 引用保持 show/close 调用点不断。

use tauri::{AppHandle, Manager, WebviewUrl};

#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

use crate::settings::{PopupPosition, PopupSize};

use super::geometry::{popup_dimensions, PopupList};
use super::session_popup_shortcut_map;

// macOS：用 tauri-nspanel 把 popup 直接构造成 NSPanel
// （nonactivating_panel + no_activate=true），从窗口模型层面避免 popup
// 抢前台 app 焦点。鼠标悬停时短暂 make_key（hover_activate 模式）只为
// 让 WebKit 能更新 :hover / mouseenter，离开即 resign_key，键盘仍留在原 app。
#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use tauri::{LogicalPosition, LogicalSize, Position, Size};
#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
    TrackingAreaOptions,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    // PokePoke popup 的 NSPanel 子类：
    //   - can_become_key_window=true + becomes_key_only_if_needed：仅 hover 时
    //     短暂升 key，供 WebKit 更新 hover；平时不持 key，键盘不打断
    //   - can_become_main_window=false：不做 main window，不激活 app
    //   - is_floating_panel=true：浮动在普通窗口之上，等价于 NSFloatingWindowLevel
    //   - tracking_area.active_always：非前台 app 也能收到 mouse enter/exit
    panel!(PokePopupPanel {
        config: {
            can_become_key_window: true,
            becomes_key_only_if_needed: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
        with: {
            tracking_area: {
                options: TrackingAreaOptions::new()
                    .active_always()
                    .mouse_entered_and_exited()
                    .mouse_moved()
                    .cursor_update(),
                auto_resize: true,
            }
        }
    })

    panel_event!(PokePopupEventHandler {})
}

/// Tauri event:popup_list 变化后推给所有 popup webview,让它们重读自己的
/// ⌘N 角标编号。payload 是 `Vec<{ label, n }>`,前端按 label 找自己的项,n>=9
/// 不显示角标(全局快捷键上限 ⌘1~⌘9 + ⌘Enter)。
///
/// 也是全局快捷键 register/unregister 决策的同步点 -- lib.rs 监听同一 event
/// 决定 popup_list 空时反注册、非空时注册。
pub(super) fn emit_popup_shortcuts_updated(app: &AppHandle, popup_list: &PopupList) {
    use tauri::Emitter;
    let map = session_popup_shortcut_map(popup_list);
    let payload: Vec<serde_json::Value> = map
        .into_iter()
        .map(|(label, n)| serde_json::json!({ "label": label, "n": n }))
        .collect();
    let _ = app.emit("popup-shortcuts-updated", payload);
}

/// macOS:Accessory 进程下 NSPanel 初次 show 偶发不真正上屏;补一次延迟
/// orderFrontRegardless 并落 is_visible 诊断日志,便于真机 smoke。
#[cfg(target_os = "macos")]
fn schedule_popup_visibility_retry(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        let app_for_retry = app.clone();
        let _ = app.run_on_main_thread(move || match app_for_retry.get_webview_panel(&label) {
            Ok(panel) => {
                panel.order_front_regardless();
                crate::log_info!(
                    "popup",
                    "visibility retry label={} is_visible={}",
                    label,
                    panel.is_visible(),
                );
            }
            Err(e) => {
                crate::log_warn!("popup", "visibility retry miss label={} err={:?}", label, e,);
            }
        });
    });
}

/// macOS:鼠标进入 popup 时短暂 make_key、离开 resign_key，让 WebKit 能响应
/// hover / mouseenter。nonactivating_panel + no_activate 保证不激活 PokePoke
/// 进程、不抢前台 app 的键盘焦点（tauri-nspanel hover_activate 同款）。
#[cfg(target_os = "macos")]
fn attach_popup_hover_key_handler(app: &AppHandle, label: &str) {
    let Ok(panel) = app.get_webview_panel(label) else {
        crate::log_warn!(
            "popup",
            "attach_popup_hover_key_handler: panel not found label={}",
            label
        );
        return;
    };

    let app_enter = app.clone();
    let label_enter = label.to_string();
    let app_exit = app.clone();
    let label_exit = label.to_string();

    let handler = PokePopupEventHandler::new();
    handler.on_mouse_entered(move |_event| {
        if let Ok(panel) = app_enter.get_webview_panel(&label_enter) {
            panel.make_key_window();
        }
    });
    handler.on_mouse_exited(move |_event| {
        if let Ok(panel) = app_exit.get_webview_panel(&label_exit) {
            panel.resign_key_window();
        }
    });
    panel.set_event_handler(Some(handler.as_ref()));
}

/// macOS:走 PanelBuilder 构造 NSPanel(不抢焦点)
#[cfg(target_os = "macos")]
pub(super) fn build_popup_window(
    app: &AppHandle,
    label: &str,
    x: f64,
    y: f64,
    url: &str,
    size: PopupSize,
) -> bool {
    let (w, h) = popup_dimensions(size);
    let result = PanelBuilder::<_, PokePopupPanel>::new(app, label)
        .url(WebviewUrl::App(url.to_string().into()))
        .position(Position::Logical(LogicalPosition::new(x, y)))
        .size(Size::Logical(LogicalSize::new(w, h)))
        .level(PanelLevel::Floating)
        .has_shadow(false)
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .stationary()
                .full_screen_auxiliary(),
        )
        .hides_on_deactivate(false)
        // 非 key 窗口也投递 mouseMoved（配合 hover key 回调）。
        .accepts_mouse_moved_events(true)
        .becomes_key_only_if_needed(true)
        // 必须显式 nonactivating_panel：告诉 AppKit 这个 panel 被点击时
        // 不让所属 app 激活到前台。
        .style_mask(StyleMask::empty().nonactivating_panel())
        // 创建期就声明不激活本进程；这样即便有第一次 show 的瞬间也不抢 key window。
        .no_activate(true)
        .with_window(|w| {
            w.title("")
                .decorations(false)
                .resizable(false)
                .transparent(true)
                .accept_first_mouse(true)
                .skip_taskbar(true)
        })
        .build();

    match result {
        Ok(panel) => {
            // show() 在 LSUIElement (Accessory) 进程下不强制 order front:
            // 如果 app 当前不在前台,新 panel 可能不真正显示出来,要等
            // app 下一次被激活(比如用户点 tray)。
            // order_front_regardless() 强制把 panel 排到前面但不抢 key,
            // 配合 nonactivating panel + no_activate 不会让进程活跃 / 抢焦点,
            // 但 panel 会立刻可见,跨 Space 跨屏都能看到。
            panel.show();
            panel.order_front_regardless();
            attach_popup_hover_key_handler(app, label);
            let visible_now = panel.is_visible();
            crate::log_info!(
                "popup",
                "build_popup_window ok label={} is_visible={}",
                label,
                visible_now,
            );
            schedule_popup_visibility_retry(app, label);
            true
        }
        Err(e) => {
            crate::log_error!(
                "popup",
                "build_popup_window FAILED label={} err={}",
                label,
                e
            );
            false
        }
    }
}

/// 非 macOS:沿用原来的 WebviewWindowBuilder
#[cfg(not(target_os = "macos"))]
pub(super) fn build_popup_window(
    app: &AppHandle,
    label: &str,
    x: f64,
    y: f64,
    url: &str,
    size: PopupSize,
) -> bool {
    let (w, h) = popup_dimensions(size);
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.to_string().into()))
        .title("")
        .inner_size(w, h)
        .position(x, y)
        .decorations(false)
        .always_on_top(true)
        .focused(false)
        .skip_taskbar(true)
        .resizable(false)
        .transparent(true)
        .shadow(false)
        .accept_first_mouse(true)
        .build();

    match window {
        Ok(win) => {
            let _ = win.show();
            true
        }
        Err(e) => {
            crate::log_error!(
                "popup",
                "build_popup_window FAILED (non-macOS) label={} err={}",
                label,
                e
            );
            false
        }
    }
}

/// 用 hash 而非 query 传 popup 参数:Tauri `WebviewUrl::App` 会对路径里的 `?`
/// 做 URL 编码(`?` -> `%3F`),结果 webview 加载 `index.html%3F...` 找不到文件 ->
/// 空白窗口。hash(`#`)是纯客户端片段,Tauri 不动,前端通过
/// `window.location.hash` 同步读到。
///
/// session popup 参数:`position`(同 0.0.2)+ `kind=session` + `n`(0-based
/// 视觉编号,>= 9 时前端不显示 ⌘N 角标)。
pub(super) fn session_popup_url(position: PopupPosition, n: usize, size: PopupSize) -> String {
    format!(
        "index.html#position={}&kind=session&n={}&size={}",
        position.as_kebab_str(),
        n,
        size.as_kebab_str(),
    )
}

/// updater 提示 popup 参数:`position` + `kind=update` + `version`。前端按
/// `kind=update` 渲染【立即升级】【跳过】两按钮,不显示 ⌘N 角标。
///
/// `version` 不做 URL encode:semver(0.0.3 / 1.0.0-rc.1)的字符集 `0-9 . -` 在
/// URL fragment 里都安全。**如果未来 version schema 引入 `+build` 之类带 `+` 的字符,
/// 必须改成 `percent_encoding::utf8_percent_encode`**,否则前端 `URLSearchParams` 会
/// 把 `+` 解码成空格。
pub(super) fn update_popup_url(position: PopupPosition, version: &str) -> String {
    format!(
        "index.html#position={}&kind=update&version={}",
        position.as_kebab_str(),
        version,
    )
}

#[cfg(test)]
mod tests {
    use super::{session_popup_url, update_popup_url};
    use crate::settings::{PopupPosition, PopupSize};

    #[test]
    fn session_popup_url_carries_position_kind_n_and_size() {
        // 用 hash 而非 query,绕开 Tauri WebviewUrl::App 对 `?` 的 URL 编码。
        // 0.0.3+:多带 kind=session 让前端按类型路由、n 用于 ⌘N 角标。
        // 0.0.10+:带 size 参数让前端切换渲染模式。
        assert_eq!(
            session_popup_url(PopupPosition::TopRight, 0, PopupSize::Default),
            "index.html#position=top-right&kind=session&n=0&size=default"
        );
        assert_eq!(
            session_popup_url(PopupPosition::BottomLeft, 4, PopupSize::Mini),
            "index.html#position=bottom-left&kind=session&n=4&size=mini"
        );
    }

    #[test]
    fn update_popup_url_carries_version() {
        assert_eq!(
            update_popup_url(PopupPosition::TopRight, "0.0.3"),
            "index.html#position=top-right&kind=update&version=0.0.3"
        );
    }
}
