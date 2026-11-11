//! popup 重排动画:关闭一个 popup 后平滑滑动剩余 popup 到新位置。
//!
//! 从 `popup.rs` 抽出:`animate_reposition`。依赖 geometry 的尺寸/角点计算。
//! 行为零变化;`popup.rs` 引用保持调用点(close_popup_by_label)不断。

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::settings::PopupPosition;

use super::geometry::{popup_dimensions, popup_x, target_y, MonitorMetrics, PopupList, POPUP_GAP};

/// Smoothly animate remaining popups to their new positions after one is removed.
/// stack 方向决定 slide_delta 符号:top 堆叠时被移除项后面的兄弟“从下方滑入新位置”
/// (start_y > final_y);bottom 堆叠时反过来“从上方滑入”(start_y < final_y)。
pub(super) fn animate_reposition(
    app: &AppHandle,
    popup_list: &PopupList,
    removed_idx_within_screen: usize,
    metrics: MonitorMetrics,
    position: PopupPosition,
) {
    // 用创建时记录下来的 PopupSize,不要重新读 settings--否则用户中途改了
    // popup_size 会导致 slide_delta 算错(旧 popup 的 pitch 跟新 size 不一致)。
    let size = {
        let list = popup_list.lock().unwrap_or_else(|p| p.into_inner());
        list.iter()
            .find(|(_, m, _, _)| *m == metrics)
            .map(|(_, _, _, s)| *s)
            .unwrap_or_default()
    };
    let (popup_w, popup_h) = popup_dimensions(size);
    // 只取同屏的 popup,按它们在该屏 list 里的顺序 reposition。
    // 跨屏的 popup 不动 - 它们的 stack 跟这次 close 无关。
    let same_screen: Vec<String> = if let Ok(list) = popup_list.lock() {
        list.iter()
            .filter(|(_, m, _, _)| *m == metrics)
            .map(|(l, _, _, _)| l.clone())
            .collect()
    } else {
        return;
    };

    // 直接用被关闭 popup 所在屏的 metrics,不再反查 cursor / primary。
    let (mon_x, mon_y, mon_w, mon_h) = (metrics.x, metrics.y, metrics.w, metrics.h);
    let x = mon_x + popup_x(mon_w, position, popup_w);

    let stack_direction: f64 = match position {
        PopupPosition::TopLeft | PopupPosition::TopRight => 1.0,
        PopupPosition::BottomLeft | PopupPosition::BottomRight => -1.0,
    };
    let slide_delta = (popup_h + POPUP_GAP) * stack_direction;
    let steps = 14;
    let frame_ms = 16; // ~60fps

    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        // ease-out cubic for natural deceleration
        let eased = 1.0 - (1.0 - t).powi(3);

        for (i, label) in same_screen
            .iter()
            .enumerate()
            .skip(removed_idx_within_screen)
        {
            let final_y = mon_y + target_y(i, position, mon_h, popup_h);
            let start_y = final_y + slide_delta;
            let y = start_y + (final_y - start_y) * eased;

            if let Some(win) = app.get_webview_window(label) {
                let _ =
                    win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
            }
        }

        std::thread::sleep(Duration::from_millis(frame_ms));
    }
}
