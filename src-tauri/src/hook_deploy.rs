//! Hook binary 部署到 `~/.local/bin/poke-hook` 的共享逻辑。
//! `ensure_hook_deployed` 与 `pokehook --deploy-self`(`pnpm install:hook`) 共用。

use std::path::Path;
use std::process::Command;

/// macOS 从 bundle / target 拷到 ~/.local/bin 时会带上 quarantine xattr,
/// Gatekeeper 可能直接 SIGKILL hook 进程(CC 侧静默,server 收不到 /notify)。
#[cfg(target_os = "macos")]
fn strip_macos_quarantine(path: &Path) {
    let Some(path_str) = path.to_str() else {
        return;
    };
    let _ = Command::new("xattr")
        .args(["-d", "com.apple.quarantine", path_str])
        .output();
}

#[cfg(not(target_os = "macos"))]
fn strip_macos_quarantine(_path: &Path) {}

/// 覆盖 copy + chmod 0755 + 自动建父目录 + (macOS) 去 quarantine。
/// 先写 `.deploying` 临时文件再 rename,避免覆盖旧 inode 时残留 quarantine。
pub fn deploy_hook_from_to(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dst.with_extension("deploying");
    std::fs::copy(src, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms)?;
    }
    strip_macos_quarantine(&tmp);
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::deploy_hook_from_to;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn deploys_when_target_missing_and_creates_parent_dir() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src-hook");
        std::fs::write(&src, b"#!/bin/sh\nfake binary\n").unwrap();

        let dst_dir = dir.path().join("nested/.local/bin");
        let dst = dst_dir.join("poke-hook");
        assert!(!dst_dir.exists());

        deploy_hook_from_to(&src, &dst).unwrap();

        assert!(dst.exists(), "target should exist after deploy");
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"#!/bin/sh\nfake binary\n",
            "content should match src"
        );
    }

    #[test]
    fn overwrites_existing_target() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src-hook");
        std::fs::write(&src, b"new").unwrap();
        let dst = dir.path().join("dst-hook");
        std::fs::write(&dst, b"old").unwrap();

        deploy_hook_from_to(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"new");
    }

    #[test]
    fn sets_executable_perms_0755() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src-hook");
        std::fs::write(&src, b"x").unwrap();
        let dst = dir.path().join("dst-hook");
        std::fs::write(&dst, b"old").unwrap();
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&dst, perms).unwrap();

        deploy_hook_from_to(&src, &dst).unwrap();

        let mode = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "deploy should set mode to 0755");
    }

    #[test]
    fn propagates_not_found_when_src_missing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("does-not-exist");
        let dst = dir.path().join("dst");

        let err = deploy_hook_from_to(&src, &dst).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
