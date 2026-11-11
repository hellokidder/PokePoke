//! Session panel（托盘点击弹出的会话列表窗口）生命周期。
//!
//! 从 `tray.rs` 抽出（AGENTS.md §4a：大文件按视图/功能拆 mod 目录）。含
//! CAS 防重入、prewarm、NSPanel 构建与 `Focused(false)` 自动隐藏。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;
use tauri::{AppHandle, Manager, WebviewUrl};
#[cfg(target_os = "macos")]
use tauri::{LogicalPosition, LogicalSize, Position, Size};
#[cfg(target_os = "macos")]
use tauri_nspanel::{CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask};

/// 防止 tray 连点时重复排队 `build_panel_nspanel`（label 撞车）。
static PANEL_CREATING: AtomicBool = AtomicBool::new(false);
/// 预热尚未完成时用户已点 tray：建好后立刻 show，并落到这次点击的坐标。
static PANEL_PENDING_SHOW: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// `build_panel_nspanel` 的 CAS 复位守卫：函数内任何 panic（如 PanelBuilder
/// 内部 assert）都会经 Drop 复位 `PANEL_CREATING`，否则后续所有 tray 点击
/// 都被判 "create in progress, queued show" 而排队空转，面板永久打不开。
struct PanelCreatingGuard;
impl Drop for PanelCreatingGuard {
    fn drop(&mut self) {
        PANEL_CREATING.store(false, Ordering::SeqCst);
    }
}

// Session panel 自己的 NSPanel 子类。
//
// 跟 popup 那个 PokePopupPanel 的关键差别：
//   can_become_key_window: true
//
// session panel 必须能拿/丢 key window，否则 `WindowEvent::Focused(false)`
// 永远不会触发，失焦自动关闭整条链全断。NSPanel 提供原生的 resignKey
// 派发——只要它能成为 key，AppKit 就会在用户点别的窗口时正常发通知；
// nonactivating_panel style mask 又保证拿 key 不会让我们这个 Accessory
// 进程激活到 Regular（不出 Dock 图标、不抢 menu bar）。
//
// 为什么不直接用 WebviewWindow + always_on_top？因为 Accessory 进程下普通
// NSWindow 在跨进程激活时（用户点 iTerm）AppKit 不发 resignKey notification，
// Tauri 的 Focused(false) 也就不来——实测 panel 卡死必须手动再点 tray 才
// 能收掉。NSPanel + NonactivatingPanel 是 macOS 给"popover/spotlight 类
// 浮动窗"的标准模型，专门处理这个场景。
//
// `tauri_panel!` 展开会引入一堆 `use` 项(NSEvent / Bool / define_class 等)，
// 同一模块里第二次调用就会 E0252 name defined multiple times。所以每个
// panel 类单独包一层子 mod 隔离命名空间，再 `pub use` 出来。
#[cfg(target_os = "macos")]
mod session_panel_class {
    use tauri::Manager;
    use tauri_nspanel::tauri_panel;
    tauri_panel! {
        panel!(PokeSessionPanel {
            config: {
                can_become_key_window: true,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })
    }
}
#[cfg(target_os = "macos")]
use session_panel_class::PokeSessionPanel;

const PANEL_W: f64 = 380.0;
const PANEL_H: f64 = 520.0;

pub(crate) fn toggle_panel(app: &AppHandle, click_x: f64, click_y: f64) {
    let (x, y) = compute_panel_position(app, click_x, click_y);

    // 存在判定一律走 get_webview_window：tauri-nspanel 的 panel manager
    // 和 webview manager 注册不一致（hide 后 get_webview_panel 会返回
    // Err 但 webview 这边仍持有 label，导致重复 create label 撞车）。
    // webview manager 是底层 source of truth。
    if let Some(window) = app.get_webview_window("panel") {
        #[cfg(target_os = "macos")]
        {
            if window.is_visible().unwrap_or(false) {
                crate::log_debug!("panel", "toggle_panel: hiding visible panel");
                // NSPanel 必须 hide 不能 destroy——destroy 会触发 AppKit
                // 的 last-window-close 链路把整个 Accessory 进程拉退出。
                let _ = window.hide();
            } else {
                crate::log_debug!("panel", "toggle_panel: re-showing hidden panel");
                // 重新定位：用户可能换屏 / tray 点击位置变了
                let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
                show_existing_panel(app);
            }
            return;
        }
        #[cfg(not(target_os = "macos"))]
        {
            // 非 macOS：destroy + recreate（沿用旧路径，没有"关进程"问题）
            crate::log_debug!("panel", "toggle_panel: existing window found, destroying");
            let _ = window.destroy();
            return;
        }
    }

    if PANEL_CREATING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        if let Ok(mut pending) = PANEL_PENDING_SHOW.lock() {
            *pending = Some((x, y));
        }
        crate::log_debug!("panel", "toggle_panel: create in progress, queued show");
        return;
    }
    crate::log_debug!("panel", "toggle_panel: no existing window, creating");
    create_panel_window(app, x, y, true);
}

/// 启动后后台预热 panel webview，避免 dev 冷启动后第一次点 tray 卡在
/// PanelBuilder + Vite 首包编译（用户体感「点完等很久才出来」）。
///
/// **取舍(刻意无配置开关)**:
/// - 主实例在 setup 后 ~400ms 构建一个**常驻隐藏** panel webview,直到进程退出;
///   即使用户从不点 tray,也会占用一份 WKWebView 内存,换首点 tray 低延迟。
/// - 须在 [`crate::http_server::bind_or_detect_peer`] 成功之后调用(`lib.rs`),
///   否则被单实例顶掉的第二进程会在退出前白建一次 webview。
pub(crate) fn prewarm_session_panel(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // 让 setup / tray 先落稳，别跟首屏抢主线程。
        std::thread::sleep(Duration::from_millis(400));
        let app = app.clone();
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            if app_for_main.get_webview_window("panel").is_some() {
                return;
            }
            if PANEL_CREATING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return;
            }
            crate::log_info!("panel", "prewarming session panel webview");
            let (x, y) = default_panel_position(&app_for_main);
            build_panel_nspanel(&app_for_main, x, y, false);
        });
    });
}

fn default_panel_position(app: &AppHandle) -> (f64, f64) {
    let (mon_x_log, mon_y_log, mon_w_log) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let (x, y, w, _) = crate::monitors::monitor_logical_metrics(&m);
            (x, y, w)
        })
        .unwrap_or((0.0, 0.0, 1920.0));
    (mon_x_log + (mon_w_log - PANEL_W) / 2.0, mon_y_log + 30.0)
}

fn finish_panel_create(app: &AppHandle, show: bool) {
    PANEL_CREATING.store(false, Ordering::SeqCst);
    let pending = PANEL_PENDING_SHOW
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if !show {
        if let Some((x, y)) = pending {
            // build 失败时窗口不存在，queued show 无处落地；CAS 已复位，下次
            // tray 点击会重新走创建，这里只告警，不静默吞掉这次点击。
            let Some(window) = app.get_webview_window("panel") else {
                crate::log_warn!(
                    "panel",
                    "finish_panel_create: prewarm produced no window, dropping queued tray show (x={}, y={})",
                    x, y
                );
                return;
            };
            crate::log_debug!("panel", "prewarm complete, honoring queued tray show");
            let _ = window.set_position(tauri::LogicalPosition::new(x, y));
            #[cfg(target_os = "macos")]
            show_existing_panel(app);
            #[cfg(not(target_os = "macos"))]
            {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }
}

/// macOS:Accessory 进程下 NSPanel 初次 show 偶发不真正上屏（popup 已踩过）。
/// 补一次延迟 orderFrontRegardless 并落 is_visible 诊断日志。
#[cfg(target_os = "macos")]
fn schedule_panel_visibility_retry(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        let app_for_retry = app.clone();
        let _ = app.run_on_main_thread(move || match app_for_retry.get_webview_panel("panel") {
            Ok(panel) => {
                panel.order_front_regardless();
                crate::log_info!(
                    "panel",
                    "visibility retry is_visible={}",
                    panel.is_visible(),
                );
            }
            Err(e) => {
                crate::log_warn!("panel", "visibility retry miss err={:?}", e);
            }
        });
    });
}

/// 把已经存在但被 hide 的 NSPanel 重新 show 出来。
/// 优先走 panel manager 的 show_and_make_key 触发 becomeKey notification；
/// 拿不到 panel handle 时兜底用 webview window 的 show + set_focus。
#[cfg(target_os = "macos")]
fn show_existing_panel(app: &AppHandle) {
    if let Ok(panel) = app.get_webview_panel("panel") {
        panel.show_and_make_key();
        panel.order_front_regardless();
        schedule_panel_visibility_retry(app);
        return;
    }
    if let Some(win) = app.get_webview_window("panel") {
        crate::log_debug!(
            "panel",
            "show_existing_panel: panel manager miss, falling back to webview show"
        );
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 把 tray click 的物理坐标定位到正确的屏幕,并算出 panel 在虚拟桌面坐标系
/// (Tauri/Tao 的 LogicalPosition 原点 = primary monitor 左上角,副屏会有
/// 正/负偏移)里的位置。多屏场景:用户在副屏点 tray icon,panel 应该弹在
/// 副屏顶部该屏内,而不是回到主屏。
fn compute_panel_position(app: &AppHandle, click_x_phys: f64, click_y_phys: f64) -> (f64, f64) {
    let monitor = crate::monitors::pick_monitor_at_point(app, click_x_phys, click_y_phys)
        .or_else(|| app.primary_monitor().ok().flatten());

    let (mon_x_log, mon_y_log, mon_w_log, scale) = match monitor {
        Some(m) => {
            let scale = m.scale_factor();
            let (x_log, y_log, w_log, _h_log) = crate::monitors::monitor_logical_metrics(&m);
            (x_log, y_log, w_log, scale)
        }
        None => (0.0, 0.0, 1920.0, 1.0),
    };

    let logical_click_x = click_x_phys / scale;
    let local_x = logical_click_x - mon_x_log;
    let x_in_mon = (local_x - PANEL_W / 2.0).clamp(8.0, (mon_w_log - PANEL_W - 8.0).max(8.0));
    (mon_x_log + x_in_mon, mon_y_log + 30.0)
}

/// 隐藏 session panel（外部命令 / Focused(false) 自动关都走这里）。
///
/// macOS 走 hide() / orderOut——destroy NSPanel 会触发 AppKit 的
/// last-window-close 链路把整个 Accessory 进程拉退出。webview 状态保留，
/// 下次 show_existing_panel 直接复用。
///
/// 非 macOS 仍 destroy（保持原有"重建 webview 让 React 重新挂载"行为）。
///
/// 注意：始终从 get_webview_window 拿窗口句柄。tauri-nspanel 的 panel
/// manager 会在某些时机和 webview manager 失同步（hide 后 get_webview_panel
/// 偶发返回 Err），webview 这边是底层 source of truth。
pub(crate) fn hide_session_panel(app: &AppHandle) {
    let Some(window) = app.get_webview_window("panel") else {
        return;
    };
    #[cfg(target_os = "macos")]
    let _ = window.hide();
    #[cfg(not(target_os = "macos"))]
    let _ = window.destroy();
}

#[cfg(target_os = "macos")]
fn create_panel_window(app: &AppHandle, x: f64, y: f64, show: bool) {
    // PanelBuilder + AppKit 的 NSPanel 初始化要求主线程。tray click handler
    // 一般跑在 Tauri 主线程，但保留 run_on_main_thread 派发跟 popup 一致，
    // 避免日后被搬到非主线程时静默崩溃。
    let app_clone = app.clone();
    let _ = app.run_on_main_thread(move || {
        build_panel_nspanel(&app_clone, x, y, show);
    });
}

#[cfg(not(target_os = "macos"))]
fn create_panel_window(app: &AppHandle, x: f64, y: f64, show: bool) {
    build_panel_nspanel(app, x, y, show);
}

#[cfg(target_os = "macos")]
fn build_panel_nspanel(app: &AppHandle, x: f64, y: f64, show: bool) {
    // panic 时也要复位 CAS（finish_panel_create 只在正常/Err 分支跑）。
    let _guard = PanelCreatingGuard;
    let result = PanelBuilder::<_, PokeSessionPanel>::new(app, "panel")
        .url(WebviewUrl::App("index.html".into()))
        .position(Position::Logical(LogicalPosition::new(x, y)))
        .size(Size::Logical(LogicalSize::new(PANEL_W, PANEL_H)))
        .level(PanelLevel::Floating)
        .has_shadow(false)
        .collection_behavior(CollectionBehavior::new().can_join_all_spaces().stationary())
        // hides_on_deactivate=false：app 激活状态不该影响 panel 可见性；
        // panel 关闭时机由"用户点别的窗口" → resignKey 驱动，跟 app 激活
        // 没关系。
        .hides_on_deactivate(false)
        // 关键：nonactivating_panel + can_become_key_window=true（panel!
        // macro config 里）→ panel 能拿/丢 key，但不会激活 PokePoke 进程。
        // resignKey notification 会在用户点别的窗口时正常派发，Tauri 的
        // Focused(false) 也跟着稳。
        .style_mask(StyleMask::empty().nonactivating_panel())
        .no_activate(true)
        .with_window(move |w| {
            w.title("Poke Poke")
                .decorations(false)
                .resizable(false)
                .transparent(true)
                .accept_first_mouse(true)
                .skip_taskbar(true)
                .visible(show)
        })
        .build();

    match result {
        Ok(panel) => {
            attach_panel_events(app);
            if show {
                // makeKeyAndOrderFront：直接 show() 只 orderFront、不升 key，
                // resignKey 永远没机会触发——必须在创建时就拿 key。
                // order_front_regardless：Accessory 进程下 show 偶发不上屏，跟 popup 同补一刀。
                panel.show_and_make_key();
                panel.order_front_regardless();
                let visible_now = panel.is_visible();
                crate::log_info!(
                    "panel",
                    "create_panel_window: NSPanel built is_visible={}",
                    visible_now,
                );
                schedule_panel_visibility_retry(app);
            } else {
                // PanelBuilder.build() 在 macOS 上仍可能短暂 orderFront；
                // 预热必须 orderOut，否则启动后 panel 会自己弹出来。
                if let Some(win) = app.get_webview_window("panel") {
                    let _ = win.hide();
                }
                crate::log_info!("panel", "prewarm: NSPanel built (hidden)");
            }
        }
        Err(err) => {
            crate::log_error!(
                "panel",
                "create_panel_window: NSPanel build failed: {}",
                err
            );
        }
    }
    finish_panel_create(app, show);
}

#[cfg(not(target_os = "macos"))]
fn build_panel_nspanel(app: &AppHandle, x: f64, y: f64, show: bool) {
    // panic 时也要复位 CAS（finish_panel_create 只在正常/Err 分支跑）。
    let _guard = PanelCreatingGuard;
    let builder = WebviewWindowBuilder::new(app, "panel", WebviewUrl::App("index.html".into()))
        .title("Poke Poke")
        .inner_size(PANEL_W, PANEL_H)
        .position(x, y)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .transparent(true)
        .visible(show);

    match builder.build() {
        Ok(window) => {
            attach_panel_events(app);
            if show {
                let _ = window.set_focus();
                let _ = window.show();
                crate::log_debug!(
                    "panel",
                    "create_panel_window: window built, attaching window_event listener"
                );
            } else {
                let _ = window.hide();
                crate::log_info!("panel", "prewarm: window built (hidden)");
            }
        }
        Err(err) => {
            crate::log_error!("panel", "create_panel_window: build failed: {}", err);
        }
    }
    finish_panel_create(app, show);
}

/// 给 panel 绑 Focused/Destroyed 监听。NSPanel 和 WebviewWindow 共享 Tauri
/// 的 Window 抽象，监听 API 一致，不分平台。
fn attach_panel_events(app: &AppHandle) {
    let Some(window) = app.get_webview_window("panel") else {
        crate::log_error!("panel", "attach_panel_events: no window for label=panel");
        return;
    };
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(focused) = event {
            crate::log_debug!("panel", "WindowEvent::Focused({})", focused);
        }
        if let tauri::WindowEvent::Destroyed = event {
            crate::log_debug!("panel", "WindowEvent::Destroyed received");
        }
        if let tauri::WindowEvent::Focused(false) = event {
            // 走 hide_session_panel：macOS 上 NSPanel 必须 hide 不能 destroy
            // （destroy 会拉退出 Accessory 进程），其他平台仍 destroy。
            crate::log_debug!("panel", "hide via Rust Focused(false)");
            hide_session_panel(&app_handle);
        }
    });
}
