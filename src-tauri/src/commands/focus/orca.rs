//! Orca IDE 终端: `orca terminal list/switch` CLI 聚焦 + visual layout 抑制判定。
//! Agent 直接跑在 Orca worktree pane 里时 short-circuit;herdr/cmux/wezterm/otty 优先。

#[path = "orca_herdr.rs"]
mod orca_herdr;
pub(crate) use orca_herdr::{
    apply_orca_binding_for_herdr_host_if_missing, focus_orca_herdr_anchor_by_tty,
    orca_herdr_host_focused,
};

#[path = "orca_runtime.rs"]
mod orca_runtime;
// 运行时可用性前置门 + CLI 失败分类(2026-10-13 从本文件抽出,守 §4a 体量门禁)。
use orca_runtime::{run_orca_json, run_orca_json_checked, OrcaProbe};

use super::cli_timeout::{command_output_with_timeout, FOCUS_CLI_TIMEOUT};
use crate::sessions::{Session, SessionStore};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

const MACOS_ORCA_CLI: &str = "/Applications/Orca.app/Contents/Resources/bin/orca";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrcaTerminalRow {
    pub handle: String,
    pub tab_id: String,
    pub leaf_id: String,
    pub worktree_id: String,
    pub worktree_path: String,
    #[serde(default, deserialize_with = "deserialize_null_string")]
    pub title: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_output_at: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct OrcaCliEnvelope<T> {
    #[serde(default)]
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaTerminalListResult {
    #[serde(default)]
    terminals: Vec<OrcaTerminalRow>,
    #[serde(default)]
    visual_layouts: Vec<OrcaVisualLayout>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaVisualLayout {
    worktree_id: String,
    root: OrcaLayoutRoot,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaLayoutRoot {
    active_tab_id: String,
    #[serde(default)]
    tabs: Vec<OrcaLayoutTab>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaLayoutTab {
    tab_id: String,
    active_leaf_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaWorktreePsRow {
    worktree_id: String,
    #[serde(default)]
    is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrcaWorktreePsResult {
    #[serde(default)]
    worktrees: Vec<OrcaWorktreePsRow>,
}

fn resolve_orca_cli() -> PathBuf {
    if let Ok(cmd) = std::env::var("ORCA_CLI_COMMAND") {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() && Path::new(trimmed).is_file() {
            return PathBuf::from(trimmed);
        }
    }
    if Path::new(MACOS_ORCA_CLI).is_file() {
        return PathBuf::from(MACOS_ORCA_CLI);
    }
    PathBuf::from("orca")
}

fn deserialize_null_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}

fn parse_pane_key(pane_key: &str) -> Option<(&str, &str)> {
    let pane_key = pane_key.trim();
    let (tab_id, leaf_id) = pane_key.split_once(':')?;
    if tab_id.is_empty() || leaf_id.is_empty() {
        return None;
    }
    Some((tab_id, leaf_id))
}

/// `orca terminal list` 需要显式 `--include-visual-layouts` 才会返回 visualLayouts
/// (Orca CLI 1.4.171 起不再默认内嵌;缺失时 `orca_bound_pane_focused` 拿不到
/// active tab/leaf,聚焦检测整链恒 false)。旧版 CLI 不认该 flag 会 `ok=false`,
/// 回退到无 flag 调用(老版本默认内嵌 layouts,行为不变)。
///
/// 2026-08-07 回归后补防(见 brief 20260807-orca-e2e-scenario-and-diagnostic):
/// 外部 CLI 契约再变更时,这里要能露出信号而不是静默全灭 —— 见
/// [`warn_if_visual_layouts_contract_broken`]。
fn list_terminals() -> Option<OrcaTerminalListResult> {
    let result = match run_orca_json_checked::<OrcaTerminalListResult>(&[
        "terminal",
        "list",
        "--include-visual-layouts",
    ]) {
        OrcaProbe::Ok(result) => result,
        // 只有"参数不兼容"才回退(旧版 CLI 不认该 flag);运行时不可用 / 超时 /
        // 不可解析重试无意义,且已进冷却。见 orca_runtime.rs 模块注释。
        OrcaProbe::InvalidArgument(error) => {
            crate::log_info!(
                "focus",
                "orca terminal list --include-visual-layouts rejected ({}) — retry without flag",
                error.code
            );
            run_orca_json(&["terminal", "list"])?
        }
        OrcaProbe::Unavailable => return None,
    };
    warn_if_visual_layouts_contract_broken(&result.terminals, &result.visual_layouts);
    Some(result)
}

/// 纯谓词:terminals 存在但 visualLayouts 恒空 = Orca CLI 契约疑似变更。
/// 2026-08-07 教训:1.4.175 把 visualLayouts 改成 opt-in 后,`orca_bound_pane_focused`
/// 整链静默失效两天(聚焦抑制 + watcher 全灭),日志里只有普通的
/// `already_focused=false`。抽成 pure fn 供单测锁语义。
fn is_visual_layouts_contract_broken(terminals_len: usize, layouts_len: usize) -> bool {
    terminals_len > 0 && layouts_len == 0
}

/// 契约变更告警:只 warn 一次(进程生命周期),防 focus-watcher 每秒 tick 刷屏。
fn warn_if_visual_layouts_contract_broken(
    terminals: &[OrcaTerminalRow],
    layouts: &[OrcaVisualLayout],
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !is_visual_layouts_contract_broken(terminals.len(), layouts.len()) {
        return;
    }
    if WARNED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::log_warn!(
        "focus",
        "orca visualLayouts empty but {} terminal(s) present — focus detection degraded; Orca CLI may no longer serve layouts without --include-visual-layouts (brief 20260807-orca-terminal-list-visual-layouts-flag)",
        terminals.len(),
    );
}

fn list_worktrees_ps() -> Option<Vec<OrcaWorktreePsRow>> {
    run_orca_json::<OrcaWorktreePsResult>(&["worktree", "ps"]).map(|r| r.worktrees)
}

fn find_terminal_by_pane_key<'a>(
    terminals: &'a [OrcaTerminalRow],
    pane_key: &str,
) -> Option<&'a OrcaTerminalRow> {
    let (tab_id, leaf_id) = parse_pane_key(pane_key)?;
    terminals
        .iter()
        .find(|row| row.tab_id == tab_id && row.leaf_id == leaf_id)
}

fn worktree_id_matches(row: &OrcaTerminalRow, expected: &str) -> bool {
    row.worktree_id == expected
}

/// Cursor agent 常在 Orca 里从 "Cursor ready" shell 切到新的 split leaf;hook 仍报旧
/// `ORCA_PANE_KEY` 时,精确 leaf 已不在 list,但同 tab 仍有活跃 pane。此时按 tab +
/// worktree 找最近有输出的 connected terminal 兜底。
fn find_terminal_for_focus<'a>(
    terminals: &'a [OrcaTerminalRow],
    pane_key: &str,
    worktree_id: Option<&str>,
) -> Option<&'a OrcaTerminalRow> {
    if let Some(row) = find_terminal_by_pane_key(terminals, pane_key) {
        return Some(row);
    }
    let (tab_id, _) = parse_pane_key(pane_key)?;
    let mut candidates: Vec<&OrcaTerminalRow> = terminals
        .iter()
        .filter(|row| row.connected && row.tab_id == tab_id)
        .filter(|row| {
            worktree_id
                .filter(|expected| !expected.is_empty())
                .map(|expected| worktree_id_matches(row, expected))
                .unwrap_or(true)
        })
        .collect();
    match candidates.len() {
        0 => None,
        1 => Some(candidates[0]),
        _ => {
            if candidates.iter().all(|row| row.last_output_at.is_none()) {
                // 全部无 lastOutputAt:稳定排序会取列表末位,可能是同 tab 下任意
                // 无关 shell。跳错终端比不跳更糟,放弃 fallback(只留精确 leaf 匹配)。
                crate::log_warn!(
                    "orca",
                    "fallback: {} candidates all lack last_output_at, bailing",
                    candidates.len()
                );
                return None;
            }
            candidates.sort_by_key(|row| row.last_output_at.unwrap_or(0));
            candidates.last().copied()
        }
    }
}

/// `None` = Orca CLI 不可用,不得据此清 binding;`Some(false)` = 确认 pane/tab 已消失。
fn pane_still_reachable(pane_key: &str, worktree_id: Option<&str>) -> Option<bool> {
    let list = list_terminals()?;
    Some(find_terminal_for_focus(&list.terminals, pane_key, worktree_id).is_some())
}

pub(crate) fn should_clear_orca_binding_after_focus_fail(
    pane_key: &str,
    worktree_id: Option<&str>,
) -> bool {
    pane_still_reachable(pane_key, worktree_id) == Some(false)
}

pub(super) fn clear_stale_orca_binding(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    pane_key: &str,
) {
    let cleared = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear_surface_binding_if_matches(
            session_id,
            &crate::surface::SurfaceBinding::Orca {
                pane_key: pane_key.to_string(),
                worktree_id: None,
                tab_id: None,
            },
        );
    crate::log_info!(
        "focus",
        "cleared stale orca_binding session_id={} pane_key={} cleared={}",
        session_id,
        pane_key,
        cleared
    );
}

fn is_worktree_active(worktree_id: &str) -> bool {
    list_worktrees_ps().is_some_and(|rows| {
        rows.iter()
            .any(|row| row.worktree_id == worktree_id && row.is_active)
    })
}

/// `tab_id_hint` 有值时先比对 layout 的 active tab,不匹配可快速判定未聚焦。
fn layout_active_tab_matches_hint(
    layouts: &[OrcaVisualLayout],
    worktree_id: &str,
    tab_id_hint: &str,
) -> Option<bool> {
    let layout = layouts
        .iter()
        .find(|layout| layout.worktree_id == worktree_id)?;
    Some(layout.root.active_tab_id == tab_id_hint)
}

fn active_pane_matches_layout(
    layouts: &[OrcaVisualLayout],
    worktree_id: &str,
    pane_key: &str,
) -> bool {
    let (tab_id, leaf_id) = match parse_pane_key(pane_key) {
        Some(parts) => parts,
        None => return false,
    };
    let Some(layout) = layouts
        .iter()
        .find(|layout| layout.worktree_id == worktree_id)
    else {
        return false;
    };
    if layout.root.active_tab_id != tab_id {
        return false;
    }
    layout
        .root
        .tabs
        .iter()
        .any(|tab| tab.tab_id == tab_id && tab.active_leaf_id == leaf_id)
}

/// 用户是否正看着 bound Orca pane:Orca 为 active worktree + layout 内 active tab/leaf 命中。
/// `tab_id_hint` 为 session 存的 `orca_tab_id`,可在 leaf 比对前快速排除错误 tab。
pub(crate) fn orca_bound_pane_focused(
    pane_key: &str,
    worktree_id: Option<&str>,
    tab_id_hint: Option<&str>,
) -> bool {
    let Some(worktree_id) = worktree_id.filter(|w| !w.is_empty()) else {
        return false;
    };
    if !is_worktree_active(worktree_id) {
        return false;
    }
    let list = match list_terminals() {
        Some(list) => list,
        None => return false,
    };
    if let Some(hint) = tab_id_hint.filter(|tab_id| !tab_id.is_empty()) {
        match layout_active_tab_matches_hint(&list.visual_layouts, worktree_id, hint) {
            Some(false) => return false,
            None => {}
            Some(true) => {}
        }
    }
    active_pane_matches_layout(&list.visual_layouts, worktree_id, pane_key)
}

pub(crate) fn lookup_worktree_id_for_pane_key(pane_key: &str) -> Option<String> {
    let list = list_terminals()?;
    let row = find_terminal_by_pane_key(&list.terminals, pane_key)?;
    if row.worktree_id.is_empty() {
        return None;
    }
    Some(row.worktree_id.clone())
}

pub(super) fn pane_key_for_row(row: &OrcaTerminalRow) -> String {
    format!("{}:{}", row.tab_id, row.leaf_id)
}

/// `orca terminal switch --terminal <handle>`:激活 worktree + tab + split pane。
pub(crate) fn focus_orca_pane(pane_key: &str, worktree_id: Option<&str>) -> bool {
    let list = match list_terminals() {
        Some(list) => list,
        None => {
            crate::log_info!(
                "focus",
                "orca focus_orca_pane pane_key={} terminal list unavailable",
                pane_key
            );
            return false;
        }
    };
    let exact = find_terminal_by_pane_key(&list.terminals, pane_key).is_some();
    let row = match find_terminal_for_focus(&list.terminals, pane_key, worktree_id) {
        Some(row) => row,
        None => {
            crate::log_info!(
                "focus",
                "orca focus_orca_pane pane_key={} not found in terminal list",
                pane_key
            );
            return false;
        }
    };
    if !exact {
        crate::log_info!(
            "focus",
            "orca focus_orca_pane pane_key={} leaf miss -> tab fallback handle={} title={}",
            pane_key,
            row.handle,
            row.title
        );
    }
    if let Some(expected) = worktree_id.filter(|w| !w.is_empty()) {
        if row.worktree_id != expected {
            crate::log_info!(
                "focus",
                "orca focus_orca_pane pane_key={} worktree mismatch expected={} actual={}",
                pane_key,
                expected,
                row.worktree_id
            );
            return false;
        }
    }
    focus_orca_terminal_handle(&row.handle)
}

/// `orca terminal switch` 只切 worktree/tab/pane,不保证把 Orca 窗口拉到前台(实测
/// 前台是 Cursor 等其它 app 时 switch 后仍停留原 app)。补 `open -a Orca`,对齐
/// WezTerm `activate_wezterm_application` / Codex desktop activate 路径。
#[cfg(target_os = "macos")]
fn activate_orca_application() {
    match Command::new("open").args(["-a", "Orca"]).output() {
        Ok(o) if o.status.success() => {
            crate::log_info!("focus", "orca activate_application ok=true");
        }
        Ok(o) => {
            crate::log_info!(
                "focus",
                "orca activate_application exit={} stderr={}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Err(e) => {
            crate::log_info!("focus", "orca activate_application spawn err={}", e);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn activate_orca_application() {}

pub(super) fn focus_orca_terminal_handle(handle: &str) -> bool {
    let cli = resolve_orca_cli();
    let mut cmd = Command::new(&cli);
    cmd.args(["terminal", "switch", "--terminal", handle, "--json"]);
    let output = command_output_with_timeout(cmd, FOCUS_CLI_TIMEOUT, "orca/terminal-switch");
    match output {
        Some(o) if o.status.success() => {
            crate::log_info!("focus", "orca terminal switch handle={} ok=true", handle);
            activate_orca_application();
            true
        }
        Some(o) => {
            crate::log_info!(
                "focus",
                "orca terminal switch handle={} exit={} stderr={}",
                handle,
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            );
            false
        }
        None => {
            crate::log_info!("focus", "orca terminal switch handle={} timed out", handle);
            false
        }
    }
}

pub(crate) fn apply_orca_binding(
    store: &Arc<Mutex<SessionStore>>,
    task_id: &str,
    pane_key: &str,
    worktree_id: &str,
    tab_id: Option<String>,
) {
    let changed = {
        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
        store.set_surface_binding(
            task_id,
            crate::surface::SurfaceBinding::Orca {
                pane_key: pane_key.to_string(),
                worktree_id: Some(worktree_id.to_string()),
                tab_id,
            },
        )
    };
    if changed {
        crate::log_info!(
            "notify",
            "set_surface_binding orca task_id={} pane_key={} worktree_id={}",
            task_id,
            pane_key,
            worktree_id
        );
    }
}

pub(crate) fn decide_orca_capture_reason(
    session: &Session,
    herdr_pane_id: Option<&str>,
    cmux_surface_id: Option<&str>,
    wezterm_pane_id: Option<&str>,
    otty_pane_id: Option<&str>,
    is_new: bool,
    retry_eligible: bool,
) -> Option<&'static str> {
    use crate::surface::SurfaceBinding as B;
    if herdr_pane_id.is_some()
        || cmux_surface_id.is_some()
        || wezterm_pane_id.is_some()
        || otty_pane_id.is_some()
        // W6(brief 20260813-surface-readers):内层绑定从 surface 读。
        || matches!(session.surface, B::Cmux { .. })
        || matches!(session.surface, B::Wezterm { .. })
        || matches!(session.surface, B::Otty { .. })
        || matches!(session.surface, B::Herdr { .. })
    {
        return None;
    }
    let pane_key = match &session.surface {
        B::Orca { pane_key, .. } if !pane_key.is_empty() => Some(pane_key.as_str()),
        _ => None,
    };
    pane_key?;
    let worktree_id = match &session.surface {
        B::Orca { worktree_id, .. } => worktree_id.as_deref().filter(|s| !s.is_empty()),
        _ => None,
    };
    if worktree_id.is_some() {
        return None;
    }
    if is_new {
        Some("orca_session_start")
    } else if retry_eligible {
        Some("orca_retry")
    } else {
        None
    }
}

pub(crate) fn spawn_orca_capture(
    store: &Arc<Mutex<SessionStore>>,
    session: &Session,
    reason: &'static str,
) {
    let Some(pane_key) = (match &session.surface {
        crate::surface::SurfaceBinding::Orca { pane_key, .. } if !pane_key.is_empty() => {
            Some(pane_key.clone())
        }
        _ => None,
    }) else {
        return;
    };
    let task_id = session.task_id.clone();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let store = store.clone();
        crate::log_info!(
            "orca",
            "capture spawning task_id={} reason={}",
            task_id,
            reason
        );
        handle.spawn(async move {
            let captured = tokio::task::spawn_blocking(move || {
                lookup_worktree_id_for_pane_key(&pane_key).map(|worktree_id| {
                    let tab_id = parse_pane_key(&pane_key).map(|(tab, _)| tab.to_string());
                    (pane_key, worktree_id, tab_id)
                })
            })
            .await
            .ok()
            .flatten();
            match captured {
                Some((pane_key, worktree_id, tab_id)) => {
                    let written = {
                        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                        store.set_surface_binding(
                            &task_id,
                            crate::surface::SurfaceBinding::Orca {
                                pane_key: pane_key.clone(),
                                worktree_id: Some(worktree_id.clone()),
                                tab_id,
                            },
                        )
                    };
                    crate::log_info!(
                        "orca",
                        "captured pane_key={} worktree_id={} task_id={} reason={} written={}",
                        pane_key,
                        worktree_id,
                        task_id,
                        reason,
                        written,
                    );
                }
                None => {
                    crate::log_info!(
                        "orca",
                        "worktree capture returned None task_id={} reason={} (Orca not running / pane gone)",
                        task_id,
                        reason,
                    );
                }
            }
        });
    } else {
        crate::log_info!(
            "orca",
            "capture skipped (no tokio runtime) task_id={} reason={}",
            task_id,
            reason,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_terminal_row_deserializes_null_title() {
        let json = r#"{
            "handle": "term_x",
            "tabId": "tab-a",
            "leafId": "leaf-b",
            "worktreeId": "wt-1",
            "worktreePath": "/tmp/proj",
            "title": null,
            "connected": true,
            "lastOutputAt": null
        }"#;
        let row: OrcaTerminalRow = serde_json::from_str(json).expect("null title should parse");
        assert_eq!(row.title, "");
    }

    #[test]
    fn orca_terminal_list_envelope_deserializes_null_title_row() {
        let json = r#"{
            "ok": true,
            "result": {
                "terminals": [{
                    "handle": "term_x",
                    "tabId": "tab-a",
                    "leafId": "leaf-b",
                    "worktreeId": "wt-1",
                    "worktreePath": "/tmp/proj",
                    "title": null,
                    "connected": true,
                    "lastOutputAt": null
                }],
                "visualLayouts": []
            }
        }"#;
        let envelope: OrcaCliEnvelope<OrcaTerminalListResult> =
            serde_json::from_str(json).expect("envelope with null title should parse");
        assert!(envelope.ok);
        assert_eq!(envelope.result.unwrap().terminals[0].title, "");
    }

    #[test]
    fn parse_pane_key_splits_tab_and_leaf() {
        assert_eq!(parse_pane_key("abc:def"), Some(("abc", "def")));
    }

    #[test]
    fn parse_pane_key_rejects_missing_leaf() {
        assert_eq!(parse_pane_key("abc"), None);
        assert_eq!(parse_pane_key("abc:"), None);
    }

    #[test]
    fn find_terminal_by_pane_key_matches_row() {
        let rows = vec![OrcaTerminalRow {
            handle: "term_x".into(),
            tab_id: "tab-a".into(),
            leaf_id: "leaf-b".into(),
            worktree_id: "wt-1".into(),
            worktree_path: "/tmp/proj".into(),
            title: String::new(),
            connected: true,
            last_output_at: None,
        }];
        let found = find_terminal_by_pane_key(&rows, "tab-a:leaf-b");
        assert_eq!(found.map(|r| r.handle.as_str()), Some("term_x"));
    }

    #[test]
    fn active_pane_matches_layout_requires_active_tab_and_leaf() {
        let layouts = vec![OrcaVisualLayout {
            worktree_id: "wt-1".into(),
            root: OrcaLayoutRoot {
                active_tab_id: "tab-a".into(),
                tabs: vec![OrcaLayoutTab {
                    tab_id: "tab-a".into(),
                    active_leaf_id: "leaf-b".into(),
                }],
            },
        }];
        assert!(active_pane_matches_layout(&layouts, "wt-1", "tab-a:leaf-b"));
        assert!(!active_pane_matches_layout(
            &layouts,
            "wt-1",
            "tab-a:leaf-other"
        ));
        assert!(!active_pane_matches_layout(
            &layouts,
            "wt-other",
            "tab-a:leaf-b"
        ));
    }

    #[test]
    fn find_terminal_for_focus_falls_back_to_active_tab_sibling() {
        let terminals = vec![
            OrcaTerminalRow {
                handle: "term_old".into(),
                tab_id: "tab-a".into(),
                leaf_id: "leaf-old".into(),
                worktree_id: "wt-1".into(),
                worktree_path: "/tmp/proj".into(),
                title: "Cursor ready".into(),
                connected: true,
                last_output_at: Some(1),
            },
            OrcaTerminalRow {
                handle: "term_new".into(),
                tab_id: "tab-a".into(),
                leaf_id: "leaf-new".into(),
                worktree_id: "wt-1".into(),
                worktree_path: "/tmp/proj".into(),
                title: "Cursor Agent".into(),
                connected: true,
                last_output_at: Some(99),
            },
        ];
        let found = find_terminal_for_focus(&terminals, "tab-a:leaf-gone", Some("wt-1"));
        assert_eq!(found.map(|r| r.handle.as_str()), Some("term_new"));
    }

    #[test]
    fn find_terminal_for_focus_bails_when_all_candidates_lack_output_time() {
        let terminals = vec![
            OrcaTerminalRow {
                handle: "term_a".into(),
                tab_id: "tab-a".into(),
                leaf_id: "leaf-a".into(),
                worktree_id: "wt-1".into(),
                worktree_path: "/tmp/proj".into(),
                title: "shell a".into(),
                connected: true,
                last_output_at: None,
            },
            OrcaTerminalRow {
                handle: "term_b".into(),
                tab_id: "tab-a".into(),
                leaf_id: "leaf-b".into(),
                worktree_id: "wt-1".into(),
                worktree_path: "/tmp/proj".into(),
                title: "shell b".into(),
                connected: true,
                last_output_at: None,
            },
        ];
        // 精确 leaf 仍可命中
        assert!(find_terminal_for_focus(&terminals, "tab-a:leaf-a", Some("wt-1")).is_some());
        // 全无 last_output_at 时放弃 fallback,不选列表末位
        assert!(find_terminal_for_focus(&terminals, "tab-a:leaf-gone", Some("wt-1")).is_none());
    }

    #[test]
    fn should_clear_orca_binding_only_when_tab_has_no_connected_terminals() {
        let terminals = vec![OrcaTerminalRow {
            handle: "term_x".into(),
            tab_id: "tab-a".into(),
            leaf_id: "leaf-b".into(),
            worktree_id: "wt-1".into(),
            worktree_path: "/tmp/proj".into(),
            title: String::new(),
            connected: true,
            last_output_at: None,
        }];
        assert!(find_terminal_for_focus(&terminals, "tab-a:leaf-b", Some("wt-1")).is_some());
        assert!(find_terminal_for_focus(&terminals, "tab-a:leaf-missing", Some("wt-1")).is_some());
        assert!(find_terminal_for_focus(&terminals, "tab-z:leaf-missing", Some("wt-1")).is_none());
    }

    #[test]
    fn layout_active_tab_matches_hint_compares_active_tab_only() {
        let layouts = vec![OrcaVisualLayout {
            worktree_id: "wt-1".into(),
            root: OrcaLayoutRoot {
                active_tab_id: "tab-a".into(),
                tabs: vec![],
            },
        }];
        assert_eq!(
            layout_active_tab_matches_hint(&layouts, "wt-1", "tab-a"),
            Some(true)
        );
        assert_eq!(
            layout_active_tab_matches_hint(&layouts, "wt-1", "tab-b"),
            Some(false)
        );
        assert_eq!(
            layout_active_tab_matches_hint(&layouts, "wt-missing", "tab-a"),
            None
        );
    }

    /// 2026-08-07 回归护栏:Orca CLI 1.4.171 的真实 `terminal list
    /// --include-visual-layouts` 输出形状。layout 含 `panes` / `groupId` /
    /// `worktreePath` 等本代码不消费的字段,serde 必须忽略;`activeTabId` /
    /// `activeLeafId` 必须能解出来供 `active_pane_matches_layout` 判定。
    #[test]
    fn parses_real_orca_cli_visual_layout_shape() {
        let json = r#"{
            "ok": true,
            "result": {
                "terminals": [],
                "visualLayouts": [{
                    "worktreeId": "4b5488c4-78b8-4b66-9a63-830a65bdadae::/Users/edy/Documents/Kidder/BlackHole/poke-poke",
                    "worktreePath": "/Users/edy/Documents/Kidder/BlackHole/poke-poke",
                    "root": {
                        "type": "group",
                        "groupId": "185af91c-439b-46e2-9810-407eab54196e",
                        "activeTabId": "ba4174d0-c706-4850-b3f2-a054bfe2f113",
                        "tabs": [{
                            "tabId": "ba4174d0-c706-4850-b3f2-a054bfe2f113",
                            "title": "⠋ Pi",
                            "activeLeafId": "ebfa3a2f-200f-4388-a851-8efe7ae287cc",
                            "panes": {
                                "type": "pane-split",
                                "direction": "vertical",
                                "first": {"type": "terminal", "leafId": "ebfa3a2f-200f-4388-a851-8efe7ae287cc", "active": true}
                            }
                        }]
                    }
                }]
            }
        }"#;
        let envelope: OrcaCliEnvelope<OrcaTerminalListResult> =
            serde_json::from_str(json).expect("real CLI layout shape should parse");
        let result = envelope.result.expect("ok=true must carry result");
        assert_eq!(result.visual_layouts.len(), 1);
        let layout = &result.visual_layouts[0];
        assert_eq!(
            layout.root.active_tab_id,
            "ba4174d0-c706-4850-b3f2-a054bfe2f113"
        );
        assert_eq!(
            layout.root.tabs[0].active_leaf_id,
            "ebfa3a2f-200f-4388-a851-8efe7ae287cc"
        );
        assert!(active_pane_matches_layout(
            &result.visual_layouts,
            "4b5488c4-78b8-4b66-9a63-830a65bdadae::/Users/edy/Documents/Kidder/BlackHole/poke-poke",
            "ba4174d0-c706-4850-b3f2-a054bfe2f113:ebfa3a2f-200f-4388-a851-8efe7ae287cc",
        ));
    }

    /// 契约自诊断谓词:terminals 存在 + layouts 恒空 → broken(2026-08-07
    /// 1.4.175 回归时就是 24 terminals + 0 layouts)。
    #[test]
    fn visual_layouts_contract_broken_semantics() {
        assert!(
            is_visual_layouts_contract_broken(24, 0),
            "terminals 存在但 layouts 空 = 契约疑似变更"
        );
        assert!(
            !is_visual_layouts_contract_broken(0, 0),
            "无 terminals 不算 broken(可能只是 Orca 没 pane)"
        );
        assert!(
            !is_visual_layouts_contract_broken(24, 6),
            "layouts 正常返回不算 broken"
        );
        assert!(
            !is_visual_layouts_contract_broken(0, 6),
            "无 terminals 有 layouts 也不该告警"
        );
    }
}
