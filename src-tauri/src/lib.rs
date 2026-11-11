mod agent_presence;
mod bootstrap;
pub mod cmux;
pub mod codex_appserver;
pub mod codex_rollout;
mod commands;
pub mod confetti;
pub mod herdr;
pub mod herdr_remote;
#[path = "bin/pokehook.rs"]
pub mod hook_cli;
pub mod hook_deploy;
mod popup_decision;
mod runtime_metrics;
mod session_lifecycle;
pub mod supervisor;
pub mod wezterm;
// 这几个模块对外 pub,是为了让 `tests/http_integration.rs` 这个外部
// integration test crate 能拿到 AppState / 三个 trait / SessionStore。
// 生产代码内部仍从 crate root 引用,行为零变化。
pub mod custom_sounds;
pub mod ghostty_reconcile;
pub mod http_server;
pub mod logger;
pub mod monitors;
pub mod osascript;
pub mod popup;
pub mod quiet_mode;
pub mod sessions;
pub mod settings;
pub mod shortcut;
pub mod sound;
pub mod surface;
pub mod transcript_watch;
pub mod transport;
mod tray;
#[cfg(target_os = "macos")]
mod tray_quiet_title;
pub mod warp;

use codex_appserver::{CodexAppServerHandle, CodexThreadStatusCache};
use codex_rollout::AutoApprovalCache;
use http_server::{AppEventEmitter, AppState, SettingsNotificationGate};
use popup::{AppPopupController, AppleScriptFocusOracle};
use sessions::SessionStore;
use settings::SettingsStore;
use sound::AppSoundPlayer;
use std::sync::{Arc, Mutex};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if hook_cli::should_run_from_current_process() {
        hook_cli::main();
        return;
    }

    // global-shortcut handler 需要 popup_list,但 popup_list 在 setup 里才创建。
    // 解法:用 Arc<OnceLock> 占位,plugin 装配时拿引用,setup 里 set 真值,
    // handler 第一次触发时已经 set 过。
    let pl_handle: Arc<std::sync::OnceLock<popup::PopupList>> =
        Arc::new(std::sync::OnceLock::new());
    let pl_for_handler = pl_handle.clone();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if shortcut::handle_quiet_mode_press(app, shortcut, event) {
                        return;
                    }
                    if let Some(pl) = pl_for_handler.get() {
                        shortcut::handle_press(app, shortcut, event, pl);
                    }
                })
                .build(),
        );

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(move |app| {
            // macOS 的 "菜单栏工具" 模式通过 Info.plist 的 LSUIElement=true 声明
            // （见 tauri.conf.json / 打包产物的 Info.plist）。这样 NSApp 从启动
            // 之初就不是 Regular app，创建窗口时不会激活本进程、不抢用户输入焦点。
            // 这里不再运行时调用 set_activation_policy——实测会导致 dev 下 NSApp
            // 在 finishLaunching 时立刻退出。

            let data_dir = dirs_next().unwrap_or_else(|| std::path::PathBuf::from("."));

            // Logger 必须最早初始化(早于 SessionStore::load / SettingsStore::load
            // 等任何可能写日志的模块)。DEBUG 启用与否由 POKEPOKE_DEBUG env 在 init
            // 内部读取——不再依赖 settings,见 scope-audit §3.8。
            logger::init(data_dir.join("pokepoke.log"));
            // panic hook 必须在 logger 初始化后立刻装：任何线程 panic 落盘
            // pokepoke.log(含 backtrace),让"watcher 静默死亡 / daemon 无痕退出"
            // 这类可靠性问题从不可见变为可诊断。见 brief 20260802-fix-reliability-findings。
            logger::install_panic_hook();

            if let Ok(resource_dir) = app.path().resource_dir() {
                let sounds_dir = resource_dir.join("sounds");
                if sounds_dir.is_dir() {
                    crate::sound::init_bundled_sounds_dir(sounds_dir);
                }
            }

            let runtime_metrics = Arc::new(runtime_metrics::RuntimeMetrics::new());
            runtime_metrics::install(runtime_metrics.clone());
            app.handle().manage(runtime_metrics);

            // W3(brief 20260813-watcher-supervisor):watcher 统一监督 monitor ——
            // 必须在 logger init 之后(与 runtime_metrics::install 同批装配),
            // 心跳停滞时 log_warn + metrics 计数 + 事件环,诊断导出可见。
            supervisor::spawn_monitor(std::time::Duration::from_secs(5));

            #[cfg(not(target_os = "macos"))]
            log_warn!(
                "platform",
                "PokePoke targets macOS only; session reaper, popup placement, and terminal focus are unsupported on this platform"
            );

            // Migrate legacy data file name
            let legacy_path = data_dir.join("notifications.json");
            let store_path = data_dir.join("sessions.json");
            if !store_path.exists() && legacy_path.exists() {
                let _ = std::fs::rename(&legacy_path, &store_path);
            }

            let store = Arc::new(Mutex::new(SessionStore::load(store_path.clone())));
            SessionStore::spawn_flush_worker(store.clone());
            let popup_list = popup::create_popup_list();
            // 把 popup_list set 进 OnceLock,让 global-shortcut handler 看得到
            let _ = pl_handle.set(popup_list.clone());

            let settings_path = data_dir.join("settings.json");
            // 抓"首次运行"快照在 SettingsStore::load 之前——load 不写文件,
            // 所以 settings_path.exists() 此时仍准确反映"用户曾经有没有 PokePoke
            // 留下的痕迹"。后面 ensure_hook_deployed 之后用这个标志决定是否
            // 触发首次自动接入(写 cc + cursor hooks),完成后调 save 把
            // settings.json 落盘——下次启动 is_first_run=false,不再重装。
            //
            // review H1:0.0.x 旧版本可能没落 settings.json,但只要用过就一定落过
            // sessions.json(hook 一响就写)。把 sessions.json 作为补充判据,避免
            // "老用户升级 + 手动断开过某端"被误判 first-run 后再次自动重装。
            let is_first_run = !settings_path.exists() && !store_path.exists();
            let settings_store = Arc::new(Mutex::new(SettingsStore::load(settings_path)));

            app.handle().manage(store.clone());
            app.handle().manage(popup_list.clone());
            app.handle().manage(settings_store.clone());

            // §3.5 Hook drift state:启动自检 detect 后写,前端"一键修复"
            // 成功后清。app 通过 manage 持有,frontend / commands.rs 共享。
            let hook_drift_state = Arc::new(Mutex::new(commands::HookDriftState::default()));
            app.handle().manage(hook_drift_state.clone());

            let quiet_mode_state = Arc::new(quiet_mode::QuietModeState::new());
            app.handle().manage(quiet_mode_state.clone());

            let initial_tray_style = settings_store.lock().unwrap_or_else(|p| p.into_inner()).settings.tray_icon_style;
            tray::create_tray(app.handle(), initial_tray_style)?;

            let app_handle = app.handle().clone();
            let popup_controller = Arc::new(AppPopupController::new(
                app_handle.clone(),
                popup_list.clone(),
            ));
            let sound_player = Arc::new(AppSoundPlayer::new(settings_store.clone()));
            let focus_oracle = Arc::new(AppleScriptFocusOracle);
            let event_emitter = Arc::new(AppEventEmitter::new(app_handle.clone()));
            let gate = Arc::new(SettingsNotificationGate::new(settings_store.clone()));
            // W8(brief 20260813-session-memory):五窗口单锁多槽合一。
            let memory = Arc::new(crate::http_server::SessionMemory::default());
            let auto_approval = Arc::new(AutoApprovalCache::default());
            let codex_thread_status = Arc::new(CodexThreadStatusCache::default());
            let codex_appserver = CodexAppServerHandle::spawn(codex_thread_status.clone());
            let mut app_state = AppState {
                store: store.clone(),
                popup: popup_controller,
                sound: sound_player,
                focus: focus_oracle,
                events: event_emitter,
                gate,
                memory: memory.clone(),
                auto_approval: auto_approval.clone(),
                codex_thread_status,
                codex_appserver,
                settings: settings_store.clone(),
                quiet_mode: Some(quiet_mode_state.clone()),
                staging: crate::http_server::StagingDispatcher::inline(),
            };
            // popup 呈现(gate 里的 focus osascript 探测 + show/sound)脱离 /notify
            // 响应路径,交给 notify-staging 线程 FIFO 串行执行。
            // 见 docs/briefs/20261013-focus-off-response-path.md。
            app_state.staging = crate::http_server::StagingDispatcher::spawn_worker(app_state.clone());
            // commands::focus_session_terminal 需要拿 SessionMemory 注入,所以
            // 也得 manage 一份(Tauri 的 State 是按类型查的)。
            app.handle().manage(memory);
            let transcript_watch_state = app_state.clone();
            transcript_watch::spawn_transcript_interruption_watcher(transcript_watch_state);
            // Codex 自动审批判据缓存轮询:5s 读 rollout 文件更新 cache,
            // evaluate_and_show_popup 查询决定是否抑制。见 docs/briefs/20260715-codex-auto-approval-suppression.md
            codex_rollout::spawn_codex_rollout_watcher(app_state.clone());
            // Codex app-server thread status 订阅:thread/resume + status/changed,
            // 区分自动审批回环 Idle 与轮次真结束。见 docs/briefs/20260715-codex-appserver-thread-status.md
            codex_appserver::spawn_codex_thread_scan_watcher(app_state.clone());
            // brief 20260629:Ghostty terminal_id 周期对账(title 主键 + cwd 兜底),
            // 补单次 capture 的冷启动缺口 + 纠正 cwd first-match 错绑。
            ghostty_reconcile::spawn_ghostty_reconcile_watcher(app_state.clone());

            // Herdr 探测:仅日志,不阻塞。让用户 / 我们看诊断日志时能立刻分辨:
            //   "有 herdr 但没 binding" 是 herdr 没 hook 起来
            //   "没 herdr" 就走既有 Ghostty 路径
            {
                let home = std::env::var("HOME").unwrap_or_default();
                let sock = std::path::PathBuf::from(&home).join(".config/herdr/herdr.sock");
                match crate::herdr::ping(&sock) {
                    Some(info) => crate::log_info!(
                        "herdr", "detected version={} protocol={}", info.version, info.protocol
                    ),
                    None => crate::log_info!("herdr", "not running or socket unreachable at {:?}", sock),
                }
            }

            // review H4:单实例判定必须发生在任何写盘副作用之前。
            //   - bind 成功 → 后续正常起面板 / 清 session / spawn accept
            //   - PeerAlive → 立刻 app.exit,不能继续跑 clear_sessions / bootstrap /
            //     spawn_drift_detection —— 否则半活实例会去动共享 sessions.json /
            //     settings.json / 三端 hook 配置,把主实例状态搅烂。
            let Some(std_listener) = http_server::bind_or_detect_peer() else {
                log_warn!(
                    "app",
                    "另一实例已在运行(UDS PeerAlive) → 退出本进程,避免污染共享状态"
                );
                app.handle().exit(0);
                return Ok(());
            };

            // 单实例确认后再预热 panel webview,避免被顶掉的第二实例白建 WKWebView。
            tray::prewarm_session_panel(app.handle());

            // review M2:clear 在 accept 前跑,消除"刚 upsert 的真 session 被清历史残留误删"的窄窗。
            session_lifecycle::clear_sessions_on_startup(&store);

            // 远程 herdr 监视器(2026-08-07,brief 20260804-herdr-remote-notification
            // 修订版):公司机 PokePoke 通过 SSH 复用 herdr --remote 的连接,监视
            // 远程 agent 状态并弹本地 popup。自动发现(本地/远程),零配置;发现不到
            // 就静默退避重试,不干扰。详见 herdr_remote.rs 模块注释。
            // 必须在 app_state 被 serve_with_listener move 之前 clone。
            let monitor_state = app_state.clone();
            herdr_remote::spawn_monitor(monitor_state);

            tauri::async_runtime::spawn(async move {
                http_server::serve_with_listener(app_state, std_listener).await;
            });

            // 远程 herdr 监视器说明见上(已 clone 到 monitor_state)。
            session_lifecycle::spawn_session_reaper(
                store.clone(),
                popup_list.clone(),
                app.handle().clone(),
            );

            // 启动期把 bundle 内的 poke-hook 部署到 ~/.local/bin/poke-hook(自愈)。
            // 协议升级时(server-hook transport 变化等)用户不必再 `pnpm install:hook`,
            // 重启 PokePoke 即可。详见 `docs/briefs/20260423-bundle-hook-binary.md`。
            // 必须在 detect_hook_drift_and_emit 之前调:drift 检测要 spawn 子进程跑
            // ~/.local/bin/poke-hook --check-*,如果 hook 还没部署到位就会查不到。
            // 本身只做本地 fs::copy,毫秒级,留在同步 setup 里。
            commands::integration::ensure_hook_deployed(app.handle());

            // 首启自动接入(run_first_run_hook_install) + 隐性补齐(ensure_auto_connect_hooks)
            // + drift 检测(detect_hook_drift_and_emit) 一并挪进独立线程,保持内部串行
            // 保序。前两步会跑 login-shell CLI 探测和子进程 --check-*,rc 复杂时可到
            // 秒级,不再拖 Tauri setup 阶段(2026-07-20)。详见 spawn_post_deploy_pipeline
            // 内部注释。
            bootstrap::spawn_post_deploy_pipeline(
                app.handle().clone(),
                is_first_run,
                settings_store.clone(),
            );

            // PendingUpdate:后台巡检命中新版后,把 Update 句柄缓存起来给
            // `install_update` command 复用,避免点击【立即升级】时再走一次
            // updater.check() 网络往返(评审 H4)。 None = 当前无可升级版本。
            // N'2:type alias 提升可读性,跟 PopupList 一组。
            let pending_update: commands::PendingUpdate =
                Arc::new(Mutex::new(commands::PendingUpdateState::None));
            app.handle().manage(pending_update.clone());

            bootstrap::spawn_updater_poll(
                app.handle().clone(),
                settings_store.clone(),
                popup_list.clone(),
                pending_update.clone(),
            );

            // 全局快捷键监听 popup-shortcuts-updated,popup_list 非空时 register
            // 前缀+1~9 + 前缀+Enter,空时全部 unregister。前缀+Z 常驻。
            shortcut::install_listener(app.handle(), popup_list.clone());
            shortcut::register_persistent_shortcuts(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::get_session_by_id,
            commands::remove_session,
            commands::close_popup_window,
            commands::focus::open_session_source,
            commands::focus::focus_session_terminal,
            commands::integration::check_cc_integration,
            commands::integration::repair_cc_integration,
            commands::integration::check_codex_integration,
            commands::integration::check_cursor_integration,
            commands::integration::install_cursor_integration,
            commands::integration::uninstall_cursor_integration,
            commands::integration::check_opencode_integration,
            commands::integration::install_opencode_integration,
            commands::integration::uninstall_opencode_integration,
            commands::integration::check_pi_integration,
            commands::integration::install_pi_integration,
            commands::integration::uninstall_pi_integration,
            commands::get_settings,
            commands::save_settings,
            commands::open_settings_window,
            commands::close_settings_window,
            commands::hide_panel,
            commands::sounds::list_system_sounds,
            commands::sounds::list_bundled_sounds,
            commands::sounds::list_custom_sounds,
            commands::sounds::import_custom_sound,
            commands::sounds::delete_custom_sound,
            commands::sounds::preview_sound,
            commands::celebrate,
            commands::close_confetti_overlay,
            commands::log_from_frontend,
            commands::app_version,
            commands::integration::repair_hook_drift,
            commands::integration::get_hook_drift_state,
            commands::integration::list_managed_integrations,
            commands::integration::check_managed_integration,
            commands::integration::connect_managed_integration,
            commands::integration::disconnect_managed_integration,
            commands::quit_app,
            commands::updater::check_update_now,
            commands::updater::mark_update_skipped,
            commands::updater::install_update,
            commands::updater::get_pending_update_info,
            commands::updater::download_update_with_progress,
            commands::updater::install_cached_update,
            commands::updater::dismiss_update_popup,
            commands::diagnostics::export_diagnostics,
            commands::quiet_mode::get_quiet_mode_status,
            commands::quiet_mode::start_quiet_mode,
            commands::quiet_mode::cancel_quiet_mode,
            commands::quiet_mode::open_quiet_mode_picker,
            commands::quiet_mode::close_quiet_mode_picker,
        ])
        .build(tauri::generate_context!())
        .expect("error while building PokePoke")
        .run(|_app_handle, event| {
            // NSApp finishLaunching 之后才切换激活策略。
            // 放在这个时机的原因：
            //   - dev 模式下裸二进制不读 Info.plist，LSUIElement 只在打包产物生效
            //   - 直接在 setup 里同步切会早于 finishLaunching，导致 NSApp 启动即退出
            //   - Ready 事件保证 run loop 已稳定进入
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Ready = event {
                let _ = _app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            // 只拦截"用户交互导致的退出请求"（关最后一个窗口等），code=None；
            // 托盘 Quit 走 app.exit(0)，code=Some(0)，不拦，让 app 正常退出。
            if let tauri::RunEvent::ExitRequested { ref api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            // TCP 端口由 OS 在进程退出时自动释放,无需显式清理(原 UDS sock + endpoints.json
            // 路径已在 §3.1 让出)。
        });
}

fn dirs_next() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = std::path::PathBuf::from(home).join(".pokepoke");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}
