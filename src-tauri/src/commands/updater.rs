use crate::settings::SettingsStore;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

/// 用户按 popup 右上角的 × —— 关掉 popup 但**不**动 `settings.update_skip_version`。
/// 后台 24h 巡检命中同 version 时还是会再弹。语义跟「跳过本版」不同:跳过 = 我对这个
/// 版本说不;dismiss = 暂时不决定,下次提醒我。
#[tauri::command]
pub fn dismiss_update_popup(
    version: String,
    popup_list: State<'_, crate::popup::PopupList>,
    app: AppHandle,
) {
    crate::log_info!(
        "updater",
        "user dismissed popup for version {} (will re-show on next 24h check)",
        version
    );
    if let Some(label) = crate::popup::find_update_popup_label(&popup_list, &version) {
        crate::popup::close_popup_by_label(
            &app,
            &label,
            &popup_list,
            crate::popup::PopupCloseReason::UpdaterDismiss,
        );
    }
}

/// 把"用户跳过本版"持久化到 settings.update_skip_version,并关掉对应的 updater
/// 提示 popup。后台 24h 巡检命中同 version 时会读取该字段后静默(高版本仍会弹)。
#[tauri::command]
pub fn mark_update_skipped(
    version: String,
    store: State<'_, Arc<Mutex<SettingsStore>>>,
    popup_list: State<'_, crate::popup::PopupList>,
    app: AppHandle,
) {
    {
        let mut s = store.lock().unwrap_or_else(|p| p.into_inner());
        let mut next = s.settings.clone();
        next.update_skip_version = Some(version.clone());
        s.update(next);
    }
    crate::log_info!("updater", "user skipped version {}", version);
    // 0.0.3 unique-label:label 带 token,必须反查当前实例;找不到 = 已被别路径关
    if let Some(label) = crate::popup::find_update_popup_label(&popup_list, &version) {
        crate::popup::close_popup_by_label(
            &app,
            &label,
            &popup_list,
            crate::popup::PopupCloseReason::UpdaterSkipped,
        );
    }
}

/// 后台 updater 巡检命中新版后缓存 `Update` 句柄给 updater command 复用。
/// `lib.rs` 创建 + manage,`commands.rs` 通过 `State` 读。
///
/// 三态机:
/// - `None`:当前无可升级版本(或被用户 skip 过 —— H'2:skip 版本不进缓存)
/// - `Available(Update)`:巡检命中,等待用户触发下载
/// - `Downloaded { update, bytes }`:popup 的两步流程已下完,等待 install_cached_update 装
///
/// 三态防止"已下载但 update 句柄丢了"或"反复下载"的不一致。
pub type PendingUpdate = Arc<Mutex<PendingUpdateState>>;

#[derive(Default)]
pub enum PendingUpdateState {
    #[default]
    None,
    Available(tauri_plugin_updater::Update),
    Downloaded {
        update: tauri_plugin_updater::Update,
        bytes: Vec<u8>,
    },
}

impl PendingUpdateState {
    pub fn as_update(&self) -> Option<&tauri_plugin_updater::Update> {
        match self {
            Self::None => None,
            Self::Available(u) | Self::Downloaded { update: u, .. } => Some(u),
        }
    }
}

/// popup webview 加载后拉取本次升级元信息(版本、release notes、包大小)。
/// 替代旧的 URL hash 透传方案:notes 可能很长,hash 容易撞 URL 长度上限 +
/// 双重转义维护麻烦。
#[derive(serde::Serialize, Clone)]
pub struct PendingUpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub content_length: Option<u64>,
}

fn pending_info_from_update(update: &tauri_plugin_updater::Update) -> PendingUpdateInfo {
    PendingUpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
        content_length: None,
    }
}

/// 设置面板「检查更新」与后台巡检共用 `PendingUpdate` 缓存。
#[derive(serde::Serialize)]
pub struct CheckUpdateResult {
    pub available: bool,
    pub info: Option<PendingUpdateInfo>,
}

#[tauri::command]
pub async fn check_update_now(
    pending: State<'_, PendingUpdate>,
    store: State<'_, Arc<Mutex<SettingsStore>>>,
    app: AppHandle,
) -> Result<CheckUpdateResult, String> {
    use tauri_plugin_updater::UpdaterExt;

    let skip_ver = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .settings
        .update_skip_version
        .clone();
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            if Some(update.version.as_str()) == skip_ver.as_deref() {
                *pending.lock().unwrap_or_else(|p| p.into_inner()) = PendingUpdateState::None;
                crate::log_info!(
                    "updater",
                    "manual check: version {} skipped by user",
                    update.version
                );
                Ok(CheckUpdateResult {
                    available: false,
                    info: None,
                })
            } else {
                let info = pending_info_from_update(&update);
                *pending.lock().unwrap_or_else(|p| p.into_inner()) =
                    PendingUpdateState::Available(update);
                crate::log_info!("updater", "manual check: update available {}", info.version);
                Ok(CheckUpdateResult {
                    available: true,
                    info: Some(info),
                })
            }
        }
        Ok(None) => {
            *pending.lock().unwrap_or_else(|p| p.into_inner()) = PendingUpdateState::None;
            crate::log_info!("updater", "manual check: up_to_date");
            Ok(CheckUpdateResult {
                available: false,
                info: None,
            })
        }
        Err(e) => {
            crate::log_warn!("updater", "manual check failed: {}", e);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub fn get_pending_update_info(
    pending: State<'_, PendingUpdate>,
) -> Result<PendingUpdateInfo, String> {
    let guard = pending.lock().unwrap_or_else(|p| p.into_inner());
    let update = guard
        .as_update()
        .ok_or_else(|| "no cached update".to_string())?;
    Ok(pending_info_from_update(update))
}

/// 触发更新 popup 的「立即升级」按钮:走 updater 下载+安装+重启,与设置面板的
/// 「立即更新并重启」按钮等价。封装在 backend command 是为了让 popup webview 不需
/// 要单独引入 @tauri-apps/plugin-updater(popup bundle 体积也小一点)。
///
/// 0.0.3 H4 修复:不再现场 `updater.check()`,而是直接读后台巡检任务缓存的
/// `PendingUpdate`。避免多一次网络 RTT(按钮 hang 1-2s)+ 避免"远端 release 撤回
/// 导致 Ok(None) 出 'no update available' 错误"这种边界 case。缓存为空(用户手动
/// 在 popup 之前就触发了 mark_update_skipped 或后台还没首检完)时返回明确错误,
/// 前端拿来显示给用户(H3a)。
///
/// 0.0.5+:popup 改走 [`download_update_with_progress`] + [`install_cached_update`]
/// 两步路径,本 command 仍保留给设置面板"立即更新并重启"按钮使用(无进度需求)。
#[tauri::command]
pub async fn install_update(
    pending: State<'_, PendingUpdate>,
    app: AppHandle,
) -> Result<(), String> {
    let update = {
        let guard = pending.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_update()
            .cloned()
            .ok_or_else(|| "no cached update; try re-checking from settings panel".to_string())?
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

/// 0.0.5+:popup「立即升级」的第一步。下载 update 到内存,通过 `update-progress`
/// event 推进度给前端。下载完成后把 bytes 缓存进 `PendingUpdate::Downloaded`,
/// 由 [`install_cached_update`] 完成安装 + 重启。
///
/// progress event payload:`{ downloaded: u64, total: Option<u64> }`。前端先 listen
/// 再 invoke,避免早期 chunk 在订阅前 emit 丢失(R2)。
#[tauri::command]
pub async fn download_update_with_progress(
    pending: State<'_, PendingUpdate>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let update = {
        let guard = pending.lock().unwrap_or_else(|p| p.into_inner());
        match &*guard {
            PendingUpdateState::Available(u) => u.clone(),
            PendingUpdateState::Downloaded { .. } => {
                // 幂等:已下载好就当成功,前端直接进入 install 阶段
                return Ok(());
            }
            PendingUpdateState::None => {
                return Err("no cached update; try re-checking from settings panel".to_string());
            }
        }
    };

    let downloaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let downloaded_for_cb = downloaded.clone();
    let app_for_cb = app.clone();
    // chunk 回调一秒可能几百次,前端 throttle 即可;后端只如实 emit,避免漏推
    // 进度跳变(R7 留给前端)。
    let on_chunk = move |chunk_length: usize, content_length: Option<u64>| {
        let prev =
            downloaded_for_cb.fetch_add(chunk_length as u64, std::sync::atomic::Ordering::Relaxed);
        let now = prev + chunk_length as u64;
        let _ = app_for_cb.emit(
            "update-progress",
            serde_json::json!({
                "downloaded": now,
                "total": content_length,
                "phase": "downloading",
            }),
        );
    };
    let app_for_finish = app.clone();
    let on_finish = move || {
        let _ = app_for_finish.emit(
            "update-progress",
            serde_json::json!({
                "downloaded": 0u64,
                "total": null,
                "phase": "download_finished",
            }),
        );
    };

    let bytes = update
        .download(on_chunk, on_finish)
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut guard = pending.lock().unwrap_or_else(|p| p.into_inner());
        *guard = PendingUpdateState::Downloaded { update, bytes };
    }
    Ok(())
}

/// 0.0.5+:popup「立即升级」的第二步。从 [`download_update_with_progress`] 缓存的
/// bytes 装包并重启。install 阶段不可控空白(几秒到几十秒),前端在调用前应切到
/// indeterminate 进度条 + "正在安装并重启..."文案兜底。
#[tauri::command]
pub async fn install_cached_update(
    pending: State<'_, PendingUpdate>,
    app: AppHandle,
) -> Result<(), String> {
    let (update, bytes) = {
        let mut guard = pending.lock().unwrap_or_else(|p| p.into_inner());
        match std::mem::take(&mut *guard) {
            PendingUpdateState::Downloaded { update, bytes } => (update, bytes),
            PendingUpdateState::Available(u) => {
                // 还原 state 再报错,避免 take 把可用 update 吃掉
                *guard = PendingUpdateState::Available(u);
                return Err("update not yet downloaded".to_string());
            }
            PendingUpdateState::None => {
                return Err("no cached update".to_string());
            }
        }
    };
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
