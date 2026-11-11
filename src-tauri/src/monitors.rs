//! 多屏定位的共享 helper。
//!
//! macOS 多屏环境下,Tauri/Tao 的 `LogicalPosition` 原点 = primary monitor
//! 左上角,副屏带正/负 logical 偏移。要把窗口定位到正确的屏,必须找到
//! 目标 monitor 的物理 origin / size,然后除以 scale_factor 得到 logical
//! 偏移,加到 in-monitor 位置上。
//!
//! 三个使用场景:
//! - **Session panel** (tray click 触发):用 `pick_monitor_at_point`
//!   按点击物理坐标找屏(`tray.rs::compute_panel_position`)。
//! - **Settings window** (用户主动打开):用 `cursor_monitor` 找鼠标
//!   当前所在屏,在该屏中央打开。
//! - **Popup** (hook event 自动触发,无 click):用 `cursor_monitor` 找
//!   鼠标当前屏,在该屏对应角弹出。

use crate::surface::{resolve_surface, SurfaceEvidence, SurfaceKind};
use tauri::{AppHandle, Monitor};

/// 鼠标当前所在的屏。读不到 cursor / 找不到包含屏 → fallback primary。
/// 适合无 click 上下文的窗口(popup / settings)。
pub fn cursor_monitor(app: &AppHandle) -> Option<Monitor> {
    let cursor = app.cursor_position().ok()?;
    pick_monitor_at_point(app, cursor.x, cursor.y).or_else(|| app.primary_monitor().ok().flatten())
}

pub fn surface_monitor(app: &AppHandle, evidence: &SurfaceEvidence) -> Option<Monitor> {
    let resolved = resolve_surface(evidence);
    crate::log_debug!(
        "surface",
        "screen resolve session={} kind={:?} confidence={:?} reason={}",
        evidence.session_id,
        resolved.kind,
        resolved.confidence,
        resolved.reason,
    );

    match resolved.kind {
        SurfaceKind::TerminalTty | SurfaceKind::GhosttyTerminal => terminal_monitor(
            app,
            evidence.terminal_tty.as_deref(),
            evidence.terminal_id.as_deref(),
        ),
        SurfaceKind::CodexDesktop
        | SurfaceKind::DshSession
        | SurfaceKind::CursorWorkspace
        | SurfaceKind::FinderWorkspace
        | SurfaceKind::OttyTerminal
        | SurfaceKind::OrcaTerminal
        // herdr session 无 tty/terminal_id,无法定位屏幕,popup fallback primary。
        | SurfaceKind::HerdrPane
        | SurfaceKind::Unknown => None,
    }
}

/// 触发 hook 事件的终端窗口所在屏。用 AppleScript 查窗口 bounds(logical
/// 坐标),取窗口中心点 → pick_monitor_at_logical_point。
///
/// 三层尝试:
/// 1. **iTerm2** 按 tty 查 session
/// 2. **Terminal.app** 按 tty 查 tab
/// 3. **Ghostty** 按 terminal_id 查 terminal(0.0.5+,任一 tty/id 缺一即跳过)
///
/// 全部失败(无 tty 也无 id / agent 不在三端 / Cursor GUI 等)返回 None,
/// popup 自己 fallback 到 cursor_monitor / primary。
///
/// 注意:每次 popup show 都跑一次 osascript ≈ 50–150ms。在 popup 显示流程
/// 里加这点延迟可接受,但不要在热路径反复调。
pub fn terminal_monitor(
    app: &AppHandle,
    tty: Option<&str>,
    terminal_id: Option<&str>,
) -> Option<Monitor> {
    let tty = tty.filter(|t| !t.is_empty());
    let terminal_id = terminal_id.filter(|t| !t.is_empty());
    let attempts = terminal_bounds_attempts(tty, terminal_id);
    let bounds = tty
        .and_then(|t| iterm2_window_bounds(t).or_else(|| terminal_app_window_bounds(t)))
        .or_else(|| terminal_id.and_then(ghostty_window_bounds));
    let bounds = match bounds {
        Some(b) => b,
        None => {
            crate::log_info!(
                "popup",
                "terminal_monitor: AppleScript bounds query failed tty={:?} terminal_id={:?} attempts={} fallback=cursor",
                tty,
                terminal_id,
                attempts.join("->"),
            );
            return None;
        }
    };
    let (x1, y1, x2, y2) = bounds;
    let cx = (x1 + x2) / 2.0;
    let cy = (y1 + y2) / 2.0;

    // 详细诊断:把 AppleScript 返回的窗口 bounds、计算出的中心点、所有
    // 可用 monitor 的 logical 矩形都打出来,便于定位"popup 跑偏到主屏"问题。
    if let Ok(monitors) = app.available_monitors() {
        let summary: Vec<String> = monitors
            .iter()
            .map(|m| {
                let scale = m.scale_factor();
                let p = m.position();
                let s = m.size();
                format!(
                    "[{:.0},{:.0} {:.0}x{:.0} @{:.1}x]",
                    p.x as f64 / scale,
                    p.y as f64 / scale,
                    s.width as f64 / scale,
                    s.height as f64 / scale,
                    scale
                )
            })
            .collect();
        crate::log_info!(
            "popup",
            "terminal_monitor: tty={:?} terminal_id={:?} bounds=({:.0},{:.0},{:.0},{:.0}) center=({:.0},{:.0}) monitors={}",
            tty,
            terminal_id,
            x1,
            y1,
            x2,
            y2,
            cx,
            cy,
            summary.join(" "),
        );
    }

    pick_monitor_at_logical_point(app, cx, cy)
}

fn iterm2_window_bounds(tty: &str) -> Option<(f64, f64, f64, f64)> {
    let script = format!(
        r#"tell application "iTerm2"
    repeat with w in windows
        repeat with t in tabs of w
            repeat with s in sessions of t
                if tty of s is "{tty}" then
                    set wb to bounds of w
                    return (item 1 of wb as text) & "," & (item 2 of wb as text) & "," & (item 3 of wb as text) & "," & (item 4 of wb as text)
                end if
            end repeat
        end repeat
    end repeat
    return ""
end tell"#,
        tty = tty
    );
    parse_bounds(&run_applescript_text(&script))
}

fn terminal_app_window_bounds(tty: &str) -> Option<(f64, f64, f64, f64)> {
    let script = format!(
        r#"tell application "Terminal"
    repeat with w in windows
        repeat with t in tabs of w
            if tty of t is "{tty}" then
                set wb to bounds of w
                return (item 1 of wb as text) & "," & (item 2 of wb as text) & "," & (item 3 of wb as text) & "," & (item 4 of wb as text)
            end if
        end repeat
    end repeat
    return ""
end tell"#,
        tty = tty
    );
    parse_bounds(&run_applescript_text(&script))
}

/// Ghostty 1.3+ AppleScript dictionary 路径:
///   `application` → `window` → `tab` → `terminal`(每个 terminal 有 stable `id`)
/// 遍历 windows × tabs × terminals 找到 `id == terminal_id` 的那个,返回所在
/// window 的 bounds(logical 坐标四元组)。
///
/// 性能:用户一般 windows × tabs 量级 ~10,osascript 单次 ~80ms 可接受。
/// 如果用户 tab 极多导致明显卡,可以下沉 try 块跳过空 windows,但当前不优化。
fn terminal_bounds_attempts(tty: Option<&str>, terminal_id: Option<&str>) -> Vec<&'static str> {
    let mut attempts = Vec::new();
    if tty.is_some_and(|t| !t.is_empty()) {
        attempts.push("iTerm2");
        attempts.push("Terminal");
    }
    if terminal_id.is_some_and(|t| !t.is_empty()) {
        attempts.push("Ghostty");
    }
    if attempts.is_empty() {
        attempts.push("none");
    }
    attempts
}

fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_position_size_as_bounds(s: &str) -> Option<(f64, f64, f64, f64)> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<f64> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [x, y, w, h] if *w > 0.0 && *h > 0.0 => Some((*x, *y, *x + *w, *y + *h)),
        _ => None,
    }
}

fn ghostty_window_bounds_script(terminal_id: &str) -> String {
    let terminal_id = applescript_escape(terminal_id);
    format!(
        r#"set targetWindowIndex to 0

tell application "Ghostty"
    try
        set windowIndex to 0
        repeat with w in windows
            set windowIndex to windowIndex + 1
            repeat with tb in tabs of w
                repeat with term in terminals of tb
                    if id of term is "{terminal_id}" then
                        set targetWindowIndex to windowIndex
                        exit repeat
                    end if
                end repeat
                if targetWindowIndex is not 0 then exit repeat
            end repeat
            if targetWindowIndex is not 0 then exit repeat
        end repeat
    on error
        return ""
    end try
end tell

if targetWindowIndex is 0 then return ""

tell application "System Events"
    try
        tell process "Ghostty"
            set p to position of window targetWindowIndex
            set s to size of window targetWindowIndex
            return (item 1 of p as text) & "," & (item 2 of p as text) & "," & (item 1 of s as text) & "," & (item 2 of s as text)
        end tell
    on error
        return ""
    end try
end tell"#,
        terminal_id = terminal_id
    )
}

fn ghostty_window_bounds(terminal_id: &str) -> Option<(f64, f64, f64, f64)> {
    let script = ghostty_window_bounds_script(terminal_id);
    parse_position_size_as_bounds(&run_applescript_text(&script))
}

fn run_applescript_text(script: &str) -> String {
    // 带超时:Ghostty 卡住时不让窗口 bounds 查询无限阻塞(见 osascript 模块)。
    crate::osascript::run(
        script,
        crate::osascript::DEFAULT_TIMEOUT,
        "monitors",
        "window_bounds",
    )
    .unwrap_or_default()
}

fn parse_bounds(s: &str) -> Option<(f64, f64, f64, f64)> {
    if s.is_empty() {
        return None;
    }
    let parts: Vec<f64> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [x1, y1, x2, y2] => Some((*x1, *y1, *x2, *y2)),
        _ => None,
    }
}

/// 用 logical 坐标 (AppleScript 返回的窗口 bounds 是 logical 像素) 找包含该
/// 点的 monitor。把每个 monitor 的物理 (position, size) 各自除自己的
/// scale_factor 转成 logical bounds,再判定包含。
fn pick_monitor_at_logical_point(app: &AppHandle, x_log: f64, y_log: f64) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    monitors.into_iter().find(|m| {
        let scale = m.scale_factor();
        let pos = m.position();
        let size = m.size();
        let mx = pos.x as f64 / scale;
        let my = pos.y as f64 / scale;
        let mw = size.width as f64 / scale;
        let mh = size.height as f64 / scale;
        x_log >= mx && x_log < mx + mw && y_log >= my && y_log < my + mh
    })
}

/// 用物理坐标遍历所有 monitor 找包含该点的屏。
/// `monitor.position()` / `.size()` 都是物理像素,直接区间判定不用除 scale。
pub fn pick_monitor_at_point(app: &AppHandle, x_phys: f64, y_phys: f64) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    monitors.into_iter().find(|m| {
        let pos = m.position();
        let size = m.size();
        let mx = pos.x as f64;
        let my = pos.y as f64;
        let mw = size.width as f64;
        let mh = size.height as f64;
        x_phys >= mx && x_phys < mx + mw && y_phys >= my && y_phys < my + mh
    })
}

/// 返回 monitor 的 (origin_x_logical, origin_y_logical, width_logical,
/// height_logical),把物理像素都除以 scale_factor。窗口 set_position 走
/// LogicalPosition 时,把 origin 加到 in-monitor 偏移上得到全局 logical 坐标。
pub fn monitor_logical_metrics(monitor: &Monitor) -> (f64, f64, f64, f64) {
    let scale = monitor.scale_factor();
    let pos = monitor.position();
    let size = monitor.size();
    (
        pos.x as f64 / scale,
        pos.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_bounds_attempts_include_ghostty_when_tty_and_terminal_id_present() {
        assert_eq!(
            terminal_bounds_attempts(Some("/dev/ttys001"), Some("ghost-id-1")),
            vec!["iTerm2", "Terminal", "Ghostty"],
            "tty + terminal_id 同时存在时,诊断日志必须体现三端都试过；否则会误判为 Ghostty 没参与定位"
        );
    }

    #[test]
    fn terminal_bounds_attempts_reports_no_identifier_when_empty() {
        assert_eq!(
            terminal_bounds_attempts(Some(""), None),
            vec!["none"],
            "tty / terminal_id 都为空时,失败日志应直接说明没有可定位的终端标识"
        );
    }

    #[test]
    fn parse_position_size_converts_to_bounds() {
        assert_eq!(
            parse_position_size_as_bounds("100,200,800,600"),
            Some((100.0, 200.0, 900.0, 800.0))
        );
    }

    #[test]
    fn parse_position_size_rejects_empty_or_incomplete_output() {
        assert_eq!(parse_position_size_as_bounds(""), None);
        assert_eq!(parse_position_size_as_bounds("100,200,800"), None);
    }

    #[test]
    fn parse_position_size_rejects_non_positive_size() {
        assert_eq!(parse_position_size_as_bounds("100,200,0,600"), None);
        assert_eq!(parse_position_size_as_bounds("100,200,800,-1"), None);
    }

    #[test]
    fn applescript_escape_handles_quotes_and_backslashes() {
        assert_eq!(
            applescript_escape(r#"id-"with"\slash"#),
            r#"id-\"with\"\\slash"#
        );
    }

    #[test]
    fn ghostty_bounds_script_uses_system_events_not_ghostty_bounds() {
        let script = ghostty_window_bounds_script("abc-123");

        assert!(script.contains("tell application \"Ghostty\""));
        assert!(script.contains("tell application \"System Events\""));
        assert!(script.contains("process \"Ghostty\""));
        assert!(script.contains("position of window"));
        assert!(script.contains("size of window"));
        assert!(
            !script.contains("bounds of w"),
            "Ghostty dictionary bounds are known broken and must not be used"
        );
    }

    #[test]
    fn ghostty_bounds_script_escapes_terminal_id() {
        let script = ghostty_window_bounds_script(r#"id-"quote"\slash"#);

        assert!(script.contains(r#"id-\"quote\"\\slash"#));
    }
}
