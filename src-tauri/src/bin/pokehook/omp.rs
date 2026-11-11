//! omp(oh-my-pi)agent 扩展接入:install / uninstall / check。
//!
//! omp 是 pi 的 fork(`@oh-my-pi/pi-coding-agent`,CLI 名 `omp`),接入机制同族:
//! file-drop 一个 TS 扩展,omp 启动时自动发现 `<agent dir>/extensions/<name>/index.ts`,
//! 扩展订阅 omp 事件并归一为 PokePoke 内部协议经 stdin 传给 `poke-hook --source=omp`。
//! server 端复用现有 SessionStart/UserPromptSubmit/Stop/SessionEnd 链路,零 schema 变更。
//!
//! 与 pi 的三处实质差异(详见 `omp-extension.ts` 头部注释):
//!   1. omp 没有 `agent_settled`,settle 判据是 `agent_end` + `willContinue !== true`;
//!   2. 一个 omp 进程承载多个 session(subagent),sub-session 的 id 前 8 字符与父相同
//!      → 必须过滤,否则会冒充父 session 的 Stop;
//!   3. omp 有原生 `tool_approval_requested` → 可进 Pending。
//!
//! 落点默认 `~/.omp/agent/extensions/pokepoke/index.ts`,honor omp 自身的
//! `PI_CODING_AGENT_DIR` / `OMP_PROFILE` / `PI_PROFILE`(见 `paths::omp_agent_dir`)。

use super::deploy::{ensure_binary_installed, print_json};
use super::paths::{hook_bin_path, omp_extension_dir, omp_extension_path, pi_agent_dir};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use super::verify;
use std::fs;

const OMP_EXTENSION_TS: &str = include_str!("omp-extension.ts");

/// 占位符替换后的扩展全文。
pub(crate) fn omp_extension_contents() -> String {
    OMP_EXTENSION_TS.replace("__POKE_HOOK_BIN__", &hook_bin_path().display().to_string())
}

/// omp 与 pi 解析到同一个 agent dir 时无法投放 omp 变体:两个宿主会加载同一个
/// `extensions/pokepoke/index.ts`,只能有一份内容生效,强行写入还会覆盖 pi 变体。
/// 返回 Some(人类可读原因) 表示拒绝投放。
pub(crate) fn omp_shared_agent_dir_conflict() -> Option<String> {
    shared_agent_dir_reason(&super::paths::omp_agent_dir(), &pi_agent_dir())
}

/// omp 与 pi 解析到同一个 agent dir 的可读原因(共用判定,install / uninstall 都吃这条)。
fn shared_agent_dir_reason(omp_dir: &std::path::Path, pi_dir: &std::path::Path) -> Option<String> {
    if !paths_equal(omp_dir, pi_dir) {
        return None;
    }
    Some(format!(
        "omp and pi resolve to the same agent dir ({}); omp extension would overwrite the pi one. \
         Unset PI_CODING_AGENT_DIR or use `omp --profile <name>` instead.",
        omp_dir.display()
    ))
}

/// 卸载前的安全判定:`None` = 可以删,`Some(reason)` = 拒绝删。
///
/// 必须在删之前判,理由(2026-09-24 hello 真机实测):
/// - **共用 agent dir 时 `pokepoke/` 里放的是 pi 变体**:install 会拒绝投放,uninstall
///   若无条件删目录会把 pi 接入的扩展一起删掉(pi 静默失效,只剩 snapshot 可手工恢复);
/// - **目录里可能不是 PokePoke 的东西**:用户自己手写的 `extensions/pokepoke/index.ts`
///   没有 marker,同样不能删。
///
/// 入参化路径 → 纯函数,可用 tempdir 单测(不依赖 HOME / env)。
pub(crate) fn omp_uninstall_refusal(
    omp_dir: &std::path::Path,
    pi_dir: &std::path::Path,
    ext_path: &std::path::Path,
) -> Option<String> {
    if let Some(reason) = shared_agent_dir_reason(omp_dir, pi_dir) {
        return Some(format!("refusing to uninstall: {reason}"));
    }
    if !omp_extension_configured(ext_path) {
        return Some(format!(
            "refusing to remove {}: not a PokePoke-managed omp extension (missing marker or --source=omp)",
            ext_path.display()
        ));
    }
    None
}

/// 路径等价判定:优先文件系统规范化(兼容 symlink / `/private/tmp` 前缀),
/// 规范化失败(目录尚不存在等)退化为字符串比较(去尾斜杠)。
fn paths_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
        return ca == cb;
    }
    let norm = |p: &std::path::Path| p.to_string_lossy().trim_end_matches('/').to_string();
    norm(a) == norm(b)
}

pub(crate) fn cmd_install_omp() {
    if !ensure_binary_installed() {
        return;
    }
    if let Some(reason) = omp_shared_agent_dir_conflict() {
        print_json(&serde_json::json!({
            "status": "error",
            "message": reason,
        }));
        return;
    }

    let ext_path = omp_extension_path();
    let ext_dir = omp_extension_dir();
    if let Err(e) = fs::create_dir_all(&ext_dir) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to create {}: {}", ext_dir.display(), e),
        }));
        return;
    }

    record_pre_write_snapshot("omp", &ext_path);

    let mut tx = WriteTransaction::new();
    tx.snapshot(&ext_path);

    // pokepoke/ 目录是 PokePoke 独占投放物,index.ts 直接覆盖即幂等。
    if let Err(e) = fs::write(&ext_path, omp_extension_contents()) {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to write {}: {}", ext_path.display(), e),
        }));
        return;
    }

    if let Err(e) = verify::verify_omp_writes(&ext_path) {
        tx.rollback();
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!("omp install verify failed (rolled back): {}", e),
        }));
        return;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!("Installed omp extension at {}", ext_path.display()),
    }));
}

pub(crate) fn cmd_uninstall_omp() {
    let ext_dir = omp_extension_dir();
    if !ext_dir.exists() {
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "No omp extension found, nothing to do"
        }));
        return;
    }

    let ext_path = omp_extension_path();
    // 删前守卫:共用 agent dir(里面是 pi 变体)或目录内容不是 PokePoke 投放的 omp 变体 → 拒绝。
    if let Some(reason) =
        omp_uninstall_refusal(&super::paths::omp_agent_dir(), &pi_agent_dir(), &ext_path)
    {
        print_json(&serde_json::json!({
            "status": "error",
            "message": reason,
        }));
        return;
    }

    // 卸载前对 index.ts 做一份 snapshot,便于用户手动复原。
    record_pre_write_snapshot("omp", &ext_path);

    // pokepoke/ 目录是 PokePoke 独占的(上面已确认内容是本接入投放的),直接删整个目录;不碰用户其他扩展。
    match fs::remove_dir_all(&ext_dir) {
        Ok(()) => print_json(&serde_json::json!({
            "status": "ok",
            "message": "Removed omp extension",
        })),
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": format!("Failed to remove {}: {}", ext_dir.display(), e),
        })),
    }
}

/// omp 扩展是否已接入:文件存在且内容含 PokePoke 标记 + `--source=omp`。
pub(crate) fn omp_extension_configured(ext_path: &std::path::Path) -> bool {
    fs::read_to_string(ext_path)
        .map(|c| c.contains("PokePoke omp integration") && c.contains("--source=omp"))
        .unwrap_or(false)
}

/// 已接入但扩展内烘焙的 hook 路径与当前 `~/.local/bin/poke-hook` 不一致 ->
/// 判为过期(drift),交由 detect_hook_drift 走"告警 + 一键修复"。
pub(crate) fn omp_extension_outdated(ext_path: &std::path::Path) -> bool {
    let Ok(content) = fs::read_to_string(ext_path) else {
        return false;
    };
    if !content.contains("PokePoke omp integration") {
        return false;
    }
    let expected = hook_bin_path().display().to_string();
    !content.contains(&expected)
}

pub(crate) fn cmd_check_omp() {
    let installed = hook_bin_path().exists();
    let ext_path = omp_extension_path();
    let hooks_configured = ext_path.exists() && omp_extension_configured(&ext_path);
    let hooks_outdated = hooks_configured && omp_extension_outdated(&ext_path);
    let shared_dir_conflict = omp_shared_agent_dir_conflict().is_some();

    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": hooks_configured,
        "hooks_outdated": hooks_outdated,
        "connected": installed && hooks_configured,
        // 诊断用:omp 与 pi 共用 agent dir 时不投放(见 omp_shared_agent_dir_conflict)
        "shared_agent_dir_conflict": shared_dir_conflict,
        "agent_dir": super::paths::omp_agent_dir().display().to_string(),
    }));
}
