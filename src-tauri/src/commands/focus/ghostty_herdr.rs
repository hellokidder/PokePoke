//! Ghostty 上 herdr UI client 的定位与聚焦(从 `ghostty.rs` 拆出,
//! 2026-08-10 段 C Ghostty 宿主分支重构,brief 20260810-herdr-remote-ghostty-jumpback)。
//! 本地 herdr(段 A anchor)+ herdr-remote(段 C client tty → tab)两类宿主。

use super::applescript::{applescript_escape, is_app_running, run_applescript_bool};
use super::ghostty::focus_ghostty_by_id;
use super::ghostty_capture::{enumerate_ghostty_terminals, GhosttyCandidate};
use super::terminal_id_hint;

fn pick_ghostty_herdr_terminal(sock: Option<&std::path::Path>) -> Option<String> {
    if !is_app_running("Ghostty") {
        return None;
    }
    let named_session = sock.and_then(crate::herdr::named_session_from_socket_path);
    let session_ref = named_session.as_deref();
    let matches: Vec<_> = enumerate_ghostty_terminals()
        .into_iter()
        .filter(|c| {
            if sock.is_some() {
                crate::herdr::ghostty_tab_title_matches_named_session(&c.title, session_ref)
            } else {
                c.title.trim().starts_with("herdr")
            }
        })
        .collect();
    match matches.len() {
        1 => Some(matches[0].id.clone()),
        n => {
            crate::log_info!(
                "focus",
                "pick_ghostty_herdr_terminal sock={:?} session={:?} found {} tabs (need exactly 1)",
                sock.map(|p| p.display().to_string()),
                session_ref,
                n
            );
            None
        }
    }
}

pub(super) fn focus_ghostty_herdr_anchor_for_socket_opt(sock: Option<&std::path::Path>) -> bool {
    let Some(id) = pick_ghostty_herdr_terminal(sock) else {
        return false;
    };
    let escaped = applescript_escape(&id);
    let ok = focus_ghostty_by_id(&escaped);
    crate::log_info!(
        "focus",
        "focus_ghostty_herdr_anchor sock={:?} terminal_id_hint={} -> {}",
        sock.map(|p| p.display().to_string()),
        terminal_id_hint(Some(&id)),
        ok,
    );
    ok
}

pub(super) fn focus_ghostty_herdr_anchor_for_socket(sock: &std::path::Path) -> bool {
    focus_ghostty_herdr_anchor_for_socket_opt(Some(sock))
}

pub(super) fn focus_ghostty_herdr_anchor() -> bool {
    focus_ghostty_herdr_anchor_for_socket_opt(None)
}

/// 段 C(herdr-remote)Ghostty 宿主分支:激活承载 `herdr --remote <target>` UI
/// client 的 Ghostty tab(brief 20260810-herdr-remote-ghostty-jumpback)。
///
/// Ghostty AppleScript 只暴露 id/name/working directory(sdef 无 tty 属性),
/// 无法像 Otty 那样按 tty 精确定位,用三级定位 + activate-only 兜底:
/// 1. cwd 精确匹配:client 进程启动目录 ↔ Ghostty working directory(OSC7 最后
///    上报值;herdr 全屏运行期间 shell precmd 不跑、冻结在启动目录,两者一致)
/// 2. title 前缀 "herdr"(复用 pick_ghostty_herdr_terminal 无 socket 语义)
/// 3. Ghostty 仅一个 terminal:直接聚焦
/// 全失败:activate 抬前窗(比"完全没动静"强,2026-08-07 段 C 用户反馈)。
pub(super) fn focus_ghostty_herdr_client_by_tty(client_tty: &str) -> bool {
    if !is_app_running("Ghostty") {
        return false;
    }
    let candidates = enumerate_ghostty_terminals();
    if candidates.is_empty() {
        return false;
    }
    let client_cwd = herdr_client_cwd_on_tty(client_tty);
    let Some(id) = pick_ghostty_client_terminal(&candidates, client_cwd.as_deref()) else {
        let ok = run_applescript_bool(r#"tell application "Ghostty" to activate"#);
        crate::log_info!(
            "focus",
            "focus_ghostty_herdr_client_by_tty tty={} no unique tab, activate-only -> {}",
            client_tty,
            ok
        );
        return ok;
    };
    let escaped = applescript_escape(&id);
    let ok = focus_ghostty_by_id(&escaped);
    crate::log_info!(
        "focus",
        "focus_ghostty_herdr_client_by_tty tty={} terminal_id_hint={} -> {}",
        client_tty,
        terminal_id_hint(Some(&id)),
        ok
    );
    ok
}

/// herdr client(wrapper)进程在 tty 上的 cwd:ps 找 comm=="herdr" 的 pid,
/// `lsof -a -p <pid> -d cwd -Fn` 拿路径。进程 cwd 在 herdr 前台运行期间冻结
/// 在启动目录,与 Ghostty working directory(OSC7 最后一次上报)一致。
fn herdr_client_cwd_on_tty(tty: &str) -> Option<String> {
    use std::process::Command;
    let bare = tty.trim().strip_prefix("/dev/").unwrap_or(tty.trim());
    let out = Command::new("ps")
        .args(["-t", bare, "-o", "pid=,comm="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pid: Option<u32> = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid_s), Some(comm)) = (it.next(), it.next()) else {
            continue;
        };
        if comm == "herdr" || comm.ends_with("/herdr") {
            pid = pid_s.parse::<u32>().ok();
            break;
        }
    }
    let pid = pid?;
    let out = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|l| {
        l.strip_prefix('n')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
    })
}

/// 段 C Ghostty 宿主 tab 选择:优先级 cwd 精确 > title "herdr" > 单候选兜底。
/// 抽纯函数供单测(真 osascript / lsof 单测跑不了)。
fn pick_ghostty_client_terminal(
    candidates: &[GhosttyCandidate],
    client_cwd: Option<&str>,
) -> Option<String> {
    if let Some(cwd) = client_cwd {
        let norm = cwd.trim().trim_end_matches('/');
        if !norm.is_empty() {
            let cwd_matches: Vec<&GhosttyCandidate> = candidates
                .iter()
                .filter(|c| {
                    let tc = c.cwd.trim().trim_end_matches('/');
                    !tc.is_empty() && tc == norm
                })
                .collect();
            if cwd_matches.len() == 1 {
                return Some(cwd_matches[0].id.clone());
            }
        }
    }
    let title_matches: Vec<&GhosttyCandidate> = candidates
        .iter()
        .filter(|c| c.title.trim().starts_with("herdr"))
        .collect();
    if title_matches.len() == 1 {
        return Some(title_matches[0].id.clone());
    }
    if candidates.len() == 1 {
        return Some(candidates[0].id.clone());
    }
    None
}

/// popup 聚焦抑制:按 session socket 判定 Ghostty 前台是否在对的 herdr tab。
pub(crate) fn is_ghostty_herdr_tab_focused_for_socket(sock: &std::path::Path) -> bool {
    let Some(id) = pick_ghostty_herdr_terminal(Some(sock)) else {
        return false;
    };
    let escaped = applescript_escape(&id);
    let script = format!(
        r#"tell application "System Events"
    set frontApp to name of first application process whose frontmost is true
end tell
if frontApp is not "Ghostty" then return false
using terms from application id "com.mitchellh.ghostty"
    tell application id "com.mitchellh.ghostty"
        try
            return (id of focused terminal of selected tab of front window) is "{escaped}"
        end try
    end tell
end using terms from
return false"#
    );
    run_applescript_bool(&script)
}

#[cfg(test)]
mod ghostty_herdr_client_tests {
    use super::{pick_ghostty_client_terminal, GhosttyCandidate};

    fn cand(id: &str, cwd: &str, title: &str) -> GhosttyCandidate {
        GhosttyCandidate {
            id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
        }
    }

    /// cwd 唯一命中优先于 title(本地 herdr tab 与 remote herdr tab 同 title
    /// 时,启动目录不同 → cwd 消歧)。
    #[test]
    fn pick_prefers_unique_cwd_over_title() {
        let cands = vec![
            cand("LOCAL", "/Users/edy/work", "herdr"),
            cand("REMOTE", "/Users/edy/remote-proj", "herdr"),
        ];
        assert_eq!(
            pick_ghostty_client_terminal(&cands, Some("/Users/edy/remote-proj")),
            Some("REMOTE".into())
        );
    }

    /// cwd 不匹配时退到 title 前缀唯一命中。
    #[test]
    fn pick_falls_back_to_unique_herdr_title() {
        let cands = vec![
            cand("A", "/tmp/other", "btm"),
            cand("B", "/Users/edy/x", "herdr"),
        ];
        assert_eq!(
            pick_ghostty_client_terminal(&cands, Some("/nonexistent")),
            Some("B".into())
        );
    }

    /// cwd 歧义(多个 tab 同目录)且 title 也歧义 → None,调用方走 activate-only。
    #[test]
    fn pick_ambiguous_cwd_and_title_returns_none() {
        let cands = vec![
            cand("A", "/Users/edy", "herdr"),
            cand("B", "/Users/edy", "herdr"),
        ];
        assert_eq!(
            pick_ghostty_client_terminal(&cands, Some("/Users/edy")),
            None
        );
    }

    /// 单候选兜底:即使 title/cwd 都对不上也聚焦唯一的 tab。
    #[test]
    fn pick_single_candidate_fallback() {
        let cands = vec![cand("ONLY", "/Users/edy", "btm")];
        assert_eq!(
            pick_ghostty_client_terminal(&cands, Some("/elsewhere")),
            Some("ONLY".into())
        );
    }

    /// cwd 尾斜杠差异归一(OSC7 上报 `/Users/edy/` vs lsof `/Users/edy`)。
    #[test]
    fn pick_normalizes_trailing_slash() {
        let cands = vec![cand("A", "/Users/edy/proj/", "herdr")];
        assert_eq!(
            pick_ghostty_client_terminal(&cands, Some("/Users/edy/proj")),
            Some("A".into())
        );
    }

    /// 无 client cwd(进程已退 / lsof 失败)时走 title / 单候选。
    #[test]
    fn pick_without_client_cwd() {
        let cands = vec![
            cand("A", "/tmp/a", "zsh"),
            cand("B", "/tmp/b", "herdr hello"),
        ];
        assert_eq!(pick_ghostty_client_terminal(&cands, None), Some("B".into()));
    }
}
