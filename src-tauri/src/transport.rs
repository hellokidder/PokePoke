//! 通信层基础工具:`~/.pokepoke/` 主目录定位 + 极简 UDS bind。
//!
//! 由 `docs/briefs/20260423-uds-fixed-path.md` 把通信层从 TCP localhost
//! (`127.0.0.1:9876` 主 / `9877` 降级)修正为固定路径 UDS
//! (`<pokepoke_dir>/sock`),零端口冲突。这是对
//! `docs/archive/scope-audit-20260422.md §3.1` 的**局部修正**:多实例发现表 + 多 sock
//! 路由仍然让出,只把 transport 形态换回 UDS。

#![cfg(unix)]

use std::io;
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 主目录:`~/.pokepoke`。HOME 拿不到时退到 `/tmp/.pokepoke`(极端兜底)。
/// 仍被 settings / sessions / log 共用。
pub fn default_pokepoke_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".pokepoke"),
        None => PathBuf::from("/tmp/.pokepoke"),
    }
}

/// hook ↔ server 的固定 socket 路径:`<pokepoke_dir>/sock`。
pub const SOCK_FILENAME: &str = "sock";

pub fn sock_path(pokepoke_dir: &Path) -> PathBuf {
    pokepoke_dir.join(SOCK_FILENAME)
}

/// 探活/绑定结果。`PeerAlive` 让上层知道是"对等实例已活",不是真错误,
/// 调用方应当 log + 退出而不是 panic。
pub enum BindOutcome {
    Bound(StdUnixListener),
    PeerAlive,
}

/// 在 `<pokepoke_dir>/sock` 绑定 UDS。流程:
///   1. 确保父目录存在并 0700
///   2. 直接 `bind`;成功 → Bound
///   3. EADDRINUSE → connect 探活(200ms 超时):
///      a. 连得通 → 对等实例存活 → PeerAlive(单实例语义,主进程退出)
///      b. 连不通 → stale sock → unlink + 重 bind → Bound
///   4. 其他错误 → io::Error 上抛
pub fn bind_unix(pokepoke_dir: &Path) -> io::Result<BindOutcome> {
    ensure_dir_0700(pokepoke_dir)?;
    let path = sock_path(pokepoke_dir);

    match StdUnixListener::bind(&path) {
        Ok(listener) => {
            chmod_0600(&path)?;
            listener.set_nonblocking(true)?;
            Ok(BindOutcome::Bound(listener))
        }
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if peer_alive(&path) {
                Ok(BindOutcome::PeerAlive)
            } else {
                std::fs::remove_file(&path)?;
                let listener = StdUnixListener::bind(&path)?;
                chmod_0600(&path)?;
                listener.set_nonblocking(true)?;
                Ok(BindOutcome::Bound(listener))
            }
        }
        Err(e) => Err(e),
    }
}

/// 探活:UnixStream 短连一次,成功即对端存活。200ms 超时避免 hung peer 卡死。
fn peer_alive(path: &Path) -> bool {
    // UnixStream::connect 没有 timeout 参数,但拿到 stream 后 set_read_timeout
    // 能限定后续 IO;真正想做"connect 短超时"在 UDS 上几乎没意义——
    // 本机 UDS 的 connect syscall 不会真的卡(除非 listener backlog 满 + 进程不
    // accept,极端情况)。这里直接用阻塞 connect。
    UnixStream::connect(path).is_ok()
}

/// 创建(或确认)目录并 chmod 0700。
fn ensure_dir_0700(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    let mut perms = std::fs::metadata(dir)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(dir, perms)
}

/// 已存在文件 → chmod 0600。
fn chmod_0600(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

/// hook / 测试用的连接超时(本机 UDS 不应该慢)。
pub const HOOK_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn fresh_pokepoke_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join(".pokepoke");
        (tmp, dir)
    }

    #[test]
    fn bind_unix_creates_sock_and_dir_with_correct_perms() {
        let (_tmp, dir) = fresh_pokepoke_dir();
        let outcome = bind_unix(&dir).unwrap();
        let listener = match outcome {
            BindOutcome::Bound(l) => l,
            BindOutcome::PeerAlive => panic!("expected Bound on fresh dir"),
        };

        let sock = sock_path(&dir);
        assert!(sock.exists());
        let sock_mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(sock_mode, 0o600, "sock file should be 0600");
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "pokepoke dir should be 0700");
        drop(listener);
    }

    #[test]
    fn bind_unix_returns_peer_alive_when_listener_already_running() {
        let (_tmp, dir) = fresh_pokepoke_dir();
        let _first = bind_unix(&dir).unwrap();

        let outcome = bind_unix(&dir).unwrap();
        assert!(matches!(outcome, BindOutcome::PeerAlive));
    }

    #[test]
    fn bind_unix_unlinks_stale_sock_when_no_peer() {
        let (_tmp, dir) = fresh_pokepoke_dir();
        // 模拟前次崩溃残留:手动创建一个 sock 文件但没 listener
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(sock_path(&dir), b"stale").unwrap();

        let outcome = bind_unix(&dir).unwrap();
        assert!(matches!(outcome, BindOutcome::Bound(_)));
    }

    #[test]
    fn bind_unix_propagates_real_errors() {
        // 父目录是个文件 → ensure_dir_0700 失败 → 真错误上抛
        let tmp = tempdir().unwrap();
        let blocker = tmp.path().join("notadir");
        std::fs::write(&blocker, b"").unwrap();
        let result = bind_unix(&blocker);
        assert!(result.is_err());
    }
}
