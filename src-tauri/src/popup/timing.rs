//! popup 时序:auto-close timer + focus-watcher + 宽限期决策。
//!
//! 从 `popup.rs` 抽出:`schedule_auto_close`(N 秒后按 label 关)+
//! `schedule_focus_watcher`(后台轮询聚焦态自动收)+ `should_close_on_focus_probe`
//! (宽限期决策,pure fn)+ `FOCUS_WATCHER_GRACE`/`FOCUS_WATCHER_INTERVAL` 常量 +
//! `terminal_id_log_hint`(日志 helper)。行为零变化;`popup.rs` 引用保持
//! show_popup 调用点不断。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::sessions::Session;
use crate::settings::SettingsStore;
use crate::surface::{log_resolution, resolve_surface, SurfaceEvidence, SurfaceKind};

use super::close_popup_by_label;
use super::geometry::{read_popup_auto_close_secs, PopupList};
use super::is_popup_instance_alive;
use super::is_surface_focused;
use super::PopupCloseReason;

fn terminal_id_log_hint(terminal_id: Option<&str>) -> String {
    let Some(id) = terminal_id.filter(|id| !id.is_empty()) else {
        return "absent".to_string();
    };
    let preview: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        format!("{}…", preview)
    } else {
        preview
    }
}

/// 用户配置的“显示 N 秒后自动关闭”:起一个后台线程 sleep 完按 label 关。
/// 0.0.3 unique-label 后 timer 抓本实例 label(唯一);`is_popup_instance_alive`
/// 仍带 token 校验作 defense-in-depth。值在 show_popup 时刻 snapshot,后续改设置
/// 不影响已在屏的 popup。
pub(super) fn schedule_auto_close(
    app: &AppHandle,
    popup_list: &PopupList,
    session: &Session,
    token: u64,
    label: String,
) {
    // 用户配置的“显示 N 秒后自动关闭”。值在 show_popup 时刻 snapshot -- 用户
    // 之后改设置不影响已在屏的 popup,与 popup_position 行为一致。
    //
    // 0.0.3 unique-label 后,timer 抓本实例的 label(唯一),sleep 完直接按
    // label 关。`is_popup_instance_alive` 仍带 token 校验是 defense-in-depth,
    // 万一未来 label 生成逻辑变回复用(回归),token 校验仍能挡住跨实例误关。
    if let Some(secs) = read_popup_auto_close_secs(app) {
        if secs > 0 {
            crate::log_info!(
                "popup",
                "auto-close timer scheduled task_id={} token={} secs={}",
                session.task_id,
                token,
                secs,
            );
            let app_clone = app.clone();
            let popup_list_clone = popup_list.clone();
            let task_id = session.task_id.clone();
            let label_for_timer = label.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs as u64));
                let same_instance_still_exists =
                    is_popup_instance_alive(&popup_list_clone, &label_for_timer, token);
                if !same_instance_still_exists {
                    crate::log_info!(
                        "popup",
                        "auto-close timer skip (popup already gone) task_id={} token={}",
                        task_id,
                        token,
                    );
                    return;
                }
                crate::log_info!(
                    "popup",
                    "auto-close timer firing task_id={} token={} secs={}",
                    task_id,
                    token,
                    secs,
                );
                // 按 label 直接关 -- label 唯一标识本实例,即使在销毁 dispatch
                // 排队期间有别的事发生,也不会误关后继实例。
                close_popup_by_label(
                    &app_clone,
                    &label_for_timer,
                    &popup_list_clone,
                    PopupCloseReason::AutoCloseTimer,
                );
            });
        }
    }
}

/// Watcher 探到 focused=true 后是否允许关掉 popup。这是“闪一下”bug 的核心
/// 修法(2026-07-06):在 popup 弹出后 [`FOCUS_WATCHER_GRACE`] 时间内即使
/// focused=true 也不关。这样:
///
/// - 用户报告场景:evaluate 判 not focused -> 弹 -> 用户/系统在几百 ms 内切到
///   目标终端 -> watcher 500ms 一探判 focused -> 若在 grace 内则不关,给用户
///   最起码“看清 popup 内容”的时间。
/// - 用户手动切焦点 close 的原意保留:grace 外的 focused=true 命中仍会 close,
///   跟 08 scenario A 一致(用户主动切走 -> 收 popup)。
///
/// 拆成 pure fn 让 unit test 覆盖决策,不依赖真实 osascript / 后台线程。
fn should_close_on_focus_probe(shown_at: Instant, now: Instant, grace: Duration) -> bool {
    now.duration_since(shown_at) >= grace
}

/// popup show 后 watcher 忽略 focused 探测的宽限期。太短(< 1s)挡不住 evaluate
/// / watcher 竞态引发的“闪一下”;太长(> 5s)会明显延迟“用户手动切回终端后
/// popup 被收走”的响应。3s 兼顾两侧,也跟 08 scenario 里 sleep 3s(手动 focus
/// 后等 watcher 收)的历史假设一致。
const FOCUS_WATCHER_GRACE: Duration = Duration::from_secs(3);

/// focus-watcher 轮询间隔。每 tick 会 spawn 1~9 个子进程(见 is_surface_focused
/// 分支表);多 popup + herdr 用户 shell-out 压力线性放大。1s 相对 500ms 感官上无差
/// (FOCUS_WATCHER_GRACE=3s 兜住短抖),但每秒子进程数直接减半。
const FOCUS_WATCHER_INTERVAL: Duration = Duration::from_millis(1000);

/// otty 哨兵:连续未聚焦多少次后做一次差分诊断(对比 bound tab 与 front window
/// 实际 selected tab)。
///
/// 为什么有它:2026-08-10 事故 —— `selected_tab_is` 的 window_id 分支拿 CLI
/// UUID 匹配 AppleScript 数字 id,恒 false,Otty 抑制/召回整链静默失效 4 天
/// (9590 次 ok=false 无人发现)。哨兵把"连续 false"里真正异常的那一种(用户其实
/// 就在 bound tab)识别出来打 WARN,零误报(差分拿真实值对比)。
///
/// 10 次 ≈ 10s+osascript 耗时;阈值小没事,因为诊断每 popup 至多一次,且
/// 误报率 ≈ 0(只有 bound == actual 才 WARN)。
const FOCUS_WATCHER_MISMATCH_PROBE_AFTER: u32 = 10;

/// 哨兵触发判定(纯函数,可测):连续未聚焦达阈值 + surface 是 Otty + 本
/// popup 尚未诊断过。
fn should_probe_focus_mismatch(
    consecutive_unfocused: u32,
    threshold: u32,
    is_otty_surface: bool,
    already_probed: bool,
) -> bool {
    !already_probed && is_otty_surface && consecutive_unfocused >= threshold
}

/// otty 差分诊断:bound tab vs front window 实际 selected tab。
/// 相等 = 判定契约损坏(WARN,冻结签名 `otty focus-check mismatch`);
/// 不等 = 用户确实在别的 tab(INFO,记录实际 tab 便于排障,冻结签名
/// `otty focus-check diff`)。信息源拿不到则静默(CLI/osascript 失败属既有噪音)。
///
/// ⚠️ 前台 gate(2026-08-11 修复):诊断前必须确认**系统前台是 Otty**。
/// `is_surface_focused` 的 otty 分支在系统前台不是 Otty 时直接 false(用户
/// 没在看 Otty,抑制语义正确,不打判定日志);而 `front_selected_tab_id` 拿的
/// 是 Otty 窗口栈 front window 的 selected tab,与系统前台无关——用户在别的
/// app 工作时,窗口栈最前的 tab 恰为 bound tab 就会误报 WARN。只有前台是
/// Otty 且 selected_tab_is 连续 false 才可能意味着契约损坏。
fn probe_otty_focus_mismatch(evidence: &SurfaceEvidence) {
    if crate::osascript::frontmost_app_name().as_deref() != Some("Otty") {
        crate::log_debug!(
            "focus",
            "otty focus-check sentinel skipped: frontmost is not Otty (false 属正常抑制)"
        );
        return;
    }
    let Some(pane) = evidence.otty_pane_id.as_deref() else {
        return;
    };
    match crate::commands::focus::diagnose_focus_mismatch(
        pane,
        evidence.otty_tab_id.as_deref(),
    ) {
        Some((bound, actual)) if bound == actual => crate::log_warn!(
            "focus",
            "otty focus-check mismatch: 连续 {} 次判定 false,但 front window selected tab 恰为 bound tab(tab={})—— 判定契约疑似损坏,抑制/召回可能失效",
            FOCUS_WATCHER_MISMATCH_PROBE_AFTER,
            bound,
        ),
        Some((bound, actual)) => crate::log_info!(
            "focus",
            "otty focus-check diff: 连续 {} 次 false 属正常(实际 selected tab={}, bound={})",
            FOCUS_WATCHER_MISMATCH_PROBE_AFTER,
            actual,
            bound,
        ),
        None => {}
    }
}

/// Auto-dismiss:用户聚焦对应终端 session 时关掉 popup。后台线程 500ms 轮询
/// `is_terminal_session_focused`,命中或本实例已不在 list 时退出。
/// 0.0.3 unique-label + 捕获 my_label:watcher 醒来时 `is_popup_instance_alive`
/// 直接 false -> 退出,不会动到同 session 的后继 popup(0.0.2/0.0.3 “听到声音
/// 没 popup”在此根除)。0.0.8+:是否启动 watcher 由 SurfaceEvidence resolver 的
/// capability 决定;tty/terminal_id 精确 surface 可检测聚焦,workspace fallback 不启动。
///
/// 0.0.9(2026-07-05):watcher 现在也关联 `focus_suppression_enabled`。
/// 之前 watcher 无视该开关,导致这个 UX 冲突:
///   focus_suppression_enabled=false(用户语义:“聚焦时也要弹我”)
///     + Stop 到达时 iTerm2 已在前台
///     -> evaluate_popup_presentation 不预判抑制 -> 弹出 ✓
///     -> 但 watcher 500ms 后 poll focused=true -> close ❌ 用户体感 popup“闪一下”
/// 修法:watcher 读同一开关,关掉聚焦抑制时不起 watcher。
///
/// 0.0.10(2026-07-06):加 [`FOCUS_WATCHER_GRACE`] 宽限期。上一版只覆盖了
/// focus_suppression=false 的场景,=true 时依然存在“闪一下”-- evaluate 判
/// not focused(比如前台不是终端 tab、tty 不匹配、Ghostty session 里 kind
/// 分派后 tty 单独判失败)-> 弹出;几百 ms 后前台变成目标终端 -> watcher 探到
/// focused=true -> 秒关。用户 pokepoke.log 里 flap 存活时长 1398ms 铁证。
/// 修法:popup show 后 3s 内 watcher 探到 focused=true 也不关,把这 3s 让给
/// 用户看内容 / 手动交互。3s 后 focused=true 仍关(保留“用户主动切走”语义)。
/// 见 `docs/briefs/*/popup-flap-focus-race.md` 或 08 case C。
///
/// popup 该由谁关?
///   - 用户手动关(点掉 / 快捷键):永远可以
///   - 新一轮开始:`should_close_popup_on_running`(Idle/LastFailed/Pending->Running)
///     在 http_server 里已经自动 close,和本开关无关
///   - watcher 后台收:配对本开关。开 -> 起,关 -> 让 popup 陪着,直到用户或
///     新一轮把它收掉
/// 见 `docs/briefs/*/vm-e2e-focus-watcher-conflict.md` 或 scenario 08 case C。
pub(super) fn schedule_focus_watcher(
    app: &AppHandle,
    popup_list: &PopupList,
    session: &Session,
    token: u64,
    label: String,
) {
    // 关联 focus_suppression_enabled:false 时用户明说“聚焦也要弹”,watcher 不该
    // 后台把它秒关。读不到 settings 时(unwrap_or)保守 true,维持历史行为。
    let focus_suppression_enabled = app
        .try_state::<Arc<Mutex<SettingsStore>>>()
        .and_then(|s| s.lock().ok().map(|g| g.settings.focus_suppression_enabled))
        .unwrap_or(true);
    if !focus_suppression_enabled {
        crate::log_debug!(
            "popup",
            "focus-watcher skipped task_id={} reason=focus_suppression_disabled",
            session.task_id,
        );
        return;
    }
    // Auto-dismiss when user focuses the associated terminal session.
    //
    // 0.0.3 unique-label + 捕获 my_label:即使 watcher 在 sleep 期间 popup 被
    // 别路径关 + 同 session 新 popup 进来(新 label,新 token),本 watcher 醒
    // 来时 `is_popup_instance_alive(my_label, my_token)` 直接 false -> 退出,
    // 不可能动到新 popup。0.0.2 / 0.0.3 反复出现的“听到声音没 popup”在此根除。
    // 0.0.8+:是否启动 watcher 由 SurfaceEvidence resolver 的 capability 决定;
    // tty / terminal_id 精确 surface 可检测聚焦,workspace fallback 不启动 watcher。
    let evidence_for_watcher = SurfaceEvidence::from_session(session);
    let resolved_for_watcher = resolve_surface(&evidence_for_watcher);
    log_resolution(
        "surface",
        "popup_focus_watcher",
        &evidence_for_watcher,
        &resolved_for_watcher,
    );
    if resolved_for_watcher.capabilities.can_detect_focused {
        let app_clone = app.clone();
        let popup_list_clone = popup_list.clone();
        let task_id = session.task_id.clone();
        let my_token = token;
        let my_label = label.clone();
        let surface_kind = resolved_for_watcher.kind;
        let surface_reason = resolved_for_watcher.reason;
        // Grace period 起点 = popup show 那一刻(即 schedule_focus_watcher 被调
        // 用时,show_popup 已经 push 到 popup_list)。捕获进闭包,后台线程用它
        // 判本次探测是否还在宽限期内。
        let shown_at = Instant::now();
        crate::log_info!(
            "popup",
            "focus-watcher started task_id={} token={} surface={:?} reason={} bindings={} grace_ms={}",
            task_id,
            my_token,
            surface_kind,
            surface_reason,
            crate::surface::binding_hints_for_log(&evidence_for_watcher),
            FOCUS_WATCHER_GRACE.as_millis(),
        );
        std::thread::spawn(move || {
            let mut consecutive_unfocused: u32 = 0;
            // 哨兵:每 popup 至多差分诊断一次;聚焦后重置,允许下次 popup 再诊断。
            let mut mismatch_probed = false;
            loop {
                std::thread::sleep(FOCUS_WATCHER_INTERVAL);
                let still_my_instance =
                    is_popup_instance_alive(&popup_list_clone, &my_label, my_token);
                if !still_my_instance {
                    // 我的 popup 已经被别路径关掉,watcher 退出
                    crate::log_debug!(
                        "popup",
                        "focus-watcher exit (my popup gone) task_id={} token={} surface={:?}",
                        task_id,
                        my_token,
                        surface_kind,
                    );
                    break;
                }
                if is_surface_focused(&evidence_for_watcher) {
                    consecutive_unfocused = 0;
                    mismatch_probed = false;
                    // 宽限期内即使 focused=true 也不关,给用户看清 popup 的时间。
                    // 见 [`FOCUS_WATCHER_GRACE`] 注释里的“闪一下”根因。
                    if !should_close_on_focus_probe(shown_at, Instant::now(), FOCUS_WATCHER_GRACE) {
                        crate::log_debug!(
                            "popup",
                            "focus-watcher grace hold task_id={} token={} elapsed_ms={} grace_ms={}",
                            task_id,
                            my_token,
                            shown_at.elapsed().as_millis(),
                            FOCUS_WATCHER_GRACE.as_millis(),
                        );
                        continue;
                    }
                    crate::log_info!(
                        "popup",
                        "focus-watcher closing (terminal focused) task_id={} token={} surface={:?} reason={} tty_present={} terminal_id_hint={}",
                        task_id,
                        my_token,
                        surface_kind,
                        surface_reason,
                        evidence_for_watcher
                            .terminal_tty
                            .as_deref()
                            .is_some_and(|tty| !tty.is_empty()),
                        terminal_id_log_hint(evidence_for_watcher.terminal_id.as_deref()),
                    );
                    close_popup_by_label(
                        &app_clone,
                        &my_label,
                        &popup_list_clone,
                        PopupCloseReason::FocusWatcher,
                    );
                    break;
                }
                consecutive_unfocused += 1;
                if should_probe_focus_mismatch(
                    consecutive_unfocused,
                    FOCUS_WATCHER_MISMATCH_PROBE_AFTER,
                    matches!(surface_kind, SurfaceKind::OttyTerminal),
                    mismatch_probed,
                ) {
                    mismatch_probed = true;
                    probe_otty_focus_mismatch(&evidence_for_watcher);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{should_close_on_focus_probe, should_probe_focus_mismatch, FOCUS_WATCHER_GRACE};
    use std::time::{Duration, Instant};

    /// 双重验证守门:iTerm2 / Terminal 两个分支都必须先校验
    /// frontmost-of-window 才返回 tty 匹配。任何分支漏掉这条都会让本测 fail。
    /// 宽限期正中间探到 focused=true -> hold(不关)
    #[test]
    fn focus_probe_within_grace_holds() {
        let shown = Instant::now();
        let probe = shown + Duration::from_millis(500);
        assert!(!should_close_on_focus_probe(
            shown,
            probe,
            FOCUS_WATCHER_GRACE
        ));
    }

    /// 宽限期后探到 focused=true -> 允许 close(保留“用户主动切走 -> 收 popup”语义)
    #[test]
    fn focus_probe_after_grace_allows_close() {
        let shown = Instant::now();
        let probe = shown + FOCUS_WATCHER_GRACE + Duration::from_millis(100);
        assert!(should_close_on_focus_probe(
            shown,
            probe,
            FOCUS_WATCHER_GRACE
        ));
    }

    /// 恰好等于宽限期 -> 允许 close(边界含右端;3s 后立刻收算合理)
    #[test]
    fn focus_probe_at_grace_boundary_allows_close() {
        let shown = Instant::now();
        let probe = shown + FOCUS_WATCHER_GRACE;
        assert!(should_close_on_focus_probe(
            shown,
            probe,
            FOCUS_WATCHER_GRACE
        ));
    }

    /// 用户 log 里那次 flap(1398ms)按修后逻辑应该 hold
    #[test]
    fn focus_probe_user_reported_flap_would_be_prevented() {
        let shown = Instant::now();
        let probe = shown + Duration::from_millis(1398);
        assert!(
            !should_close_on_focus_probe(shown, probe, FOCUS_WATCHER_GRACE),
            "1398ms(用户报告的 flap 时长)必须落在 grace 内,否则修法无效"
        );
    }

    /// 常量正确性:grace ≥ 2s(明显 flap 感知阈值),≤ 5s(不影响用户手动切走 -> 收响应)
    #[test]
    fn focus_watcher_grace_is_within_sensible_range() {
        assert!(FOCUS_WATCHER_GRACE >= Duration::from_secs(2));
        assert!(FOCUS_WATCHER_GRACE <= Duration::from_secs(5));
    }

    // ---------- 哨兵:otty 差分诊断触发判定 ----------

    /// 达到阈值 + otty surface + 未诊断 → 触发
    #[test]
    fn mismatch_probe_triggers_at_threshold() {
        assert!(should_probe_focus_mismatch(10, 10, true, false));
    }

    /// 未达阈值不触发
    #[test]
    fn mismatch_probe_waits_for_threshold() {
        assert!(!should_probe_focus_mismatch(9, 10, true, false));
    }

    /// 非 otty surface(herdr/ghostty/orca…)不触发——差分诊断目前只对 otty 实现
    #[test]
    fn mismatch_probe_otty_only() {
        assert!(!should_probe_focus_mismatch(99, 10, false, false));
    }

    /// 已诊断过不再触发(每 popup 至多一次,防刷屏)
    #[test]
    fn mismatch_probe_once_per_popup() {
        assert!(!should_probe_focus_mismatch(99, 10, true, true));
    }

    /// 聚焦后计数器清零,再次累积可重新触发(下次 popup / 用户切走又回来)
    #[test]
    fn mismatch_probe_resets_after_focus() {
        assert!(!should_probe_focus_mismatch(0, 10, true, false));
        assert!(should_probe_focus_mismatch(10, 10, true, false));
    }
}
