//! cmux CLI 最小客户端：解析 `cmux --json tree --all`,按 title 定位 surface,
//! shell-out `cmux focus-panel`。只服务 herdr-in-cmux 回跳段 A 的 anchor 发现;
//! 已有 `cmux_surface_id` UUID binding 的 session 仍走 `focus_cmux_surface`。

use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::commands::focus::cli_timeout::{command_output_with_timeout, FOCUS_CLI_TIMEOUT};

/// cmux.app 自带 CLI(cmux 官方安装位;`$CMUX_BUNDLED_CLI_PATH` 指向的就是它)。
const MACOS_CMUX_CLI: &str = "/Applications/cmux.app/Contents/Resources/bin/cmux";
/// Homebrew 安装的 symlink 落点。
const HOMEBREW_CMUX_CLI: &str = "/opt/homebrew/bin/cmux";
const USR_LOCAL_CMUX_CLI: &str = "/usr/local/bin/cmux";

/// cmux CLI 绝对路径候选(按优先级)。
pub(crate) fn cmux_cli_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(MACOS_CMUX_CLI),
        PathBuf::from(HOMEBREW_CMUX_CLI),
        PathBuf::from(USR_LOCAL_CMUX_CLI),
    ];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/cmux"));
    }
    candidates
}

/// 解析 cmux CLI。GUI(Dock / launchd)启动的 PokePoke 进程 PATH 是极简的
/// `/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin`,拿不到 cmux.app 的 Resources/bin
/// 也拿不到 Homebrew 目录 —— 裸 `Command::new("cmux")` 会直接 ENOENT,表现为
/// `focus-panel -> false` + `tree` 秒失败(2026-10-31 实测:cmux + 任意 agent
/// 回跳全失效)。与 `wezterm.rs` / `orca.rs` / `otty.rs` 同思路给绝对路径候选,
/// 全都没命中才回退裸命令名(dev 从终端起服务时 PATH 里有 cmux)。
fn resolve_cmux_cli() -> PathBuf {
    for candidate in cmux_cli_candidates() {
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("cmux")
}

/// tree JSON 里定位到的一个 terminal surface + 所属 workspace(ref 或 UUID)。
/// `pane` 为 cmux split 里的 pane ref(如 `pane:18`);跨 workspace 回跳时
/// 仅 `focus-panel` 会切到正确 ws 但恢复该 ws 上次聚焦的 pane —— 需配合
/// `select-workspace` + `focus-pane` 才能落到 herdr 那一侧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmuxSurfaceTarget {
    pub panel: String,
    pub workspace: String,
    pub pane: Option<String>,
}

#[derive(Deserialize, Default)]
struct TreeActive {
    #[serde(default)]
    workspace_ref: String,
    #[serde(default)]
    pane_ref: String,
    #[serde(default)]
    surface_ref: String,
}

#[derive(Deserialize)]
pub(crate) struct TreeRoot {
    #[serde(default)]
    active: Option<TreeActive>,
    #[serde(default)]
    windows: Vec<TreeWindow>,
}

#[derive(Deserialize)]
struct TreeWindow {
    #[serde(default)]
    workspaces: Vec<TreeWorkspace>,
}

#[derive(Deserialize)]
struct TreeWorkspace {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    panes: Vec<TreePane>,
}

#[derive(Deserialize)]
struct TreePane {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    surfaces: Vec<TreeSurface>,
}

#[derive(Deserialize)]
struct TreeSurface {
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tty: Option<String>,
    #[serde(default)]
    pane_ref: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    surface_type: Option<String>,
}

/// cmux `tree --json` 里当前前台 terminal surface 的快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmuxFocusedTerminal {
    pub tty: Option<String>,
    pub surface_id: Option<String>,
    pub surface_ref: String,
}

fn normalize_tty(tty: &str) -> String {
    let t = tty.trim();
    t.strip_prefix("/dev/").unwrap_or(t).to_string()
}

/// 纯函数：从 tree JSON 里按 tty 找 terminal surface。tty 可带或不带 `/dev/` 前缀。
/// 0 / 多命中都返回 vec;caller 要求唯一才 focus。
pub(crate) fn find_surfaces_by_tty_from_tree(tree: &TreeRoot, tty: &str) -> Vec<CmuxSurfaceTarget> {
    let want = normalize_tty(tty);
    if want.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for window in &tree.windows {
        for ws in &window.workspaces {
            let workspace_key = ws
                .id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(ws.r#ref.as_str());
            if workspace_key.is_empty() {
                continue;
            }
            for pane in &ws.panes {
                for surface in &pane.surfaces {
                    if surface.surface_type.as_deref() != Some("terminal") {
                        continue;
                    }
                    let Some(surface_tty) = surface.tty.as_deref() else {
                        continue;
                    };
                    if normalize_tty(surface_tty) != want {
                        continue;
                    }
                    let panel_key = surface
                        .id
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(surface.r#ref.as_str());
                    if panel_key.is_empty() {
                        continue;
                    }
                    out.push(CmuxSurfaceTarget {
                        panel: panel_key.to_string(),
                        workspace: workspace_key.to_string(),
                        pane: surface
                            .pane_ref
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    });
                }
            }
        }
    }
    out
}

/// 纯函数：从 `cmux --json tree --all` 的 JSON 里找 title **以 `herdr` 开头**(2026-07-14:实测 herdr 0.7.3 写 `herdr --session work`,改前缀匹配)
/// 的 terminal surface。跟 Ghostty 段 A anchor 同口径 —— 只认 herdr 客户端
/// surface,不认 "CMUX Herdr Debug" 这类 cmux 原生 agent surface。
///
/// 返回 (panel, workspace) 列表;caller 要求 `len()==1` 才 focus,0 / 多命中都 skip。
pub(crate) fn find_herdr_client_surfaces_from_tree(tree: &TreeRoot) -> Vec<CmuxSurfaceTarget> {
    let mut out = Vec::new();
    for window in &tree.windows {
        for ws in &window.workspaces {
            let workspace_key = ws
                .id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(ws.r#ref.as_str());
            if workspace_key.is_empty() {
                continue;
            }
            for pane in &ws.panes {
                for surface in &pane.surfaces {
                    if surface.surface_type.as_deref() != Some("terminal") {
                        continue;
                    }
                    if !surface
                        .title
                        .as_deref()
                        .is_some_and(|t| t.starts_with("herdr"))
                    {
                        continue;
                    }
                    let panel_key = surface
                        .id
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(surface.r#ref.as_str());
                    if panel_key.is_empty() {
                        continue;
                    }
                    out.push(CmuxSurfaceTarget {
                        panel: panel_key.to_string(),
                        workspace: workspace_key.to_string(),
                        pane: surface
                            .pane_ref
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    });
                }
            }
        }
    }
    out
}

static LOAD_TREE_WARN_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// 已知安装位都没有命中 → 只剩 PATH 解析。GUI 启动的进程这条基本必挂,
/// 日志直说,别再被误读成 "cmux hang / tree timeout"。
fn warn_cli_not_found_throttled(cli: &std::path::Path) {
    if cli.is_absolute() {
        return;
    }
    warn_load_tree_throttled(
        "cmux CLI not found in /Applications/cmux.app/Contents/Resources/bin, \
         /opt/homebrew/bin, /usr/local/bin, ~/.local/bin; PATH fallback likely fails \
         for a GUI-launched app",
    );
}

fn warn_load_tree_throttled(detail: &str) {
    let mut guard = LOAD_TREE_WARN_AT.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    if guard.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60)) {
        crate::log_warn!("focus", "cmux load_tree_json failed: {}", detail);
        *guard = Some(now);
    }
}

fn load_tree_json() -> Option<TreeRoot> {
    // 2026-08-18:cmux app wedged / stale socket 时 `cmux --json tree --all` 会无限期
    // hang(brief 20260818-cmux-herdr-jumpback-fallback)。裸 `Command::output()` 会把
    // 聚焦链 + popup watcher 的 cmux probe 一起钉死;套 3s 超时按失败降级,段 A 由
    // activate_terminal_host_fallback 的 cmux app activate 兜底。
    let cli = resolve_cmux_cli();
    warn_cli_not_found_throttled(&cli);
    let output = match command_output_with_timeout(
        {
            let mut cmd = Command::new(&cli);
            cmd.args(["--json", "tree", "--all"]);
            cmd
        },
        FOCUS_CLI_TIMEOUT,
        "cmux-tree",
    ) {
        Some(o) => o,
        None => {
            warn_load_tree_throttled("cmux tree timeout");
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn_load_tree_throttled(&format!(
            "cmux tree exit={} stderr={}",
            output.status,
            stderr.trim()
        ));
        return None;
    }
    match serde_json::from_slice(&output.stdout) {
        Ok(tree) => Some(tree),
        Err(e) => {
            warn_load_tree_throttled(&format!("tree json parse err={e}"));
            None
        }
    }
}

pub(crate) fn active_workspace_ref(tree: &TreeRoot) -> Option<&str> {
    tree.active
        .as_ref()
        .map(|a| a.workspace_ref.as_str())
        .filter(|s| !s.is_empty())
}

/// 从 `cmux tree --json` 的 `active` 指针解析当前前台 terminal surface。
pub(crate) fn focused_terminal_from_tree(tree: &TreeRoot) -> Option<CmuxFocusedTerminal> {
    let active = tree.active.as_ref()?;
    let surface_ref = active.surface_ref.as_str();
    if surface_ref.is_empty() {
        return None;
    }
    let workspace_ref = active.workspace_ref.as_str();
    let pane_ref = active.pane_ref.as_str();
    for window in &tree.windows {
        for ws in &window.workspaces {
            if !workspace_ref.is_empty() && ws.r#ref != workspace_ref {
                continue;
            }
            for pane in &ws.panes {
                if !pane_ref.is_empty() && pane.r#ref != pane_ref {
                    continue;
                }
                for surface in &pane.surfaces {
                    if surface.r#ref != surface_ref {
                        continue;
                    }
                    if surface.surface_type.as_deref() != Some("terminal") {
                        return None;
                    }
                    return Some(CmuxFocusedTerminal {
                        tty: surface.tty.clone(),
                        surface_id: surface.id.clone(),
                        surface_ref: surface.r#ref.clone(),
                    });
                }
            }
        }
    }
    None
}

/// 纯函数:session 的 tty / surface UUID 是否匹配 tree 里当前前台 terminal。
pub(crate) fn cmux_terminal_focus_matches_tree(
    tree: &TreeRoot,
    tty: Option<&str>,
    surface_id: Option<&str>,
) -> bool {
    let Some(focused) = focused_terminal_from_tree(tree) else {
        return false;
    };
    if let Some(want_id) = surface_id.filter(|s| !s.is_empty()) {
        if focused
            .surface_id
            .as_deref()
            .is_some_and(|have| have.eq_ignore_ascii_case(want_id))
        {
            return true;
        }
    }
    match (
        tty.filter(|t| !t.is_empty()),
        focused.tty.as_deref().filter(|t| !t.is_empty()),
    ) {
        (Some(want), Some(have)) => normalize_tty(want) == normalize_tty(have),
        _ => false,
    }
}

/// 前台 app 为 cmux 时,比对 session tty / `cmux_surface_id` 与 tree active surface。
pub fn is_cmux_terminal_focused(tty: Option<&str>, surface_id: Option<&str>) -> bool {
    let Some(tree) = load_tree_json() else {
        return false;
    };
    cmux_terminal_focus_matches_tree(&tree, tty, surface_id)
}

/// 当前前台 workspace 与目标不同 → 跨 ws 回跳前必须先 `select-workspace`。
/// 否则 `focus-panel` 虽能切到目标 ws,却会恢复该 ws **上次**聚焦的 split pane
/// (herdr + 普通 tty 同 ws 时表现为落到普通 tty)。
pub(crate) fn needs_workspace_select_first(tree: &TreeRoot, target_workspace: &str) -> bool {
    match active_workspace_ref(tree) {
        Some(active) => active != target_workspace,
        None => true,
    }
}

fn cmux_cmd(args: &[&str]) -> bool {
    // 与 load_tree_json 同因加超时:cmux CLI 挂死时 focus-panel / select-workspace
    // 同样会无限阻塞(brief 20260818-cmux-herdr-jumpback-fallback)。
    let cli = resolve_cmux_cli();
    warn_cli_not_found_throttled(&cli);
    let output = command_output_with_timeout(
        {
            let mut cmd = Command::new(&cli);
            cmd.args(args);
            cmd
        },
        FOCUS_CLI_TIMEOUT,
        "cmux-cmd",
    );
    matches!(&output, Some(o) if o.status.success())
}

fn select_workspace(workspace: &str) -> bool {
    let ok = cmux_cmd(&["select-workspace", "--workspace", workspace]);
    crate::log_info!(
        "focus",
        "cmux select_workspace workspace={} -> {}",
        workspace,
        ok
    );
    ok
}

fn focus_pane_ref(pane: &str, workspace: Option<&str>) -> bool {
    let mut args = vec!["focus-pane", "--pane", pane];
    if let Some(ws) = workspace.filter(|s| !s.is_empty()) {
        args.push("--workspace");
        args.push(ws);
    }
    let ok = cmux_cmd(&args);
    crate::log_info!(
        "focus",
        "cmux focus_pane pane={} workspace={} -> {}",
        pane,
        workspace.unwrap_or("<none>"),
        ok
    );
    ok
}

fn focus_panel_ref(panel: &str, workspace: Option<&str>) -> bool {
    let mut args = vec!["focus-panel", "--panel", panel];
    if let Some(ws) = workspace.filter(|s| !s.is_empty()) {
        args.push("--workspace");
        args.push(ws);
    }
    let ok = cmux_cmd(&args);
    crate::log_info!(
        "focus",
        "cmux focus_panel panel={} workspace={} -> {}",
        panel,
        workspace.unwrap_or("<none>"),
        ok
    );
    ok
}

pub(crate) fn activate_cmux_app() -> bool {
    let activated = crate::osascript::run_bool(
        r#"tell application id "com.cmuxterm.app" to activate
return true"#,
        "focus",
        "cmux_activate",
    );
    crate::log_info!("focus", "cmux focus activate cmux app -> {}", activated);
    activated
}

/// 按 tree 解析结果聚焦 surface:跨 ws 时先 `select-workspace`,有 pane ref 时
/// 优先 `focus-pane`(split 布局可靠),否则 `focus-panel`。
pub fn focus_surface_target(target: &CmuxSurfaceTarget) -> bool {
    let ws = target.workspace.as_str();
    let tree = load_tree_json();
    if tree
        .as_ref()
        .is_none_or(|t| needs_workspace_select_first(t, ws))
    {
        let _ = select_workspace(ws);
    }
    let focused = if let Some(pane) = target.pane.as_deref() {
        focus_pane_ref(pane, Some(ws)) || focus_panel_ref(&target.panel, Some(ws))
    } else {
        focus_panel_ref(&target.panel, Some(ws))
    };
    if focused {
        let _ = activate_cmux_app();
    }
    focused
}

/// 只读:cmux tree 里唯一 title 以 "herdr" 开头的 surface 是否为当前 active surface。
pub fn is_cmux_herdr_client_anchor_focused() -> bool {
    let matches = find_herdr_client_surfaces();
    if matches.len() != 1 {
        return false;
    }
    let Some(tree) = load_tree_json() else {
        return false;
    };
    let want_panel = &matches[0].panel;
    let Some(active) = tree.active.as_ref() else {
        return false;
    };
    !want_panel.is_empty() && active.surface_ref == *want_panel
}

/// 枚举当前 cmux 实例里 title=="herdr" 的客户端 surface。cmux 未装 / 未跑 /
/// CLI 失败 → 空 vec。
pub fn find_herdr_client_surfaces() -> Vec<CmuxSurfaceTarget> {
    load_tree_json()
        .map(|t| find_herdr_client_surfaces_from_tree(&t))
        .unwrap_or_default()
}

/// `cmux focus-panel --panel <id|ref> [--workspace <id|ref>]` + activate cmux app。
/// panel/workspace 可以是 UUID 或 `surface:N` / `workspace:N` ref。
///
/// 跨 workspace 时会在 focus 前补 `select-workspace`,避免 cmux 只切 ws、
/// 却恢复该 ws 上次聚焦的 split pane。
pub fn focus_panel(panel: &str, workspace: Option<&str>) -> bool {
    let Some(ws) = workspace.filter(|s| !s.is_empty()) else {
        let focused = focus_panel_ref(panel, None);
        if focused {
            let _ = activate_cmux_app();
        }
        return focused;
    };
    if let Some(tree) = load_tree_json() {
        if needs_workspace_select_first(&tree, ws) {
            let _ = select_workspace(ws);
        }
    } else {
        let _ = select_workspace(ws);
    }
    let focused = focus_panel_ref(panel, Some(ws));
    if focused {
        let _ = activate_cmux_app();
    }
    focused
}

/// 按 tty 在 cmux tree 里找 terminal surface。cmux 未装 / 未跑 / 无唯一命中 → 空。
pub fn find_surfaces_by_tty(tty: &str) -> Vec<CmuxSurfaceTarget> {
    load_tree_json()
        .map(|t| find_surfaces_by_tty_from_tree(&t, tty))
        .unwrap_or_default()
}

/// session-scoped 段 A：已知 herdr server socket → 定位该 session 的 UI client tty
/// → cmux tree 按 tty 找 surface → focus-panel。
pub fn focus_herdr_client_anchor_for_tty(client_tty: &str) -> bool {
    let matches = find_surfaces_by_tty(client_tty);
    match matches.len() {
        1 => {
            let t = &matches[0];
            let ok = focus_surface_target(t);
            crate::log_info!(
                "focus",
                "focus_cmux_herdr_anchor_by_tty tty={} panel={} pane={:?} workspace={} -> {}",
                client_tty,
                t.panel,
                t.pane,
                t.workspace,
                ok
            );
            ok
        }
        n => {
            crate::log_info!(
                "focus",
                "focus_cmux_herdr_anchor_by_tty skip: tty={} matched {} surfaces (need exactly 1)",
                client_tty,
                n
            );
            false
        }
    }
}

/// cmux 里跑 herdr 客户端时的段 A anchor:tree 里唯一 title 以 "herdr" 开头 的 surface
/// → focus-panel + activate。0 / 多命中返 false,交给上层继续降级。
pub fn focus_herdr_client_anchor() -> bool {
    let matches = find_herdr_client_surfaces();
    match matches.len() {
        1 => {
            let t = &matches[0];
            let ok = focus_surface_target(t);
            crate::log_info!(
                "focus",
                "focus_cmux_herdr_anchor panel={} pane={:?} workspace={} -> {}",
                t.panel,
                t.pane,
                t.workspace,
                ok
            );
            ok
        }
        n => {
            crate::log_info!(
                "focus",
                "focus_cmux_herdr_anchor skip: found {} surfaces titled \"herdr\" (need exactly 1)",
                n
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmux_cli_candidates_prefers_app_bundle_then_homebrew() {
        // brief 20261031-cmux-cli-path-resolve:GUI 启动的进程 PATH 里没有 cmux,
        // 必须靠绝对路径候选。顺序 = cmux.app 自带 CLI -> Homebrew -> /usr/local/bin。
        let candidates = cmux_cli_candidates();
        assert!(candidates.len() >= 3);
        assert_eq!(
            candidates[0],
            PathBuf::from("/Applications/cmux.app/Contents/Resources/bin/cmux")
        );
        assert_eq!(candidates[1], PathBuf::from("/opt/homebrew/bin/cmux"));
        assert_eq!(candidates[2], PathBuf::from("/usr/local/bin/cmux"));
        assert!(candidates.iter().all(|p| p.is_absolute()));
    }

    #[test]
    fn focused_terminal_from_tree_reads_active_surface_tty() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{
            "active": {
                "workspace_ref": "workspace:3",
                "pane_ref": "pane:8",
                "surface_ref": "surface:10"
            },
            "windows": [{
                "workspaces": [{
                    "ref": "workspace:3",
                    "panes": [{
                        "ref": "pane:8",
                        "surfaces": [{
                            "ref": "surface:10",
                            "id": "ED15CEBF-44D6-4B03-B7CE-851393933ADE",
                            "type": "terminal",
                            "tty": "ttys030",
                            "pane_ref": "pane:8"
                        }]
                    }]
                }]
            }]
        }"#,
        )
        .unwrap();
        let focused = focused_terminal_from_tree(&tree).unwrap();
        assert_eq!(focused.tty.as_deref(), Some("ttys030"));
        assert_eq!(
            focused.surface_id.as_deref(),
            Some("ED15CEBF-44D6-4B03-B7CE-851393933ADE")
        );
        assert!(cmux_terminal_focus_matches_tree(
            &tree,
            Some("/dev/ttys030"),
            None
        ));
        assert!(cmux_terminal_focus_matches_tree(
            &tree,
            None,
            Some("ED15CEBF-44D6-4B03-B7CE-851393933ADE")
        ));
        assert!(!cmux_terminal_focus_matches_tree(
            &tree,
            Some("/dev/ttys099"),
            None
        ));
    }

    #[test]
    fn find_herdr_client_surfaces_exact_title_only() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{
            "windows": [{
                "workspaces": [{
                    "ref": "workspace:3",
                    "id": "WS-UUID-3",
                    "panes": [{
                        "surfaces": [{
                            "ref": "surface:18",
                            "id": "SURF-18",
                            "title": "CMUX Herdr Debug - working",
                            "type": "terminal"
                        }]
                    }]
                }, {
                    "ref": "workspace:7",
                    "id": "WS-UUID-7",
                    "panes": [{
                        "surfaces": [{
                            "ref": "surface:21",
                            "id": "SURF-21",
                            "title": "herdr",
                            "type": "terminal",
                            "pane_ref": "pane:18"
                        }]
                    }]
                }]
            }]
        }"#,
        )
        .unwrap();
        let got = find_herdr_client_surfaces_from_tree(&tree);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].panel, "SURF-21");
        assert_eq!(got[0].workspace, "WS-UUID-7");
        assert_eq!(got[0].pane.as_deref(), Some("pane:18"));
    }

    #[test]
    fn find_herdr_client_surfaces_uses_ref_when_id_missing() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{"windows":[{"workspaces":[{"ref":"workspace:7","panes":[{"surfaces":[{"ref":"surface:21","title":"herdr","type":"terminal"}]}]}]}]}"#,
        )
        .unwrap();
        let got = find_herdr_client_surfaces_from_tree(&tree);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].panel, "surface:21");
        assert_eq!(got[0].workspace, "workspace:7");
    }

    #[test]
    fn find_herdr_client_surfaces_matches_session_param_title() {
        // 2026-07-14 根因:herdr 0.7.3 把 title 写成 `herdr --session work`(带
        // session 参数),不是字面 "herdr"。前缀匹配 starts_with("herdr") 必须命中。
        let tree: TreeRoot = serde_json::from_str(
            r#"{"windows":[{"workspaces":[{"ref":"workspace:7","id":"WS-7","panes":[{"surfaces":[{"ref":"surface:21","id":"SURF-21","title":"herdr --session work","type":"terminal","pane_ref":"pane:18"}]}]}]}]}"#,
        )
        .unwrap();
        let got = find_herdr_client_surfaces_from_tree(&tree);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].panel, "SURF-21");
        assert_eq!(got[0].workspace, "WS-7");
        assert_eq!(got[0].pane.as_deref(), Some("pane:18"));
    }

    #[test]
    fn find_herdr_client_surfaces_skips_browser_and_wrong_title() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{"windows":[{"workspaces":[{"ref":"workspace:1","panes":[{"surfaces":[
                {"ref":"surface:1","title":"herdr","type":"browser"},
                {"ref":"surface:2","title":"Herdr","type":"terminal"}
            ]}]}]}]}"#,
        )
        .unwrap();
        assert!(find_herdr_client_surfaces_from_tree(&tree).is_empty());
    }

    #[test]
    fn find_surfaces_by_tty_matches_with_or_without_dev_prefix() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{"windows":[{"workspaces":[{"ref":"workspace:7","id":"WS-7","panes":[{"surfaces":[
                {"ref":"surface:21","id":"SURF-21","title":"herdr","type":"terminal","tty":"ttys000","pane_ref":"pane:18"},
                {"ref":"surface:22","id":"SURF-22","title":"other","type":"terminal","tty":"ttys001"}
            ]}]}]}]}"#,
        )
        .unwrap();
        let got = find_surfaces_by_tty_from_tree(&tree, "/dev/ttys000");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].panel, "SURF-21");
        assert_eq!(got[0].workspace, "WS-7");
        assert_eq!(got[0].pane.as_deref(), Some("pane:18"));
        let got2 = find_surfaces_by_tty_from_tree(&tree, "ttys000");
        assert_eq!(got2, got);
    }

    #[test]
    fn find_surfaces_by_tty_skips_non_terminal() {
        let tree: TreeRoot = serde_json::from_str(
            r#"{"windows":[{"workspaces":[{"ref":"workspace:1","panes":[{"surfaces":[
                {"ref":"surface:1","title":"x","type":"browser","tty":"ttys000"}
            ]}]}]}]}"#,
        )
        .unwrap();
        assert!(find_surfaces_by_tty_from_tree(&tree, "ttys000").is_empty());
    }

    #[test]
    fn needs_workspace_select_first_when_active_differs() {
        let tree: TreeRoot =
            serde_json::from_str(r#"{"active":{"workspace_ref":"workspace:5"},"windows":[]}"#)
                .unwrap();
        assert!(needs_workspace_select_first(&tree, "workspace:7"));
        assert!(!needs_workspace_select_first(&tree, "workspace:5"));
    }

    #[test]
    fn active_workspace_ref_reads_tree_active() {
        let tree: TreeRoot =
            serde_json::from_str(r#"{"active":{"workspace_ref":"workspace:7"},"windows":[]}"#)
                .unwrap();
        assert_eq!(active_workspace_ref(&tree), Some("workspace:7"));
    }
}
