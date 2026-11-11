//! Otty 终端 CLient:解析 `otty pane list --format json`,shell-out 聚焦。
//! 直接跑在 Otty pane 里的 CC 回跳;herdr / cmux 内层场景由各自 short-circuit 优先。

#[path = "otty_applescript.rs"]
mod otty_applescript;
pub(crate) use otty_applescript::{focus_otty_herdr_anchor_by_tty, otty_selected_tab_has_tty};
use otty_applescript::{focus_otty_window_applescript, selected_tab_is};

use super::cli_timeout::{command_output_with_timeout, FOCUS_CLI_TIMEOUT};
use crate::sessions::{Session, SessionStore};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// otty CLI 路径。`brew install --cask otty` 装到 /Applications,CLI 二进制是
/// `otty-cli`(不是 `otty`,也不在 PATH),位于 app bundle 内。找不到再 PATH 兜底。
const APP_OTTY_CLI: &str = "/Applications/Otty.app/Contents/MacOS/otty-cli";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct OttyPane {
    pub id: String,
    pub tab_id: String,
    pub window_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub process: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OttyPaneListResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    data: Vec<OttyPane>,
}

fn resolve_otty_cli() -> PathBuf {
    if Path::new(APP_OTTY_CLI).is_file() {
        return PathBuf::from(APP_OTTY_CLI);
    }
    // PATH 兜底(官方 CLI 名为 otty-cli)
    PathBuf::from("otty-cli")
}

fn list_panes() -> Option<Vec<OttyPane>> {
    let cli = resolve_otty_cli();
    let mut cmd = Command::new(&cli);
    cmd.args(["pane", "list", "--json"]);
    let output = command_output_with_timeout(cmd, FOCUS_CLI_TIMEOUT, "otty/pane-list")?;
    if !output.status.success() {
        crate::log_info!(
            "focus",
            "otty pane list cli={} exit={} stderr={}",
            cli.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    let resp: OttyPaneListResponse = serde_json::from_slice(&output.stdout).ok()?;
    if !resp.ok {
        crate::log_info!("focus", "otty pane list returned ok=false");
        return None;
    }
    Some(resp.data)
}

/// cwd 是否与 workspace 兼容(pane cwd 在 workspace 内,或恰好匹配)。
fn cwd_compatible(pane_cwd: &str, workspace_path: &str) -> bool {
    let workspace = workspace_path.trim();
    if workspace.is_empty() {
        return true;
    }
    let pane = pane_cwd.trim();
    if pane.is_empty() {
        return false;
    }
    let workspace_path = Path::new(workspace);
    let pane_path = Path::new(pane);
    pane_path == workspace_path || pane_path.starts_with(workspace_path)
}

/// 在 list 结果里按 workspace cwd 唯一匹配 pane。0 / 多命中返回 None。
fn find_unique_pane_by_workspace<'a>(
    panes: &'a [OttyPane],
    workspace_path: &str,
) -> Option<&'a OttyPane> {
    let ws = workspace_path.trim();
    if ws.is_empty() {
        return None;
    }
    let matches: Vec<_> = panes
        .iter()
        .filter(|p| cwd_compatible(&p.cwd, ws))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

/// pane_id 规范化:剥掉 `p_` 前缀,返回规范(无前缀)形。
///
/// `OTTY_PANE_ID` env 是无前缀形(`19fa6ca625c_15`),但 `otty pane list/show`
/// 返回的 `id` 带 `p_` 前缀(`p_19fa6ca625c_15`)。两者直接比较恒不匹配,会导致:
/// focus 判 "not found in list"、`pane show` 的 `d.id == pane_id` 滤掉结果、
/// `pane_exists` 误判 pane 消失而清掉有效 binding(实测 verified,见 log)。
/// 所有 pane_id 比较都必须先过本函数对齐。
fn canonical_pane_id(id: &str) -> &str {
    id.strip_prefix("p_").unwrap_or(id)
}

/// pane 是否仍然存在(用于 focus 失败后判断是否应清 binding)。
fn pane_exists(pane_id: &str) -> bool {
    list_panes().is_some_and(|panes| {
        panes
            .iter()
            .any(|p| canonical_pane_id(&p.id) == canonical_pane_id(pane_id))
    })
}

/// focus 失败后是否应清 otty binding:仅 pane 从 list 消失才算 stale。
pub(crate) fn should_clear_otty_binding_after_focus_fail(pane_id: &str) -> bool {
    !pane_exists(pane_id)
}

pub(super) fn clear_stale_otty_binding(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    pane_id: &str,
) {
    let cleared = store
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear_surface_binding_if_matches(
            session_id,
            &crate::surface::SurfaceBinding::Otty {
                pane_id: pane_id.to_string(),
                tab_id: None,
                window_id: None,
            },
        );
    crate::log_info!(
        "focus",
        "cleared stale otty_binding session_id={} pane_id={} cleared={}",
        session_id,
        pane_id,
        cleared
    );
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// server 侧 cwd 补 capture:workspace 唯一命中才返回 (pane_id, tab_id, window_id)。
pub(crate) fn capture_otty_pane_by_workspace(
    workspace_path: &str,
) -> Option<(String, String, Option<String>)> {
    let panes = list_panes()?;
    let row = find_unique_pane_by_workspace(&panes, workspace_path)?;
    // 存储用规范(无 p_ 前缀)形,与 hook 上报的 OTTY_PANE_ID 一致。
    Some((
        canonical_pane_id(&row.id).to_string(),
        row.tab_id.clone(),
        non_empty_string(&row.window_id),
    ))
}

/// 已知 pane_id 时 `pane show` 取 tab_id + window_id(聚焦抑制多窗口场景)。
pub(crate) fn lookup_pane_binding(pane_id: &str) -> Option<(String, Option<String>)> {
    let cli = resolve_otty_cli();
    let mut cmd = Command::new(&cli);
    cmd.args(["pane", "show", "--json", "--pane", pane_id]);
    let output = command_output_with_timeout(cmd, FOCUS_CLI_TIMEOUT, "otty/pane-show")?;
    if !output.status.success() {
        crate::log_info!(
            "focus",
            "otty pane show cli={} pane_id={} exit={} stderr={}",
            cli.display(),
            pane_id,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }
    #[derive(Deserialize)]
    struct PaneShowResp {
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        data: Option<OttyPane>,
    }
    let resp: PaneShowResp = serde_json::from_slice(&output.stdout).ok()?;
    if !resp.ok {
        crate::log_info!("focus", "otty pane show ok=false pane_id={}", pane_id);
        return None;
    }
    resp.data
        .filter(|d| canonical_pane_id(&d.id) == canonical_pane_id(pane_id))
        .and_then(|d| {
            non_empty_string(&d.tab_id).map(|tab_id| (tab_id, non_empty_string(&d.window_id)))
        })
}

/// 聚焦抑制:用户是否正看着 bound pane。
///
/// 两步:
/// 1. bound pane 的 tab 必须是前窗口当前 selected tab(AppleScript `id of selected
///    tab of front window`)。**不能**单看 `pane list` 的 `active` 字段判聚焦 --
///    它是"tab 内 active pane",多 tab 时每个 tab 都有一个,会把"用户在别的 tab"
///    误判成聚焦、漏弹 popup。
/// 2. split pane 场景:tab 对了,但 tab 内 active pane 是另一个 split 时,用户并没
///    在看 bound pane -> 不抑制。第 1 步已确认 bound pane 的 tab 是 selected tab,
///    此时 `active` 字段(该 tab 内 active pane)才可靠:bound pane active 才算看着。
///
/// `tab_id` / `window_id` 优先用 session 已存的;缺则 `pane show` 现场反查。
/// 第 2 步若 `pane list` CLI 失败,返回 false(不抑制)-> 宁可弹 popup 不漏。
pub(crate) fn otty_bound_pane_focused(
    pane_id: &str,
    tab_id: Option<&str>,
    window_id: Option<&str>,
) -> bool {
    let (bound_tab, bound_window) = match tab_id.filter(|tab_id| !tab_id.is_empty()) {
        Some(tab_id) => (tab_id.to_string(), window_id.and_then(non_empty_string)),
        None => match lookup_pane_binding(pane_id) {
            Some((tab_id, window_id)) => (tab_id, window_id),
            None => return false,
        },
    };
    let applescript_id = bound_tab.strip_prefix("t_").unwrap_or(&bound_tab);
    if !selected_tab_is(applescript_id, bound_window.as_deref().or(window_id)) {
        return false;
    }
    bound_pane_is_active(pane_id)
}

/// bound pane 是否是其所在 tab 的 active pane(第 2 步,在 selected tab 已确认后)。
fn bound_pane_is_active(pane_id: &str) -> bool {
    list_panes().is_some_and(|panes| {
        panes
            .iter()
            .any(|p| canonical_pane_id(&p.id) == canonical_pane_id(pane_id) && p.active)
    })
}

/// 哨兵差分诊断:对比"bound tab"与"front window 实际 selected tab"。
///
/// 返回 `Some((bound, actual))` 表示两个信息源都拿到(均不带 `t_` 前缀):
/// - `bound == actual`:用户在 bound tab,但聚焦判定却在 false —— 判定契约
///   损坏(2026-08-10 事故形态,watcher 哨兵据此打 WARN);
/// - `bound != actual`:用户确实在别的 tab,连续 false 属正常(INFO)。
/// 任一信息源拿不到(CLI 失败 / osascript 失败 / 无窗口)返回 None。
pub(crate) fn diagnose_focus_mismatch(
    pane_id: &str,
    tab_id: Option<&str>,
) -> Option<(String, String)> {
    let bound = tab_id
        .filter(|tab_id| !tab_id.is_empty())
        .map(str::to_string)
        .or_else(|| lookup_pane_binding(pane_id).map(|(tab_id, _)| tab_id))?;
    let bound = bound.strip_prefix("t_").unwrap_or(&bound).to_string();
    let actual = otty_applescript::front_selected_tab_id()?;
    Some((bound, actual))
}

/// 聚焦 Otty 中指定 pane_id 的 pane。
///
/// 优先 `otty pane focus --pane <pane_id>`:一次 CLI 调用同时切到指定 pane、
/// 选中其所在 tab、并把 Otty 激活到前台(实测 verified)。解决了 split pane 场景下
/// "选对了 tab 但没切到目标 pane"的问题(旧实现只 `set selected of tab` 选 tab,
/// 不改 tab 内 active pane)。CLI 接受带/不带 `p_` 前缀的 pane_id(hook 上报的
/// OTTY_PANE_ID 是无前缀形)。
///
/// `pane focus` 失败(pane 已关 / CLI 不可用)时回落到 tab 级 AppleScript
/// (按 tab_id 选中 tab + activate),至少把用户带到正确 tab。
pub(crate) fn focus_otty_pane(
    pane_id: &str,
    tab_id: Option<&str>,
    workspace_path: Option<&str>,
) -> bool {
    if focus_otty_pane_via_cli(pane_id) {
        return true;
    }
    // 回落:pane focus 失败。按 tab_id 选 tab(没 tab_id 则从 pane list 反查 + cwd 校验)。
    let target_tab = if let Some(tid) = tab_id.filter(|t| !t.is_empty()) {
        tid.to_string()
    } else {
        // 没有存储的 tab_id,尝试从 pane list 获取
        let panes = match list_panes() {
            Some(p) => p,
            None => return false,
        };
        let Some(pane) = panes
            .iter()
            .find(|p| canonical_pane_id(&p.id) == canonical_pane_id(pane_id))
        else {
            crate::log_info!(
                "focus",
                "otty focus_otty_pane pane_id={} not found in list",
                pane_id
            );
            return false;
        };
        // cwd 校验(如果提供):防止 pane_id 复用误跳
        if let Some(ws) = workspace_path.filter(|w| !w.is_empty()) {
            if !cwd_compatible(&pane.cwd, ws) {
                crate::log_info!(
                    "focus",
                    "otty focus_otty_pane pane_id={} cwd mismatch pane_cwd={} workspace={}",
                    pane_id,
                    pane.cwd,
                    ws
                );
                return false;
            }
        }
        pane.tab_id.clone()
    };

    focus_otty_window_applescript(&target_tab)
}

/// `otty pane focus --pane <pane_id>`:切到指定 pane + 选中其 tab + 激活 Otty 前台。
fn focus_otty_pane_via_cli(pane_id: &str) -> bool {
    let cli = resolve_otty_cli();
    let mut cmd = Command::new(&cli);
    cmd.args(["pane", "focus", "--pane", pane_id]);
    let output = command_output_with_timeout(cmd, FOCUS_CLI_TIMEOUT, "otty/pane-focus");
    match output {
        Some(o) if o.status.success() => {
            crate::log_info!(
                "focus",
                "otty pane focus pane_id={} ok=true (pane+tab+activate)",
                pane_id
            );
            true
        }
        Some(o) => {
            crate::log_info!(
                "focus",
                "otty pane focus pane_id={} exit={} stderr={}",
                pane_id,
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            );
            false
        }
        None => {
            crate::log_info!("focus", "otty pane focus pane_id={} timed out", pane_id);
            false
        }
    }
}

// ==================== server-side capture + binding ====================
// 从 http_server 抽到 mod(AGENTS.md §4a:新终端适配用 mod 目录,不往大文件续)。

/// notify 收到 `otty_pane_id` 时写 binding(pane_id + 空 tab_id,后者由 capture 补全)。
/// herdr / cmux / wezterm 优先(在 caller 的 else-if 链里)。
pub(crate) fn apply_otty_binding(store: &Arc<Mutex<SessionStore>>, task_id: &str, pane_id: &str) {
    let changed = {
        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
        // 原 set_otty_binding 的 tab_id 是必填 String:notify 路径传空串表示
        // "缺 tab_id,待 capture 补全";镜像同步把 None → 空串,形状不变。
        store.set_surface_binding(
            task_id,
            crate::surface::SurfaceBinding::Otty {
                pane_id: pane_id.to_string(),
                tab_id: None,
                window_id: None,
            },
        )
    };
    if changed {
        crate::log_info!(
            "notify",
            "set_surface_binding otty task_id={} pane_id={}",
            task_id,
            pane_id
        );
    }
}

/// Otty capture 触发决策。
///
/// 触发条件:herdr/cmux/wezterm binding 都不存在(otty 优先级最低)+ 有
/// workspace_path + binding 缺 tab_id(补全)或缺 pane_id(发现)+ 是新 session
/// 或 retry-eligible event。`retry_eligible` 由 caller 从 NotifyEventType 算
/// (Running/Pending/Stop),避免本模块依赖 http_server 私有的 NotifyEventType。
pub(crate) fn decide_otty_capture_reason(
    session: &Session,
    herdr_pane_id: Option<&str>,
    cmux_surface_id: Option<&str>,
    wezterm_pane_id: Option<&str>,
    orca_pane_key: Option<&str>,
    is_new: bool,
    retry_eligible: bool,
) -> Option<&'static str> {
    use crate::surface::SurfaceBinding as B;
    if herdr_pane_id.is_some()
        || cmux_surface_id.is_some()
        || wezterm_pane_id.is_some()
        || orca_pane_key.is_some()
        // W6(brief 20260813-surface-readers):内层绑定从 surface 读(herdr 走
        // 共存特例 helper,兼容 surface=Orca 时 herdr 在镜像)。
        || B::herdr_fields(session).is_some()
        || matches!(session.surface, B::Cmux { .. })
        || matches!(session.surface, B::Wezterm { .. })
        || matches!(session.surface, B::Orca { .. })
    {
        return None;
    }
    // 幂等:已有完整 binding (pane_id + tab_id) 不重 capture
    if let B::Otty {
        tab_id: Some(tab), ..
    } = &session.surface
    {
        if !tab.is_empty() {
            return None;
        }
    }
    // 需要 workspace 做 cwd 匹配(发现路径)
    if session
        .workspace_path
        .as_deref()
        .is_none_or(|w| w.is_empty())
    {
        return None;
    }
    if is_new {
        Some("otty_session_start")
    } else if retry_eligible {
        Some("otty_retry")
    } else {
        None
    }
}

/// Otty capture:已知 pane_id 时 `pane show` 取 tab_id;否则 `pane list` 按 cwd
/// 唯一匹配。写入 binding(pane_id + tab_id)供 focus 精准跳转。
pub(crate) fn spawn_otty_capture(
    store: &Arc<Mutex<SessionStore>>,
    session: &Session,
    reason: &'static str,
) {
    // 已知 pane_id(hook 上报 OTTY_PANE_ID)时直接 `pane show` 精确取 tab_id,
    // 不走 workspace cwd 唯一匹配 -- 多 pane 同目录 cwd 匹配歧义返回 None,
    // 导致 tab_id 永远补不上、聚焦抑制不生效(实测 verified)。
    let known_pane_id = match &session.surface {
        crate::surface::SurfaceBinding::Otty { pane_id, .. } if !pane_id.is_empty() => {
            Some(pane_id.clone())
        }
        _ => None,
    };
    let workspace = session.workspace_path.clone().filter(|w| !w.is_empty());
    if known_pane_id.is_none() && workspace.is_none() {
        return;
    }
    let task_id = session.task_id.clone();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let store = store.clone();
        crate::log_info!(
            "otty",
            "capture spawning task_id={} reason={}",
            task_id,
            reason
        );
        handle.spawn(async move {
            let captured = tokio::task::spawn_blocking(move || {
                if let Some(pane_id) = known_pane_id.as_deref() {
                    lookup_pane_binding(pane_id)
                        .map(|(tab_id, window_id)| (pane_id.to_string(), tab_id, window_id))
                } else {
                    capture_otty_pane_by_workspace(workspace.as_deref().unwrap_or(""))
                }
            })
            .await
            .ok()
            .flatten();
            match captured {
                Some((pane_id, tab_id, window_id)) => {
                    let written = {
                        let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                        store.set_surface_binding(
                            &task_id,
                            crate::surface::SurfaceBinding::Otty {
                                pane_id: pane_id.clone(),
                                tab_id: Some(tab_id.clone()),
                                window_id: window_id.clone(),
                            },
                        )
                    };
                    crate::log_info!(
                        "otty",
                        "captured pane_id={} tab_id={} window_id={:?} task_id={} reason={} written={}",
                        pane_id,
                        tab_id,
                        window_id,
                        task_id,
                        reason,
                        written,
                    );
                }
                None => {
                    crate::log_info!(
                        "otty",
                        "pane capture returned None task_id={} reason={} (Otty not running / cwd ambiguous / CLI failed)",
                        task_id,
                        reason,
                    );
                }
            }
        });
    } else {
        crate::log_info!(
            "otty",
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
    fn canonical_pane_id_strips_p_prefix() {
        assert_eq!(canonical_pane_id("p_19fa6ca625c_15"), "19fa6ca625c_15");
    }

    #[test]
    fn canonical_pane_id_keeps_unprefixed() {
        // OTTY_PANE_ID env 形态(无 p_ 前缀)应原样返回
        assert_eq!(canonical_pane_id("19fa6ca625c_15"), "19fa6ca625c_15");
    }

    #[test]
    fn canonical_pane_id_env_matches_cli_form() {
        // hook 上报(env,无前缀)与 CLI pane list/show(带 p_ 前缀)必须判等
        assert_eq!(
            canonical_pane_id("19fa6ca625c_15"),
            canonical_pane_id("p_19fa6ca625c_15"),
        );
    }

    #[test]
    fn cwd_compatible_matches_exact() {
        assert!(cwd_compatible("/Users/edy/proj", "/Users/edy/proj"));
    }

    #[test]
    fn cwd_compatible_matches_subdir() {
        assert!(cwd_compatible("/Users/edy/proj/src", "/Users/edy/proj"));
    }

    #[test]
    fn cwd_compatible_rejects_parent_dir() {
        assert!(!cwd_compatible("/Users/edy", "/Users/edy/proj"));
    }

    #[test]
    fn cwd_compatible_rejects_different() {
        assert!(!cwd_compatible("/Users/edy/other", "/Users/edy/proj"));
    }

    #[test]
    fn cwd_compatible_accepts_empty_workspace() {
        assert!(cwd_compatible("/Users/edy/proj", ""));
    }

    #[test]
    fn find_unique_pane_by_workspace_exact_match() {
        let panes = vec![OttyPane {
            id: "p_1".into(),
            tab_id: "t_1".into(),
            window_id: "w_1".into(),
            cwd: "/Users/edy/proj".into(),
            active: false,
            process: String::new(),
        }];
        let found = find_unique_pane_by_workspace(&panes, "/Users/edy/proj");
        assert_eq!(found.map(|p| p.id.as_str()), Some("p_1"));
    }

    #[test]
    fn find_unique_pane_by_workspace_multiple_matches_returns_none() {
        let panes = vec![
            OttyPane {
                id: "p_1".into(),
                tab_id: "t_1".into(),
                window_id: "w_1".into(),
                cwd: "/Users/edy/proj".into(),
                active: false,
                process: String::new(),
            },
            OttyPane {
                id: "p_2".into(),
                tab_id: "t_2".into(),
                window_id: "w_1".into(),
                cwd: "/Users/edy/proj/src".into(),
                active: false,
                process: String::new(),
            },
        ];
        assert!(find_unique_pane_by_workspace(&panes, "/Users/edy/proj").is_none());
    }

    #[test]
    fn find_unique_pane_by_workspace_no_match() {
        let panes = vec![OttyPane {
            id: "p_1".into(),
            tab_id: "t_1".into(),
            window_id: "w_1".into(),
            cwd: "/Users/edy/other".into(),
            active: false,
            process: String::new(),
        }];
        assert!(find_unique_pane_by_workspace(&panes, "/Users/edy/proj").is_none());
    }

    #[test]
    fn find_unique_pane_by_workspace_empty_workspace_returns_none() {
        let panes = vec![OttyPane {
            id: "p_1".into(),
            tab_id: "t_1".into(),
            window_id: "w_1".into(),
            cwd: "/Users/edy/proj".into(),
            active: false,
            process: String::new(),
        }];
        assert!(find_unique_pane_by_workspace(&panes, "").is_none());
    }

    #[test]
    fn decide_otty_capture_skips_when_orca_binding_present() {
        use crate::sessions::{Priority, Session, SessionStatus};
        use chrono::Utc;
        let now = Utc::now();
        let session = Session {
            id: "s1".into(),
            task_id: "pi-1".into(),
            external_session_id: None,
            title: "t".into(),
            message: "m".into(),
            source: Some("pi".into()),
            priority: Priority::Normal,
            status: SessionStatus::Running,
            created_at: now,
            updated_at: now,
            terminal_tty: None,
            workspace_path: Some("/Users/edy/proj".into()),
            failure_reason: None,
            aborted_at: None,
            terminal_id: None,
            terminal_id_captured_cwd: None,
            transcript_path: None,
            herdr_pane_id: None,
            herdr_workspace_id: None,
            herdr_socket_path: None,
            cmux_surface_id: None,
            cmux_workspace_id: None,
            wezterm_pane_id: None,
            wezterm_unix_socket: None,
            wezterm_executable: None,
            otty_pane_id: None,
            otty_tab_id: None,
            otty_window_id: None,
            tmux_pane_id: None,
            tmux_socket_path: None,
            orca_pane_key: Some("tab:leaf".into()),
            orca_worktree_id: Some("wt".into()),
            orca_tab_id: None,
            terminal_app: None,
            dsh_nav_socket: None,
            // W6:读侧走 surface。
            surface: crate::surface::SurfaceBinding::Orca {
                pane_key: "tab:leaf".into(),
                worktree_id: Some("wt".into()),
                tab_id: None,
            },
        };
        assert!(
            decide_otty_capture_reason(&session, None, None, None, None, true, true,).is_none()
        );
    }
}
