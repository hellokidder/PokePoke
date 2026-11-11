//! 部署共享设施:JSON 输出、`~/.local/bin/poke-hook` 自安装、`--deploy-self`。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use super::hook_deploy;
use super::paths::hook_bin_path;
use serde_json::Value;
use std::env;
use std::fs;

pub(crate) fn print_json(val: &Value) {
    println!(
        "{}",
        serde_json::to_string(val).unwrap_or_else(|_| "{}".into())
    );
}

/// Copy current binary to ~/.local/bin/poke-hook if needed.
pub(crate) fn ensure_binary_installed() -> bool {
    let target = hook_bin_path();
    let self_exe = env::current_exe().unwrap_or_default();

    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if self_exe != target {
        if let Err(e) = fs::copy(&self_exe, &target) {
            print_json(
                &serde_json::json!({"status":"error","message":format!("Failed to copy binary: {}", e)}),
            );
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
        }
    }
    true
}

/// `pnpm install:hook` 走此路径:把当前 pokehook 二进制部署到 ~/.local/bin/poke-hook,
/// 与 PokePoke 启动期 `ensure_hook_deployed` 共用 hook_deploy 逻辑(去 quarantine + 原子覆盖)。
pub(crate) fn cmd_deploy_self() {
    let src = match env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            print_json(&serde_json::json!({
                "status": "error",
                "message": format!("current_exe failed: {}", e),
            }));
            return;
        }
    };
    let dst = hook_bin_path();
    match hook_deploy::deploy_hook_from_to(&src, &dst) {
        Ok(()) => print_json(&serde_json::json!({
            "status": "ok",
            "path": dst.display().to_string(),
        })),
        Err(e) => print_json(&serde_json::json!({
            "status": "error",
            "message": format!("deploy failed: {}", e),
        })),
    }
}
