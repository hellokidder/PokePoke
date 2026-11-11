//! popup label 编解码 + popup_list 查询 helper。
//!
//! 从 `popup.rs` 抽出:label 格式(`popup-{kind}-{token}-{payload}`)、
//! session/update label 互转、按 session_id/version/visual_index 查 popup_list。
//! 纯函数 + PopupList 只读查询,行为零变化;`popup.rs` 重导出保持外部引用不断。

use super::PopupList;

/// session_id(UUID,只含 hex + `-`)天然安全;version("0.0.3" 带 `.`)必须
/// 编码,否则 `WebviewWindowBuilder.build()` 直接 reject 整个 popup 创建。
/// `version_to_label_token` 做 `. → _` 编码,`find_update_popup_label` 也对入
/// 参做同等编码后再比对,语义对称。
pub fn session_popup_label(token: u64, session_id: &str) -> String {
    format!("popup-s-{}-{}", token, session_id)
}

/// updater 提示 popup label。详见 [`session_popup_label`] 总注释。
pub fn update_popup_label(token: u64, version: &str) -> String {
    format!("popup-u-{}-{}", token, version_to_label_token(version))
}

/// version 字符串里的 `.` / `+`(semver build metadata)在 Tauri label 字符集
/// 外,统一替换成 `_`。**不可逆** —— 不需要从 label 反推 version(后端有 settings
/// 持久化原始 version,前端通过 URL hash `&version=` 拿原始 version)。
fn version_to_label_token(version: &str) -> String {
    version.replace(['.', '+'], "_")
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PopupKind {
    Session,
    Update,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ParsedLabel {
    pub kind: PopupKind,
    pub token: u64,
    /// Session popup 时是 session_id;Update popup 时是 version。
    pub payload: String,
}

/// 解析 `popup-{kind}-{token}-{payload}` 格式。non-popup label 或格式错
/// (token 非数字、缺段)返回 None。**保留 payload 里的 `-`/`.`**(UUID
/// 用 `-`,version 用 `.`)—— payload 取从第 4 个 `-` 之后所有字符。
pub fn parse_popup_label(label: &str) -> Option<ParsedLabel> {
    let rest = label.strip_prefix("popup-")?;
    let (kind_str, rest) = rest.split_once('-')?;
    let kind = match kind_str {
        "s" => PopupKind::Session,
        "u" => PopupKind::Update,
        _ => return None,
    };
    let (token_str, payload) = rest.split_once('-')?;
    let token = token_str.parse::<u64>().ok()?;
    if payload.is_empty() {
        return None;
    }
    Some(ParsedLabel {
        kind,
        token,
        payload: payload.to_string(),
    })
}

/// 判定某 label 是否 updater 提示。等价 `parse_popup_label(l).map(|p| p.kind == Update)`,
/// 但热路径(每次 popup_list 迭代)避免分配 String。
pub fn is_update_popup_label(label: &str) -> bool {
    label.starts_with("popup-u-")
}

/// 反解 session popup label 拿 session_id。updater popup / 非 popup label 返回 None。
pub fn session_id_from_label(label: &str) -> Option<&str> {
    let rest = label.strip_prefix("popup-s-")?;
    // 跳过 token 段
    let (_, payload) = rest.split_once('-')?;
    if payload.is_empty() {
        None
    } else {
        Some(payload)
    }
}

/// 在 popup_list 里找当前指向某 session_id 的 popup label(如果有)。
///
/// 同 session 一时只有一个 popup 在屏(由 show_popup 的 already_visible 检查
/// 保证),所以最多返回一个。`close_popup(session_id)` 用它把 session_id-based
/// 调用桥接到 label-based 销毁。
pub fn find_session_popup_label(popup_list: &PopupList, session_id: &str) -> Option<String> {
    let list = popup_list.lock().ok()?;
    list.iter()
        .find(|(label, _, _, _)| session_id_from_label(label).is_some_and(|sid| sid == session_id))
        .map(|(label, _, _, _)| label.clone())
}

/// 在 popup_list 里找当前指向某 version 的 updater popup label。
/// `mark_update_skipped(version)` 用它定位要关的 popup。
/// 入参 version 是**原始**形态("0.0.3"),需要走 `version_to_label_token` 对称
/// 编码后再比对 —— label 里存的是 `_` 编码版,直接 `==` 比 raw version 会全 miss。
pub fn find_update_popup_label(popup_list: &PopupList, version: &str) -> Option<String> {
    let target_payload = version_to_label_token(version);
    let list = popup_list.lock().ok()?;
    list.iter()
        .find(|(label, _, _, _)| {
            parse_popup_label(label)
                .is_some_and(|p| p.kind == PopupKind::Update && p.payload == target_payload)
        })
        .map(|(label, _, _, _)| label.clone())
}

/// 屏上第 `n` 个**session** popup 的 session_id(0-based,按 popup_list 插入序,
/// 跨屏统一编号 —— 全局编号策略)。updater popup 不参与计数,因为快捷键消费
/// 语义是"跳转到 session 对应终端",对 updater popup 无效。
///
/// 用于全局快捷键 `⌘1` ~ `⌘9` / `⌘Enter` 的路由。
pub fn session_popup_at_visual_index(popup_list: &PopupList, n: usize) -> Option<String> {
    let list = popup_list.lock().ok()?;
    list.iter()
        .filter_map(|(label, _, _, _)| session_id_from_label(label))
        .nth(n)
        .map(String::from)
}

/// 屏上 session popup 总数(updater popup 不计入)。
/// 用于快捷键 register/unregister 的判定:从 0 → ≥1 时 register,反过来 unregister。
pub fn session_popup_count(popup_list: &PopupList) -> usize {
    popup_list
        .lock()
        .map(|list| {
            list.iter()
                .filter(|(label, _, _, _)| !is_update_popup_label(label))
                .count()
        })
        .unwrap_or(0)
}

/// 判定"此 (label, token) 对应的 popup 实例是否还在 list 里"。
/// auto-close timer 和 focus-watcher 都用这个 helper 决策"我的 popup 还活着吗"——
/// **必须**走 (label, token) 双键,只看 label 会跨实例错杀(同 session 复用 label)。
/// 详见 `show_popup` 内 focus-watcher 注释。
pub fn is_popup_instance_alive(popup_list: &PopupList, label: &str, token: u64) -> bool {
    popup_list
        .lock()
        .map(|list| list.iter().any(|(l, _, t, _)| l == label && *t == token))
        .unwrap_or(false)
}

/// session popup 的 (label, visual_index) 快照,用于 `popup-shortcuts-updated`
/// event 推到前端,让每个 popup window 重渲染右下角的 ⌘N 角标。`index` 是 0-based,
/// 前端要显示成 `⌘${index+1}`。`index >= 9` 的 popup 不分配角标(全局快捷键
/// 上限 ⌘1~⌘9 + ⌘Enter,⌘Enter 等价 ⌘1)。
pub fn session_popup_shortcut_map(popup_list: &PopupList) -> Vec<(String, usize)> {
    let list = match popup_list.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    list.iter()
        .filter(|(label, _, _, _)| !is_update_popup_label(label))
        .enumerate()
        .map(|(idx, (label, _, _, _))| (label.clone(), idx))
        .collect()
}
