//! 静谧模式：倒计时内 session popup 照常弹出，但不播放提示音。

use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const PICKER_LABEL: &str = "quiet-mode";
const PICKER_W: f64 = 368.0;
const PICKER_H: f64 = 400.0;

#[derive(Debug, Clone, Serialize)]
pub struct QuietModeStatus {
    pub active: bool,
    pub remaining_secs: Option<u64>,
    /// 本次静谧模式总时长(秒),用于前端进度环;未激活时为 `None`。
    pub total_secs: Option<u64>,
}

#[derive(Debug)]
pub struct QuietModeState {
    ends_at: Mutex<Option<Instant>>,
    total_duration: Mutex<Option<Duration>>,
}

impl QuietModeState {
    pub fn new() -> Self {
        Self {
            ends_at: Mutex::new(None),
            total_duration: Mutex::new(None),
        }
    }

    pub fn is_active(&self) -> bool {
        self.remaining().is_some()
    }

    pub fn remaining(&self) -> Option<Duration> {
        let guard = self.ends_at.lock().unwrap_or_else(|p| p.into_inner());
        let end = (*guard)?;
        let now = Instant::now();
        if now >= end {
            None
        } else {
            Some(end - now)
        }
    }

    pub fn status(&self) -> QuietModeStatus {
        let remaining = self.remaining();
        let total_secs = self
            .total_duration
            .lock()
            .ok()
            .and_then(|g| *g)
            .map(|d| d.as_secs().max(1));
        QuietModeStatus {
            active: remaining.is_some(),
            remaining_secs: remaining.map(|d| d.as_secs().max(1)),
            total_secs: if remaining.is_some() {
                total_secs
            } else {
                None
            },
        }
    }

    /// 启动静谧模式；返回计划结束时刻。
    pub fn start(&self, duration: Duration) -> Instant {
        let end = Instant::now() + duration;
        {
            let mut guard = self.ends_at.lock().unwrap_or_else(|p| p.into_inner());
            *guard = Some(end);
        }
        {
            let mut total = self
                .total_duration
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *total = Some(duration);
        }
        end
    }

    pub fn clear(&self) {
        let mut guard = self.ends_at.lock().unwrap_or_else(|p| p.into_inner());
        *guard = None;
        let mut total = self
            .total_duration
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *total = None;
    }
}

impl Default for QuietModeState {
    fn default() -> Self {
        Self::new()
    }
}

fn emit_changed(app: &AppHandle, state: &QuietModeState) {
    let _ = app.emit("quiet-mode-changed", state.status());
}

/// 菜单栏 tray title 用的剩余时间,格式 `MM:SS`。
pub fn format_menu_bar_countdown(remaining: Duration) -> String {
    let total = remaining.as_secs().max(1);
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// 静谧模式状态变化时:推前端事件 + 刷新托盘 title / 菜单。
pub fn notify_state_changed(app: &AppHandle, state: &QuietModeState) {
    emit_changed(app, state);
    crate::tray::sync_quiet_mode_display(app);
}

/// 静谧模式进行中每秒刷新菜单栏倒计时;`end` 对不上时自动退出。
pub fn spawn_tray_title_ticker(app: AppHandle, state: Arc<QuietModeState>, end: Instant) {
    std::thread::spawn(move || loop {
        let still_same = state
            .ends_at
            .lock()
            .map(|g| *g == Some(end))
            .unwrap_or(false);
        if !still_same || state.remaining().is_none() {
            break;
        }
        crate::tray::sync_quiet_mode_title(&app);
        std::thread::sleep(Duration::from_secs(1));
    });
}

/// 到期后清理状态并通知前端 / 托盘。
pub fn spawn_expiry_watcher(app: AppHandle, state: Arc<QuietModeState>, end: Instant) {
    std::thread::spawn(move || {
        let sleep_for = end.saturating_duration_since(Instant::now());
        std::thread::sleep(sleep_for);
        let still_active = state
            .ends_at
            .lock()
            .map(|g| g.map(|e| e == end).unwrap_or(false))
            .unwrap_or(false);
        if still_active {
            state.clear();
            notify_state_changed(&app, &state);
            crate::log_info!("quiet_mode", "countdown ended");
        }
    });
}

pub fn toggle_quiet_mode_picker(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PICKER_LABEL) {
        let _ = win.destroy();
        return;
    }
    open_quiet_mode_picker_inner(app);
}

/// 仅打开,不做 toggle 检查(由 commands/shortcut.rs 调)
pub fn open_quiet_mode_picker(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PICKER_LABEL) {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    open_quiet_mode_picker_inner(app);
}

fn open_quiet_mode_picker_inner(app: &AppHandle) {
    // 优先读上次拖拽位置,None=居中;同时预载 ui_theme 供首帧同步,避免按钮亮色闪一下
    let store = app.state::<std::sync::Arc<std::sync::Mutex<crate::settings::SettingsStore>>>();
    let (x, y, ui_theme) = store
        .lock()
        .map(|g| {
            let s = &g.settings;
            let pos = s.quiet_picker_last_position.unwrap_or_else(|| {
                let (mon_x, mon_y, mon_w, mon_h) = crate::monitors::cursor_monitor(app)
                    .as_ref()
                    .map(crate::monitors::monitor_logical_metrics)
                    .unwrap_or((0.0, 0.0, 1920.0, 1080.0));
                (
                    mon_x + (mon_w - PICKER_W) / 2.0,
                    mon_y + (mon_h - PICKER_H) / 2.0,
                )
            });
            (pos.0, pos.1, s.ui_theme)
        })
        .unwrap_or_else(|_| {
            let (mon_x, mon_y, mon_w, mon_h) = crate::monitors::cursor_monitor(app)
                .as_ref()
                .map(crate::monitors::monitor_logical_metrics)
                .unwrap_or((0.0, 0.0, 1920.0, 1080.0));
            (
                mon_x + (mon_w - PICKER_W) / 2.0,
                mon_y + (mon_h - PICKER_H) / 2.0,
                crate::settings::UiTheme::System,
            )
        });
    let theme = match ui_theme {
        crate::settings::UiTheme::Light => "light",
        crate::settings::UiTheme::Dark => "dark",
        crate::settings::UiTheme::System => "system",
    };

    // 预载状态塞进 URL hash:前端首帧直接渲染,无 idle→active 闪烁
    let state = app.state::<Arc<QuietModeState>>();
    let status = state.status();
    let hash = if status.active {
        format!(
            "#kind=quiet-mode&theme={theme}&active=1&remaining={}&total={}",
            status.remaining_secs.unwrap_or(0),
            status.total_secs.unwrap_or(0),
        )
    } else {
        format!("#kind=quiet-mode&theme={theme}")
    };

    let built = WebviewWindowBuilder::new(app, PICKER_LABEL, WebviewUrl::App(hash.into()))
        .title("")
        .inner_size(PICKER_W, PICKER_H)
        .position(x, y)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .transparent(true)
        .build();

    if let Ok(window) = built {
        let _ = window.set_focus();

        // 拖拽结束时写回位置到 settings:Moved 事件频率极高,debounce 500ms
        let app_clone = app.clone();
        let win_clone = window.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::Moved(_) = event {
                let app = app_clone.clone();
                let win = win_clone.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if let Ok(pos) = win.outer_position() {
                        let store = app.state::<std::sync::Arc<std::sync::Mutex<crate::settings::SettingsStore>>>();
                        let settings_clone = store.lock().ok().map(|g| g.settings.clone());
                        if let Some(mut s) = settings_clone {
                            s.quiet_picker_last_position = Some((pos.x as f64, pos.y as f64));
                            if let Ok(mut g) = store.lock() {
                                g.update(s);
                            }
                        }
                    }
                });
            }
        });
    }
}

pub fn close_quiet_mode_picker(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PICKER_LABEL) {
        let _ = win.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_remaining() {
        let state = QuietModeState::new();
        assert!(!state.is_active());
        let end = state.start(Duration::from_secs(60));
        assert!(state.is_active());
        assert!(
            state
                .remaining()
                .expect("remaining 应处于 active 态")
                .as_secs()
                <= 60
        );
        assert!(end > Instant::now());
    }

    #[test]
    fn clear_ends_quiet_mode() {
        let state = QuietModeState::new();
        state.start(Duration::from_secs(30));
        state.clear();
        assert!(!state.is_active());
        assert_eq!(state.status().remaining_secs, None);
        assert_eq!(state.status().total_secs, None);
    }

    #[test]
    fn menu_bar_countdown_uses_mm_ss() {
        assert_eq!(format_menu_bar_countdown(Duration::from_secs(1)), "00:01");
        assert_eq!(format_menu_bar_countdown(Duration::from_secs(60)), "01:00");
        assert_eq!(format_menu_bar_countdown(Duration::from_secs(655)), "10:55");
        assert_eq!(
            format_menu_bar_countdown(Duration::from_secs(27 * 60)),
            "27:00"
        );
    }
}
