//! 全屏彩带覆盖层。
//!
//! 为什么单独开一个窗口:`canvas-confetti` 把 canvas 挂在「当前窗口」的 DOM 上,
//! 所以在 Settings 窗口里放彩带只能铺满 Settings 那一块矩形。要真正铺满整个
//! 显示器,得另开一个**透明、置顶、点击穿透**、尺寸等于整屏的 webview 窗口,
//! 让彩带在它里面放。
//!
//! 这里不复用 popup 的 NSPanel 子类:那套是为「不激活 app / 跨 Space 浮动 /
//! 不抢焦点」设计的(popup 在 app 不在前台时也要弹)。而 celebrate 永远是在
//! 用户正盯着 Settings、app 已在前台时触发,需求简单得多——一个普通
//! always-on-top + ignore-cursor-events 的透明窗口即可,且能跨平台少写一份。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_LABEL: &str = "confetti-overlay";

/// 彩带兜底自毁时长(ms)。前端正常会在播放结束后自己 invoke 关窗;
/// 这里的兜底是防止前端异常(没加载/报错)导致透明窗口永远赖在屏上挡操作。
/// 取值 = 前端动画时长(~4.2s)+ 前端兜底(5.5s)+ 余量,确保只在前端彻底失灵时才兜。
const OVERLAY_FALLBACK_MS: u64 = 7500;

/// 弹出一个铺满当前屏幕的全屏彩带覆盖层。fire-and-forget。
///
/// 选屏策略:跟 popup 一样优先「光标所在屏」,这样多屏时彩带落在用户正在
/// 看的那块屏(Settings 窗口通常也在那)。拿不到就退回主屏。
pub fn show_confetti_overlay(app: &AppHandle) {
    // 已经有一个在放就不叠第二个,避免重复触发(连点激活/校验)时堆窗口。
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        crate::log_debug!("confetti", "overlay already present, skip");
        return;
    }

    let monitor =
        crate::monitors::cursor_monitor(app).or_else(|| app.primary_monitor().ok().flatten());
    let (x, y, w, h) = match monitor.as_ref() {
        Some(m) => crate::monitors::monitor_logical_metrics(m),
        None => {
            crate::log_warn!("confetti", "no monitor found, skip overlay");
            return;
        }
    };

    let builder = WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("index.html#kind=confetti".into()),
    )
    .title("")
    .inner_size(w, h)
    .position(x, y)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    // 不抢焦点:彩带是纯装饰,不该把键盘/输入焦点从 Settings 抢走。
    .focused(false)
    .skip_taskbar(true)
    .transparent(true)
    .shadow(false);

    let window = match builder.build() {
        Ok(win) => win,
        Err(e) => {
            crate::log_error!("confetti", "build overlay failed: {}", e);
            return;
        }
    };

    // 点击穿透:鼠标事件直接落到下层窗口,彩带覆盖层不拦截任何点击。
    // 失败不致命(顶多挡一下),只记日志。
    if let Err(e) = window.set_ignore_cursor_events(true) {
        crate::log_warn!("confetti", "set_ignore_cursor_events failed: {}", e);
    }

    crate::log_info!(
        "confetti",
        "overlay shown {:.0}x{:.0} at ({:.0},{:.0})",
        w,
        h,
        x,
        y
    );

    // 兜底自毁:前端异常时也能保证窗口最终消失,不会留一个透明窗赖在屏上。
    let app_for_close = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(OVERLAY_FALLBACK_MS)).await;
        if let Some(win) = app_for_close.get_webview_window(OVERLAY_LABEL) {
            let _ = win.destroy();
            crate::log_debug!("confetti", "overlay fallback-destroyed");
        }
    });
}

/// 关闭彩带覆盖层(前端播放结束后主动调)。幂等:窗口不在时静默返回。
pub fn close_confetti_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = win.destroy();
        crate::log_debug!("confetti", "overlay closed by frontend");
    }
}
