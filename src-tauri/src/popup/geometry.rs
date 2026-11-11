//! popup 几何与定位:尺寸常量、屏幕度量、栈位计算。
//!
//! 从 `popup.rs` 抽出:POPUP_* 尺寸常量 + `popup_dimensions`/`read_popup_size`
//! + `MonitorMetrics`(同屏分组键)+ `PopupList` 类型 + `reserve_slot_and_position`
//! (一锁原子占位)+ `current_monitor_metrics`(选屏)+ `popup_x`/`target_y`
//! (角点计算)+ `read_popup_position`/`read_popup_auto_close_secs`(配置读取)。
//! 行为零变化;`popup.rs` 重导出 `PopupList`/`MonitorMetrics` 保持外部引用不断。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use crate::sessions::Session;
use crate::settings::{PopupPosition, PopupSize, SettingsStore};
use crate::surface::SurfaceEvidence;

use super::is_update_popup_label;

const POPUP_WIDTH: f64 = 360.0;
const POPUP_HEIGHT: f64 = 150.0;
const MINI_POPUP_WIDTH: f64 = 280.0;
const MINI_POPUP_HEIGHT: f64 = 42.0;
const POPUP_MARGIN: f64 = 12.0;
pub(super) const POPUP_GAP: f64 = 8.0;

/// 根据 popup 规格返回 (width, height)。
pub(super) fn popup_dimensions(size: PopupSize) -> (f64, f64) {
    match size {
        PopupSize::Default => (POPUP_WIDTH, POPUP_HEIGHT),
        PopupSize::Mini => (MINI_POPUP_WIDTH, MINI_POPUP_HEIGHT),
    }
}

/// 从 settings 读 popup_size 配置。
pub(super) fn read_popup_size(app: &AppHandle) -> PopupSize {
    app.try_state::<Arc<Mutex<SettingsStore>>>()
        .and_then(|s| s.lock().ok().map(|g| g.settings.popup_size))
        .unwrap_or_default()
}

/// Popup 所在屏的 logical 矩形,用作 PopupList 的“屏分组键”+ 重新定位时
/// 直接拿到尺寸。两个 popup 同屏 ⇔ MonitorMetrics 相等(用 round 后的 i64
/// 元组比较,避免浮点精度噪声)。
#[derive(Clone, Copy, Debug)]
pub struct MonitorMetrics {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl MonitorMetrics {
    /// (x, y) round 成 i64 的元组,作为 popup_list 同屏判定的“键”。
    pub fn screen_key(&self) -> (i64, i64) {
        (self.x.round() as i64, self.y.round() as i64)
    }
}

impl PartialEq for MonitorMetrics {
    fn eq(&self, other: &Self) -> bool {
        self.screen_key() == other.screen_key()
    }
}

/// `(label, monitor_metrics, token)`:每个 popup 记下它所在屏的 logical 矩形,
/// stack 偏移按“同屏 popup 数”算,而不是全局 list 长度——多屏环境下
/// 副屏已有 1 个 popup,主屏新弹的应该是该屏第 0 位,不是全局第 1 位。
///
/// `token` 是从 `POPUP_INSTANCE_COUNTER` 取出的全局单调递增 ID,用来区分
/// **同 label 的不同 popup 实例** —— label 是 `popup-{session_id}` 或
/// `popup-update-{version}`(0.0.3+ updater 提示用),会被同 session/version
/// 的下次 popup 复用;之前用 label 单独标识时,auto-close timer 会跨实例误关:
/// 旧 popup 被 focus-watcher / 用户点掉后,同 session 立刻新弹,旧 timer 一
/// 觉醒来 label 还在,就把新 popup 干掉了(体感“popup 闪都没闪”)。
pub type PopupList = Arc<Mutex<Vec<(String, MonitorMetrics, u64, PopupSize)>>>;

/// 一锁原子完成“算位置 + 占视觉编号 + push 到 popup_list”:
///   1. 读 same_screen_count(stack y 偏移)
///   2. 读 visual_index(⌘N 角标,update popup 不计入)
///   3. push 本实例
/// 全部在同一次 popup_list lock 内,关闭 0.0.3 M2 之前的 TOCTOU:并发两个
/// show_popup 之间 visual_index 与 stack 位置可能不一致一帧(emit_popup_shortcuts
/// _updated 会 ~50ms 内通过 store-emit 纠正前端角标,但 URL hash 里 baked 进去的
/// 初始 n 错了)。现在 push 之前 visual_index 就是本实例的最终位置。
///
/// 注意:current_monitor_metrics 走 AppleScript / Tauri monitor manager,**不**
/// 拿 popup_list lock,所以放在 lock 外。
pub(super) fn reserve_slot_and_position(
    app: &AppHandle,
    popup_list: &PopupList,
    position: PopupPosition,
    size: PopupSize,
    label: String,
    token: u64,
    session: Option<&Session>,
) -> (f64, f64, MonitorMetrics, usize) {
    let (mon_x, mon_y, mon_w, mon_h) = current_monitor_metrics(app, session);
    let metrics = MonitorMetrics {
        x: mon_x,
        y: mon_y,
        w: mon_w,
        h: mon_h,
    };

    let (same_screen, visual_index) = {
        let mut list = popup_list.lock().unwrap_or_else(|p| p.into_inner());
        let same = list.iter().filter(|(_, m, _, _)| *m == metrics).count();
        let vis = list
            .iter()
            .filter(|(l, _, _, _)| !is_update_popup_label(l))
            .count();
        list.push((label, metrics, token, size));
        (same, vis)
    };

    let (popup_w, popup_h) = popup_dimensions(size);
    let x = mon_x + popup_x(mon_w, position, popup_w);
    let y = mon_y + target_y(same_screen, position, mon_h, popup_h);
    (x, y, metrics, visual_index)
}

/// 根据用户的 `popup_screen_strategy` 设置选 popup 显示屏:
/// - `Terminal`(默认):popup 弹在触发事件的终端窗口所在屏 - AppleScript
///   按 session.terminal_tty 查 iTerm2 / Terminal 窗口 bounds 中心点找 monitor,
///   失败回退到 cursor 当前屏 -> primary
/// - `Primary`:始终主屏 - 适合“通知统一集中在主屏”的工作流
///
/// session=None 时 (reposition_after_remove 重排路径) 跳过 terminal lookup,
/// 直接走 cursor -> primary 链。fallback 走标准虚拟桌面 (0, 0, 1920, 1080)。
fn current_monitor_metrics(app: &AppHandle, session: Option<&Session>) -> (f64, f64, f64, f64) {
    let strategy = app
        .try_state::<Arc<Mutex<SettingsStore>>>()
        .and_then(|s| s.lock().ok().map(|g| g.settings.popup_screen_strategy))
        .unwrap_or_default();

    let (monitor, source_label) = match strategy {
        crate::settings::PopupScreenStrategy::Terminal => {
            if let Some(session) = session {
                let evidence = SurfaceEvidence::from_session(session);
                if let Some(m) = crate::monitors::surface_monitor(app, &evidence) {
                    (Some(m), "terminal")
                } else if let Some(m) = crate::monitors::cursor_monitor(app) {
                    (Some(m), "cursor")
                } else {
                    (app.primary_monitor().ok().flatten(), "primary")
                }
            } else if let Some(m) = crate::monitors::cursor_monitor(app) {
                (Some(m), "cursor")
            } else {
                (app.primary_monitor().ok().flatten(), "primary")
            }
        }
        crate::settings::PopupScreenStrategy::Primary => {
            (app.primary_monitor().ok().flatten(), "primary")
        }
    };

    crate::log_info!(
        "popup",
        "current_monitor_metrics: strategy={:?} chosen_via={} tty={:?} terminal_id={:?}",
        strategy,
        source_label,
        session.and_then(|s| s.terminal_tty.as_deref()),
        session.and_then(|s| s.terminal_id.as_deref()),
    );

    monitor
        .as_ref()
        .map(crate::monitors::monitor_logical_metrics)
        .unwrap_or((0.0, 0.0, 1920.0, 1080.0))
}

/// 给定屏幕宽 + 位置 -> popup 左上角 x。right corners 贴右边距,left 贴左边距。
pub(super) fn popup_x(screen_width: f64, position: PopupPosition, width: f64) -> f64 {
    match position {
        PopupPosition::TopRight | PopupPosition::BottomRight => screen_width - width - POPUP_MARGIN,
        PopupPosition::TopLeft | PopupPosition::BottomLeft => POPUP_MARGIN,
    }
}

/// 给定 stack 索引(0 = 最先弹出的那条)+ 位置 -> popup 左上角 y。
/// - top 角:第 0 条贴菜单栏下方,其后向下堆叠(y 递增)
/// - bottom 角:第 0 条贴屏幕底部边距,其后向上堆叠(y 递减)
pub(super) fn target_y(
    index: usize,
    position: PopupPosition,
    screen_height: f64,
    height: f64,
) -> f64 {
    let stack_offset = index as f64 * (height + POPUP_GAP);
    match position {
        PopupPosition::TopLeft | PopupPosition::TopRight => {
            // 30.0 留给 macOS 菜单栏
            POPUP_MARGIN + 30.0 + stack_offset
        }
        PopupPosition::BottomLeft | PopupPosition::BottomRight => {
            screen_height - height - POPUP_MARGIN - stack_offset
        }
    }
}

pub(super) fn read_popup_position(app: &AppHandle) -> PopupPosition {
    app.try_state::<Arc<Mutex<SettingsStore>>>()
        .and_then(|s| s.lock().ok().map(|g| g.settings.popup_position))
        .unwrap_or_default()
}

pub(super) fn read_popup_auto_close_secs(app: &AppHandle) -> Option<u32> {
    app.try_state::<Arc<Mutex<SettingsStore>>>()
        .and_then(|s| s.lock().ok().and_then(|g| g.settings.popup_auto_close_secs))
}

#[cfg(test)]
mod tests {
    use super::{popup_x, target_y, POPUP_HEIGHT, POPUP_WIDTH};
    use crate::settings::PopupPosition;

    const SCREEN_W: f64 = 1920.0;
    const SCREEN_H: f64 = 1080.0;

    #[test]
    fn target_y_top_starts_below_menu_bar() {
        assert_eq!(
            target_y(0, PopupPosition::TopRight, SCREEN_H, POPUP_HEIGHT),
            12.0 + 30.0
        );
        assert_eq!(
            target_y(0, PopupPosition::TopLeft, SCREEN_H, POPUP_HEIGHT),
            12.0 + 30.0
        );
    }

    #[test]
    fn target_y_top_stacks_downward() {
        let y0 = target_y(0, PopupPosition::TopRight, SCREEN_H, POPUP_HEIGHT);
        let y1 = target_y(1, PopupPosition::TopRight, SCREEN_H, POPUP_HEIGHT);
        assert_eq!(y1 - y0, 150.0 + 8.0);
        assert!(y1 > y0, "top stacks downward");
    }

    #[test]
    fn target_y_bottom_starts_at_bottom_margin() {
        // 第一条贴底:y = screen_h - HEIGHT - MARGIN
        assert_eq!(
            target_y(0, PopupPosition::BottomRight, SCREEN_H, POPUP_HEIGHT),
            SCREEN_H - 150.0 - 12.0
        );
    }

    #[test]
    fn target_y_bottom_stacks_upward() {
        let y0 = target_y(0, PopupPosition::BottomLeft, SCREEN_H, POPUP_HEIGHT);
        let y1 = target_y(1, PopupPosition::BottomLeft, SCREEN_H, POPUP_HEIGHT);
        assert_eq!(y0 - y1, 150.0 + 8.0);
        assert!(y1 < y0, "bottom stacks upward (y decreases)");
    }

    #[test]
    fn target_y_mini_uses_mini_height() {
        assert_eq!(
            target_y(0, PopupPosition::TopRight, SCREEN_H, 42.0),
            12.0 + 30.0
        );
        let y0 = target_y(0, PopupPosition::TopRight, SCREEN_H, 42.0);
        let y1 = target_y(1, PopupPosition::TopRight, SCREEN_H, 42.0);
        assert_eq!(y1 - y0, 42.0 + 8.0);
    }

    #[test]
    fn popup_x_right_corners_hug_right_edge() {
        let x_tr = popup_x(SCREEN_W, PopupPosition::TopRight, POPUP_WIDTH);
        let x_br = popup_x(SCREEN_W, PopupPosition::BottomRight, POPUP_WIDTH);
        assert_eq!(x_tr, SCREEN_W - 360.0 - 12.0);
        assert_eq!(x_tr, x_br);
    }

    #[test]
    fn popup_x_left_corners_hug_left_edge() {
        let x_tl = popup_x(SCREEN_W, PopupPosition::TopLeft, POPUP_WIDTH);
        let x_bl = popup_x(SCREEN_W, PopupPosition::BottomLeft, POPUP_WIDTH);
        assert_eq!(x_tl, 12.0);
        assert_eq!(x_tl, x_bl);
    }

    #[test]
    fn popup_x_mini_uses_mini_width() {
        let x = popup_x(SCREEN_W, PopupPosition::TopRight, 280.0);
        assert_eq!(x, SCREEN_W - 280.0 - 12.0);
    }
}
