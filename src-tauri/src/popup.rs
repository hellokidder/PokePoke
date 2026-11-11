use crate::sessions::Session;
use crate::settings::PopupSize;
use crate::surface::{log_resolution, resolve_surface, ResolvedSurface, SurfaceEvidence};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

mod sound;
#[cfg(target_os = "macos")]
use sound::run_on_main_timed;

mod geometry;
use geometry::{read_popup_position, read_popup_size, reserve_slot_and_position};
pub use geometry::{MonitorMetrics, PopupList};

mod animate;
use animate::animate_reposition;

mod window;
use window::{
    build_popup_window, emit_popup_shortcuts_updated, session_popup_url, update_popup_url,
};

mod timing;
use timing::{schedule_auto_close, schedule_focus_watcher};

/// popup 关闭原因 — 汇入 `close_popup_by_label` 的 INFO 日志,排障时不再只靠
/// 调用方各自的前置日志(部分路径如 reaper / remove_session 原先完全无声)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupCloseReason {
    /// ② 弹后召回:focus-watcher 探到用户已聚焦对应宿主。
    FocusWatcher,
    /// 用户配置的自动关闭时长到期。
    AutoCloseTimer,
    /// 新一轮 Running,`should_close_popup_on_running` 命中。
    StageEndRunning,
    /// hook SessionEnd 删 session。
    SessionEnd,
    /// 后台 reaper 连续探活失败清 session。
    Reaper,
    /// panel `remove_session` 或用户从列表删 session。
    RemoveSession,
    /// 接入断开时批量清某 integration 的 session。
    IntegrationDisconnect,
    /// 前端 popup 点 × 或快捷键跳转后关窗。
    FrontendDismiss,
    /// updater popup 点 ×(暂不升级)。
    UpdaterDismiss,
    /// updater popup「跳过本版」。
    UpdaterSkipped,
}

impl PopupCloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FocusWatcher => "focus_watcher",
            Self::AutoCloseTimer => "auto_close_timer",
            Self::StageEndRunning => "stage_end_running",
            Self::SessionEnd => "session_end",
            Self::Reaper => "reaper",
            Self::RemoveSession => "remove_session",
            Self::IntegrationDisconnect => "integration_disconnect",
            Self::FrontendDismiss => "frontend_dismiss",
            Self::UpdaterDismiss => "updater_dismiss",
            Self::UpdaterSkipped => "updater_skipped",
        }
    }
}

// Trait 抽象点（Layer 1 测试自动化新增）：把 popup 的"显示/关闭"和
// "终端是否已聚焦"抽成接口，让 http_server 的 integration 测试可以
// 注入 spy 实现。生产路径仍然走下面的 free function（show_popup /
// close_popup / is_terminal_session_focused），AppPopupController /
// AppleScriptFocusOracle 是它们的薄包装，行为零变化。
//
// 范围克制：只抽 http_server 在 handle_notify 里调到的能力。
// lib.rs 的 remove_session_with_cleanup、commands.rs 的 close_popup_window
// 仍然直接调 free function，不强制走 trait——避免无关模块被卷入。
pub trait PopupController: Send + Sync {
    /// 显示 popup。返回 `true` 表示**真的新创建**了 popup 窗口,`false` 表示
    /// 该 session 的 popup 已在显示中——后者用 store-emit 让前端 React 自动
    /// 更新内容,不重新创建窗口、调用方也不该再发提示音。
    /// 这是"popup 已在的同 session 后续事件不重复打扰"的核心:用户只听到
    /// 第一次声、看到第一次弹;后续 transition 在屏上的 popup 自动更新。
    fn show(&self, session: &Session) -> bool;
    fn close(&self, session_id: &str, reason: PopupCloseReason);
    fn has_session_popup(&self, session_id: &str) -> bool;
}

pub trait FocusOracle: Send + Sync {
    /// SurfaceEvidence 统一承载 tty / terminal_id / workspace fallback。
    /// 只有 resolver 判定该 surface 支持 focused 检测时才会调用终端聚焦检查。
    fn is_focused(&self, evidence: &SurfaceEvidence) -> bool {
        let resolved = resolve_surface(evidence);
        log_resolution("surface", "focused", evidence, &resolved);
        self.is_focused_resolved(evidence, &resolved)
    }

    /// Caller 已经 resolve/log 过 surface 时走这条,避免热路径重复 resolver + log。
    /// 测试 fake 只实现 `is_focused` 仍可保持简单；生产实现覆盖本方法。
    fn is_focused_resolved(&self, evidence: &SurfaceEvidence, _resolved: &ResolvedSurface) -> bool {
        self.is_focused(evidence)
    }
}

pub struct AppPopupController {
    app: AppHandle,
    popup_list: PopupList,
}

impl AppPopupController {
    pub fn new(app: AppHandle, popup_list: PopupList) -> Self {
        Self { app, popup_list }
    }
}

impl PopupController for AppPopupController {
    fn show(&self, session: &Session) -> bool {
        show_popup(&self.app, session, &self.popup_list)
    }

    fn close(&self, session_id: &str, reason: PopupCloseReason) {
        close_popup(&self.app, session_id, &self.popup_list, reason);
    }

    fn has_session_popup(&self, session_id: &str) -> bool {
        find_session_popup_label(&self.popup_list, session_id).is_some()
    }
}

pub struct AppleScriptFocusOracle;

impl FocusOracle for AppleScriptFocusOracle {
    fn is_focused_resolved(&self, evidence: &SurfaceEvidence, resolved: &ResolvedSurface) -> bool {
        if !resolved.capabilities.can_detect_focused {
            return false;
        }
        // 2026-07-06 撤回 425684d(M4)的 kind 分派:evaluate 侧同时传 tty +
        // terminal_id,跟 watcher 侧对齐。
        //
        // M4 原意:同一 session 曾在 Ghostty 抓过 terminal_id 后又切到
        // iTerm2/Terminal 拿到 tty,evidence 里两个都有,resolver 判 kind=
        // TerminalTty(tty 优先);M4 用 kind 只传 tty,期望在"前台是那个残留
        // Ghostty tab"时不走 Ghostty 分支 → 不抑制 → 弹出。
        //
        // 但 M4 制造了更常见的副作用:**用户就在 Ghostty pane 里跑 CC**,tty
        // 和 terminal_id 都是同一 pane 的真实 ID;resolver 判 kind=TerminalTty →
        // evaluate 拿 (tty, None) → Ghostty 前台走 Ghostty 分支 terminal_id=
        // None fallback false → 判 not focused → **弹**;然后 watcher(仍用两
        // 个 ID)0.5-1s 后探到 focused=true → 关。用户体感"聚焦抑制没生效,
        // popup 闪一下"(pokepoke.log 2026-07-06 铁证)。
        //
        // 撤回后:evaluate 也传 (tty, terminal_id),Ghostty 前台命中 Ghostty
        // 分支 id → focused=true → SUPPRESSED,根本不弹。
        //
        // M4 想修的 stale-id 场景另有兜底:reconcile 主动清不匹配的 terminal_id
        // (`terminal_id_captured_cwd` vs `workspace_path` 一致性),让 stale id
        // 不会留在 evidence 里。这条兜底路径已在 ghostty_reconcile.rs 存在。
        is_surface_focused(evidence)
    }
}

/// popup label 格式:`popup-{kind}-{token}-{payload}`,kind ∈ {s, u}。
///
/// 这种"每实例唯一 label"是 0.0.3 引入的反 race 设计 —— Tauri 官方文档
/// (`WebviewWindowBuilder` doc)明确建议 unique labels per instance,我们
/// 之前用 `popup-{session_id}` 同 session 复用 label 是反模式,二次 bug
/// (auto-close timer / focus-watcher 跨实例误关后继 popup)的根源都在此。
/// 改成唯一 label 后,popup_list 同时不可能有两条同 label,任何 watcher 抓
/// 到自己的 label 直接关都安全,不需要复杂的 token 校验做兜底。
///
/// ⚠️ **Tauri label 字符集限制**:`alphanumeric + - / : _`,**`.` 不允许**。
mod label;
// label 编解码 + popup_list 查询 helper 抽到 label.rs,这里重导出
// 保持外部 `crate::popup::session_popup_label` 等引用不断。
pub use label::*;

mod focus;
// 焦点探测(iTerm2/Terminal/Ghostty/cmux/herdr/WezTerm/Cursor)抽到 focus.rs,
// 这里重导出 mod.rs 内部 + 外部仍引用的入口。`is_surface_focused` 被
// `AppleScriptFocusOracle` / `schedule_focus_watcher` 用;`is_terminal_session_focused`
// 历史公开(`pub`)。
pub(crate) use focus::is_surface_focused;
pub use focus::is_terminal_session_focused;

/// 全局单调递增,给每个 popup 实例发独一无二的 token。`fetch_add(Relaxed)`
/// 是因为 token 之间没有 ordering 约束 —— 只用作 != 比较,从不被读两遍。
static POPUP_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn create_popup_list() -> PopupList {
    Arc::new(Mutex::new(Vec::new()))
}

pub fn show_popup(app: &AppHandle, session: &Session, popup_list: &PopupList) -> bool {
    // 同 session 的 popup 已经在屏上 → 直接返回 false,内容靠 store-emit 让
    // 前端 React 自动 re-render,不重新创建窗口、不发声音、不触发 cooldown
    // 消耗。0.0.3 unique-label 后,"是否已在屏"靠按 session_id payload 反查
    // 当前 label,而不是构造一个预期 label 看是否在 list(因为新 label 带 token,
    // 没创建前我们不知道下一个 token 会是几)。
    let already_visible = find_session_popup_label(popup_list, &session.id).is_some();
    if already_visible {
        crate::log_info!(
            "popup",
            "show_popup: refresh-only (popup already visible) task_id={} session_id={}",
            session.task_id,
            session.id,
        );
        return false;
    }

    // 0.0.3 unique-label:token **先**算,label 由 (token, session_id) 拼出来,
    // 每实例 label 唯一,根除"同 session 复用 label 引发跨实例误关"的 race。
    // 0.0.3 M2:reserve_slot_and_position 把 same_screen + visual_index + push
    // 合到一次 popup_list lock,避免并发 show_popup 算出错位 visual_index。
    let position = read_popup_position(app);
    let size = read_popup_size(app);
    let token = POPUP_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = session_popup_label(token, &session.id);
    let (x, y, metrics, visual_index) = reserve_slot_and_position(
        app,
        popup_list,
        position,
        size,
        label.clone(),
        token,
        Some(session),
    );

    // 多屏 popup 定位的诊断:把最终选中的 monitor (logical 矩形) + 算出的
    // (x, y) 都落 log,方便排查"popup 跑偏到错屏"问题。INFO 级——这是用户
    // 排错最关心的信号,默认日志就该看见。
    crate::log_info!(
        "popup",
        "show_popup: build x={:.0} y={:.0} monitor=({:.0},{:.0},{:.0}x{:.0}) tty={:?} workspace={:?} size={:?}",
        x,
        y,
        metrics.x,
        metrics.y,
        metrics.w,
        metrics.h,
        session.terminal_tty,
        session.workspace_path,
        size,
    );

    let url = session_popup_url(position, visual_index, size);

    // 2026-08-05 kimi 评审线路(brief 20260805-pending-idle-completed-sound-swallowed):
    // 播音从 popup 创建解耦,统一走 http_server::play_notify_sound(quiet / aborted /
    // SoundCooldown 节流都在那边;refresh-only 的 stage 变化也会响,修复"完成音被
    // 3s cooldown / 在屏 popup 连带吞掉")。这里只负责窗口创建,不再播音。

    // macOS:NSPanel 的创建与 AppKit 初始化必须在主线程,否则 AppKit assertion
    // 触发 abort。http_server handler 在 tokio worker 上,这里 dispatch 到主线程。
    // popup_list 已在上面 reserve_slot_and_position 内 push,这里只走构建 + 失败回滚。
    #[cfg(target_os = "macos")]
    {
        let app_for_build = app.clone();
        let popup_list_for_rollback = popup_list.clone();
        let label_for_build = label.clone();
        let size_for_build = size;
        run_on_main_timed(app, "show_popup.build", move || {
            let ok =
                build_popup_window(&app_for_build, &label_for_build, x, y, &url, size_for_build);
            if !ok {
                crate::log_warn!(
                    "popup",
                    "show_popup: build failed, rolling back popup_list label={}",
                    label_for_build,
                );
                if let Ok(mut list) = popup_list_for_rollback.lock() {
                    list.retain(|(l, _, t, _)| !(l == &label_for_build && *t == token));
                }
                // 回滚后让 listener 也知道(不然 shortcut 还以为有 1 个 popup 在屏)
                emit_popup_shortcuts_updated(&app_for_build, &popup_list_for_rollback);
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let build_ok = build_popup_window(app, &label, x, y, &url, size);
        if !build_ok {
            // 已在 reserve_slot_and_position 里 push 过,build 失败回滚
            crate::log_warn!(
                "popup",
                "show_popup: build failed, rolling back popup_list label={}",
                label,
            );
            if let Ok(mut list) = popup_list.lock() {
                list.retain(|(l, _, t, _)| !(l == &label && *t == token));
            }
            emit_popup_shortcuts_updated(app, popup_list);
        }
    }

    schedule_auto_close(app, popup_list, session, token, label.clone());

    schedule_focus_watcher(app, popup_list, session, token, label.clone());

    emit_popup_shortcuts_updated(app, popup_list);
    true
}

/// 弹"升级提示"popup,标题/内容/按钮全部在前端 React 端按 `kind=update` 渲染。
///
/// 与 [`show_popup`] 三处差异:
/// 1. label 走 [`update_popup_label`],跟 session popup 同前缀(`popup-`)但
///    `popup-update-` 前缀隔离,`session_popup_count` / `session_popup_at_visual_index`
///    跳过它,不参与 ⌘N 角标编号
/// 2. **不**起 auto-close timer(updater 提示不能被 5s/30s 吃掉)
/// 3. **不**起 focus-watcher(updater 跟终端无关)
///
/// 同 version 已在屏 → 直接返回 false(refresh-only,跟 session popup 同语义)。
pub fn show_update_popup(app: &AppHandle, version: &str, popup_list: &PopupList) -> bool {
    // 同 version 已在屏 → refresh-only。0.0.3 unique-label:按 version payload
    // 反查当前 label(token 不确定),find_update_popup_label 命中即说明已弹过。
    if find_update_popup_label(popup_list, version).is_some() {
        crate::log_info!(
            "popup",
            "show_update_popup: refresh-only (already visible) version={}",
            version,
        );
        return false;
    }

    // 0.0.3 M2:跟 show_popup 同模式走 reserve_slot_and_position 一锁原子化
    // session=None → terminal_monitor 不查 tty,直接 cursor/primary 兜底。
    // visual_index 算了但不用 —— update popup 不参与 ⌘N 角标(被 is_update_popup_label
    // 过滤),URL 也不带 n 参数。
    let position = read_popup_position(app);
    let size = PopupSize::Default; // update popup 始终用标准尺寸
    let token = POPUP_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = update_popup_label(token, version);
    let (x, y, metrics, _vis) =
        reserve_slot_and_position(app, popup_list, position, size, label.clone(), token, None);
    crate::log_info!(
        "popup",
        "show_update_popup: build version={} x={:.0} y={:.0} metrics=({:.0},{:.0},{:.0}x{:.0})",
        version,
        x,
        y,
        metrics.x,
        metrics.y,
        metrics.w,
        metrics.h,
    );

    let url = update_popup_url(position, version);

    #[cfg(target_os = "macos")]
    {
        let app_for_build = app.clone();
        let popup_list_for_rollback = popup_list.clone();
        let label_for_build = label.clone();
        let size_for_build = size;
        run_on_main_timed(app, "show_update_popup.build", move || {
            let ok =
                build_popup_window(&app_for_build, &label_for_build, x, y, &url, size_for_build);
            if !ok {
                crate::log_warn!(
                    "popup",
                    "show_update_popup: build failed, rolling back popup_list label={}",
                    label_for_build,
                );
                if let Ok(mut list) = popup_list_for_rollback.lock() {
                    list.retain(|(l, _, t, _)| !(l == &label_for_build && *t == token));
                }
                // 0.0.3 M1:对称地 emit。当前 `session_popup_shortcut_map` 主动 filter
                // 掉 update popup,所以 update popup 进 / 出 list 不会真正改变 shortcut
                // 注册集合;但保持 emit 对称性是为了让"任何 popup_list 变化 → emit"成
                // 不变量,未来 listener 端如果扩展为也消费 update popup(比如统一 visual
                // 排序),不会因为这条 path 漏 emit 而出 bug。
                emit_popup_shortcuts_updated(&app_for_build, &popup_list_for_rollback);
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let build_ok = build_popup_window(app, &label, x, y, &url, size);
        if !build_ok {
            crate::log_warn!(
                "popup",
                "show_update_popup: build failed, rolling back popup_list label={}",
                label,
            );
            if let Ok(mut list) = popup_list.lock() {
                list.retain(|(l, _, t, _)| !(l == &label && *t == token));
            }
            emit_popup_shortcuts_updated(app, popup_list);
        }
    }

    emit_popup_shortcuts_updated(app, popup_list);
    true
}

/// 关某 session 当前的 popup(id 是 session_id)。
///
/// 0.0.3 unique-label:label 现在带 token,session_id-only 无法直接构造目标 label,
/// 必须先 `find_session_popup_label` 反查当前实际 label 再走销毁链。若该 session
/// 此刻没有 popup(已被别处关掉) → 安静 no-op,语义跟以前 label-only 找不到时一致。
/// updater 提示 popup 走 [`close_popup_by_label`],因为它们没有对应 session。
pub fn close_popup(app: &AppHandle, id: &str, popup_list: &PopupList, reason: PopupCloseReason) {
    if let Some(label) = find_session_popup_label(popup_list, id) {
        close_popup_by_label(app, &label, popup_list, reason);
    }
}

/// 按 label 直接关 popup(支持 session 和 updater popup)。
pub fn close_popup_by_label(
    app: &AppHandle,
    label: &str,
    popup_list: &PopupList,
    reason: PopupCloseReason,
) {
    let label = label.to_string();
    // reason 汇入本条 INFO,避免只有 close_popup_by_label 却说不清是 ② / reaper /
    // 前端 dismiss 等哪条路径(2026-07-29 popup 闪一下排障教训)。
    let in_list = popup_list
        .lock()
        .map(|l| l.iter().any(|(lab, _, _, _)| lab == &label))
        .unwrap_or(false);
    crate::log_info!(
        "popup",
        "close_popup_by_label label={} in_list={} reason={}",
        label,
        in_list,
        reason.as_str(),
    );
    if let Some(metrics) = crate::runtime_metrics::global() {
        metrics.record_popup_close(reason.as_str());
    }

    // 找到被移除 popup 在它**所在屏**的子序列里的位置——后续 reposition
    // 只动同屏的、且只动它后面的(stack 收缩)。同时记下 metrics 给
    // animate_reposition 直接用,免得反查 monitor。
    let removed = if let Ok(mut list) = popup_list.lock() {
        let mut idx_within_screen = None;
        let mut metrics = None;
        let mut counter: std::collections::HashMap<(i64, i64), usize> =
            std::collections::HashMap::new();
        for (l, m, _, _) in list.iter() {
            let c = counter.entry(m.screen_key()).or_insert(0);
            if l == &label {
                idx_within_screen = Some(*c);
                metrics = Some(*m);
                break;
            }
            *c += 1;
        }
        list.retain(|(l, _, _, _)| l != &label);
        idx_within_screen.zip(metrics)
    } else {
        None
    };

    // macOS:panel 销毁链调 AppKit (panel.to_window() 内部 object_setClass +
    // win.destroy() 真正释放 NSWindow),**必须在主线程**。close_popup 历史上
    // 被多种后台线程直调过(tokio worker / focus-watcher / Tauri command
    // handler / auto-close timer);非主线程改 AppKit obj-c class 是 UB,
    // 表现为"popup_list 已 retain 干净,屏上 panel 却还在"——React 端靠 store-emit
    // 把内容刷成 Running 状态,看起来像 popup 永远不消失。把主线程 dispatch
    // 沉到这里,所有调用方自动安全。已在主线程时 run_on_main_thread 同步执行,
    // 不引入额外延迟;非主线程则排到下一个 AppKit tick(~ms 级),用户感知不到。
    //
    // 0.0.3 用 `win.close()`(软关闭),在某些时机被吞成 orderOut,导致 NSPanel
    // 视觉残留——**孤儿 popup**,鼠标 hover 显示菊花、快捷键编号错位。
    // 0.0.4 改为 `win.destroy()`(硬销毁),跳过 close-requested 软路径。
    // 仍走 `to_window()`,确保 ObjC class 已改回 NSWindow,destroy 时 Tauri 用
    // NSWindow API 操作不会撞 selector mismatch(v1 fix 跳过 to_window 直接
    // remove_panel + destroy 就是因为这撞了 NSException → SIGABRT,已 stash 弃用)。
    #[cfg(target_os = "macos")]
    {
        let app_for_destroy = app.clone();
        let label_for_destroy = label.clone();
        run_on_main_timed(app, "close_popup.destroy", move || {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app_for_destroy.get_webview_panel(&label_for_destroy) {
                if let Some(win) = panel.to_window() {
                    let _ = win.destroy();
                }
            } else if let Some(win) = app_for_destroy.get_webview_window(&label_for_destroy) {
                let _ = win.destroy();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.destroy();
        }
    }

    if let Some((removed_idx, metrics)) = removed {
        let app_clone = app.clone();
        let popup_list_clone = popup_list.clone();
        let position = read_popup_position(app);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            animate_reposition(
                &app_clone,
                &popup_list_clone,
                removed_idx,
                metrics,
                position,
            );
        });
    }

    // popup_list 已变,推 ⌘N 角标重排 + 全局快捷键 register/unregister 决策
    emit_popup_shortcuts_updated(app, popup_list);
}

#[cfg(test)]
mod tests {
    use super::{
        is_popup_instance_alive, is_update_popup_label, session_id_from_label, session_popup_label,
        update_popup_label,
    };
    use crate::settings::PopupSize;

    #[test]
    fn unique_labels_make_list_retain_safe() {
        // 0.0.3 unique-label 的核心新能力:就算 popup_list 里同时有同 session 的两个
        // popup 实例(label 因为 token 不同所以**互不相同**),按 label 关一个不影响
        // 另一个。这是消灭 0.0.2/0.0.3 反复出现"听到声音没 popup" race 的根本机制。
        // (生产路径里 already_visible 保证不会同时存在,但本测试模拟 watcher 还没
        // 退出 + 新 popup 已进来的极端时序窗口。)
        use super::{create_popup_list, MonitorMetrics};
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let session_id = "266e4fc0-3ced-4dc9-8f9e-f22a1c258045";
        let label_old = session_popup_label(100, session_id);
        let label_new = session_popup_label(101, session_id);

        assert_ne!(label_old, label_new, "同 session 不同 token 必须不同 label");

        // 极端时序:旧 popup 还没从 list 拿掉,新 popup 已进来(实际可能因为 popup_list
        // lock 释放后到 close_popup_by_label 排队期间发生)。
        pl.lock().unwrap_or_else(|p| p.into_inner()).push((
            label_old.clone(),
            m,
            100,
            PopupSize::Default,
        ));
        pl.lock().unwrap_or_else(|p| p.into_inner()).push((
            label_new.clone(),
            m,
            101,
            PopupSize::Default,
        ));

        // 旧 watcher 拿 label_old 查 → 找到自己,不会误命中新实例
        assert!(is_popup_instance_alive(&pl, &label_old, 100));
        assert!(is_popup_instance_alive(&pl, &label_new, 101));
        // 关键:拿 label_old 不可能匹配到 label_new(label 唯一)
        assert!(
            !is_popup_instance_alive(&pl, &label_old, 101),
            "stale watcher 拿旧 label 不能误识别为新实例"
        );

        // close_popup_by_label(label_old) 等价 retain(|l| l != label_old) → 新实例保留
        pl.lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(l, _, _, _)| l != &label_old);
        assert!(!is_popup_instance_alive(&pl, &label_old, 100));
        assert!(
            is_popup_instance_alive(&pl, &label_new, 101),
            "按旧 label 关旧实例,新实例必须无伤"
        );
    }

    #[test]
    fn find_session_popup_label_returns_current_or_none() {
        use super::{create_popup_list, find_session_popup_label, MonitorMetrics};
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };

        // 空 list → None
        assert_eq!(find_session_popup_label(&pl, "X"), None);

        // push 一个 → 反查命中
        let lbl = session_popup_label(7, "X");
        pl.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((lbl.clone(), m, 7, PopupSize::Default));
        assert_eq!(find_session_popup_label(&pl, "X"), Some(lbl));
        assert_eq!(find_session_popup_label(&pl, "Y"), None);

        // 别的 session 不命中
        pl.lock().unwrap_or_else(|p| p.into_inner()).push((
            session_popup_label(8, "Z"),
            m,
            8,
            PopupSize::Default,
        ));
        assert_eq!(find_session_popup_label(&pl, "Y"), None);
        assert!(find_session_popup_label(&pl, "Z").is_some());
    }

    #[test]
    fn find_update_popup_label_works() {
        use super::{create_popup_list, find_update_popup_label, MonitorMetrics};
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        assert_eq!(find_update_popup_label(&pl, "0.0.3"), None);
        let lbl = update_popup_label(42, "0.0.3");
        pl.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((lbl.clone(), m, 42, PopupSize::Default));
        assert_eq!(find_update_popup_label(&pl, "0.0.3"), Some(lbl));
        assert_eq!(find_update_popup_label(&pl, "0.0.4"), None);
    }

    #[test]
    fn is_popup_instance_alive_strict_label_token_match() {
        use super::{create_popup_list, is_popup_instance_alive, MonitorMetrics};
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let lbl = session_popup_label(100, "X");
        pl.lock().unwrap_or_else(|p| p.into_inner()).push((
            lbl.clone(),
            m,
            100,
            PopupSize::Default,
        ));

        assert!(is_popup_instance_alive(&pl, &lbl, 100));
        // 同 label 但 token 不对 → 不识别(理论上 unique-label 后 label 不会被复用,
        // 但 helper 仍带 token 校验作为 defense-in-depth)
        assert!(!is_popup_instance_alive(&pl, &lbl, 999));
        // 同 token 但 label 不对 → 不识别
        let other = session_popup_label(100, "Y");
        assert!(!is_popup_instance_alive(&pl, &other, 100));
    }

    #[test]
    fn visual_index_skips_update_popup_and_handles_overflow() {
        use super::{
            create_popup_list, session_popup_at_visual_index, session_popup_count,
            session_popup_shortcut_map, MonitorMetrics,
        };
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        // 模拟:session A → update → session B → session C
        {
            let mut g = pl.lock().unwrap_or_else(|p| p.into_inner());
            g.push((session_popup_label(1, "A"), m, 1, PopupSize::Default));
            g.push((update_popup_label(2, "0.0.3"), m, 2, PopupSize::Default));
            g.push((session_popup_label(3, "B"), m, 3, PopupSize::Default));
            g.push((session_popup_label(4, "C"), m, 4, PopupSize::Default));
        }
        assert_eq!(
            session_popup_count(&pl),
            3,
            "update popup 不计入 session count"
        );
        assert_eq!(session_popup_at_visual_index(&pl, 0), Some("A".to_string()));
        assert_eq!(session_popup_at_visual_index(&pl, 1), Some("B".to_string()));
        assert_eq!(session_popup_at_visual_index(&pl, 2), Some("C".to_string()));
        assert_eq!(session_popup_at_visual_index(&pl, 3), None, "越界返回 None");

        let shortcuts = session_popup_shortcut_map(&pl);
        assert_eq!(shortcuts.len(), 3);
        assert_eq!(shortcuts[0], (session_popup_label(1, "A"), 0));
        assert_eq!(shortcuts[1], (session_popup_label(3, "B"), 1));
        assert_eq!(shortcuts[2], (session_popup_label(4, "C"), 2));
    }

    #[test]
    fn label_helpers_round_trip_and_classify() {
        let sess = session_popup_label(7, "abc-123-456");
        assert_eq!(sess, "popup-s-7-abc-123-456");
        assert!(!is_update_popup_label(&sess));
        assert_eq!(
            session_id_from_label(&sess),
            Some("abc-123-456"),
            "payload 里的 `-` 要保留(UUID 兼容)"
        );

        let upd = update_popup_label(8, "0.0.3");
        // 关键回归断言:`.` 编码为 `_`,Tauri label 字符集只允许 alphanumeric + `-`/`_`/`:`/`/`。
        // semver 必带 `.`,不编码会让 WebviewWindowBuilder.build() reject 整个 popup
        // 创建(实测 18:23 抓到 `Window labels must only include alphanumeric ...` 错)。
        assert_eq!(upd, "popup-u-8-0_0_3");
        assert!(is_update_popup_label(&upd));
        // update popup 不能反解出 session_id
        assert_eq!(session_id_from_label(&upd), None);

        // parse_popup_label 完整解析
        let parsed = super::parse_popup_label(&sess).unwrap();
        assert_eq!(parsed.kind, super::PopupKind::Session);
        assert_eq!(parsed.token, 7);
        assert_eq!(parsed.payload, "abc-123-456");

        let parsed_u = super::parse_popup_label(&upd).unwrap();
        assert_eq!(parsed_u.kind, super::PopupKind::Update);
        assert_eq!(parsed_u.token, 8);
        assert_eq!(parsed_u.payload, "0_0_3");

        // 非法格式
        assert!(super::parse_popup_label("popup-").is_none());
        assert!(
            super::parse_popup_label("popup-x-1-foo").is_none(),
            "未知 kind"
        );
        assert!(
            super::parse_popup_label("popup-s-notanum-foo").is_none(),
            "token 非数字"
        );
        assert!(super::parse_popup_label("not-a-popup").is_none());
    }

    #[test]
    fn update_popup_label_is_tauri_safe_for_semver() {
        // 18:23 真实 bug 复现:`popup-u-0-9.9.9` 因 `.` 被 Tauri reject。
        // 这个测试钉住"version 含 `.` / `+` 必须编码"的契约,防止有人改回 raw 形态。
        let l1 = update_popup_label(0, "9.9.9");
        let l2 = update_popup_label(1, "1.0.0-rc.1");
        let l3 = update_popup_label(2, "1.0.0+build.42");
        let ok = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | ':'))
        };
        assert!(ok(&l1), "label {} 含非法字符", l1);
        assert!(ok(&l2), "label {} 含非法字符", l2);
        assert!(ok(&l3), "label {} 含非法字符", l3);
        // find_update_popup_label 必须对入参 version 做同样编码,否则相同 version 找不到
        use super::{create_popup_list, find_update_popup_label, MonitorMetrics};
        let pl = create_popup_list();
        let m = MonitorMetrics {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        pl.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((l1.clone(), m, 0, PopupSize::Default));
        assert_eq!(
            find_update_popup_label(&pl, "9.9.9"),
            Some(l1),
            "find_update_popup_label 必须对 version 做对称编码,raw `.` 应能命中编码后的 label"
        );
    }
}
