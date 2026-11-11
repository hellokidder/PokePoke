//! herdr-in-Orca:按 tab title + worktree 定位外层 Orca pane(无 AppleScript 字典)。

use super::{
    apply_orca_binding, focus_orca_terminal_handle, list_terminals, orca_bound_pane_focused,
    pane_key_for_row, OrcaTerminalRow,
};
use crate::sessions::SessionStore;
use std::sync::{Arc, Mutex};

fn is_herdr_tab_title(title: &str) -> bool {
    let t = title.trim();
    t == "herdr" || t.starts_with("herdr ") || t.starts_with("herdr\t")
}

fn paths_compatible(terminal_path: &str, workspace_path: &str) -> bool {
    if terminal_path.is_empty() || workspace_path.is_empty() {
        return false;
    }
    std::path::Path::new(terminal_path) == std::path::Path::new(workspace_path)
}

fn find_herdr_terminal_candidates<'a>(
    terminals: &'a [OrcaTerminalRow],
    workspace_path: Option<&str>,
) -> Vec<&'a OrcaTerminalRow> {
    let rows: Vec<_> = terminals
        .iter()
        .filter(|r| r.connected && is_herdr_tab_title(&r.title))
        .collect();
    if let Some(ws) = workspace_path.filter(|s| !s.is_empty()) {
        let scoped: Vec<_> = rows
            .iter()
            .copied()
            .filter(|r| paths_compatible(&r.worktree_path, ws))
            .collect();
        if !scoped.is_empty() {
            return scoped;
        }
    }
    rows
}

/// herdr UI client 跑在 Orca 里、hook 未带 `ORCA_PANE_KEY` 时,用 `terminal list`
/// 按 tab title + worktree path 定位外层 pane。
pub(crate) fn discover_orca_herdr_terminal(
    workspace_path: Option<&str>,
) -> Option<OrcaTerminalRow> {
    let list = list_terminals()?;
    let candidates = find_herdr_terminal_candidates(&list.terminals, workspace_path);
    match candidates.len() {
        1 => {
            let row = candidates[0].clone();
            crate::log_info!(
                "focus",
                "orca discover_herdr_terminal handle={} pane_key={} worktree={}",
                row.handle,
                pane_key_for_row(&row),
                row.worktree_path
            );
            Some(row)
        }
        n => {
            if n > 1 {
                crate::log_info!(
                    "focus",
                    "orca discover_herdr_terminal skip: {} candidates workspace={:?}",
                    n,
                    workspace_path
                );
            }
            None
        }
    }
}

/// popup / focus 抑制:Orca 前台且 layout 内 active pane 命中 herdr 宿主 tab。
pub(crate) fn orca_herdr_host_focused(workspace_path: Option<&str>) -> bool {
    let row = match discover_orca_herdr_terminal(workspace_path) {
        Some(row) => row,
        None => return false,
    };
    orca_bound_pane_focused(
        &pane_key_for_row(&row),
        Some(row.worktree_id.as_str()),
        Some(row.tab_id.as_str()),
    )
}

/// herdr-in-Orca 段 A:Orca **没有** Terminal.app 式 AppleScript(`windows`/`tabs`),
/// 实测 -1708。改 `orca terminal list` 找 title=herdr + worktree 唯一命中后 switch。
pub(crate) fn focus_orca_herdr_anchor_by_tty(
    client_tty_dev: &str,
    workspace_path: Option<&str>,
) -> bool {
    let Some(row) = discover_orca_herdr_terminal(workspace_path) else {
        crate::log_info!(
            "focus",
            "orca focus_orca_herdr_anchor tty={} workspace={:?} -> no unique herdr terminal",
            client_tty_dev,
            workspace_path
        );
        return false;
    };
    crate::log_info!(
        "focus",
        "orca focus_orca_herdr_anchor tty={} handle={} pane_key={}",
        client_tty_dev,
        row.handle,
        pane_key_for_row(&row)
    );
    focus_orca_terminal_handle(&row.handle)
}

/// notify 侧:herdr binding 已写入但 hook 未带 ORCA_PANE_KEY 时,从 Orca CLI 补外层 pane。
pub(crate) fn apply_orca_binding_for_herdr_host_if_missing(
    store: &Arc<Mutex<SessionStore>>,
    task_id: &str,
    workspace_path: Option<&str>,
    has_orca_pane_key: bool,
) {
    if has_orca_pane_key {
        return;
    }
    let Some(row) = discover_orca_herdr_terminal(workspace_path) else {
        return;
    };
    let pane_key = pane_key_for_row(&row);
    let tab_id = Some(row.tab_id.clone());
    apply_orca_binding(store, task_id, &pane_key, &row.worktree_id, tab_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_herdr_tab_title_matches_orca_tab_names() {
        assert!(is_herdr_tab_title("herdr"));
        assert!(is_herdr_tab_title("herdr --session work"));
        assert!(!is_herdr_tab_title("bash"));
    }

    #[test]
    fn find_herdr_terminal_candidates_scopes_by_workspace() {
        let rows = vec![
            OrcaTerminalRow {
                handle: "t1".into(),
                tab_id: "tab-a".into(),
                leaf_id: "leaf-a".into(),
                worktree_id: "wt-1".into(),
                worktree_path: "/tmp/proj-a".into(),
                title: "herdr".into(),
                connected: true,
                last_output_at: None,
            },
            OrcaTerminalRow {
                handle: "t2".into(),
                tab_id: "tab-b".into(),
                leaf_id: "leaf-b".into(),
                worktree_id: "wt-2".into(),
                worktree_path: "/tmp/proj-b".into(),
                title: "herdr".into(),
                connected: true,
                last_output_at: None,
            },
        ];
        let scoped = find_herdr_terminal_candidates(&rows, Some("/tmp/proj-b"));
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].handle, "t2");
    }
}
