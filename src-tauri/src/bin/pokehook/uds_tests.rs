//! `post_unix`(hook → app 的 UDS 传输)的行为契约测试。
//!
//! 2026-10-13 从 `tests.rs` 的 UDS 段拆出:一是守 §4a 体量门禁(ratchet 只许缩不许涨),
//! 二是这两条用例守的是**跨进程契约**(客户端必须等到响应,否则 app 侧
//! axum/hyper 会取消 in-flight future、通知静默丢失),独立成文件更容易被找到。
//! 参见 brief `docs/briefs/20261013-hook-no-wait-notify.md`。

use super::super::post_unix;
use std::io::Read as _;
use std::os::unix::net::UnixListener;
use tempfile::tempdir;

/// 陈旧 socket 文件:bind 过、但已无 listener(终端/app 已退)。
/// 真实高频场景(PokePoke 未运行时 `~/.pokepoke/sock` 可能还在),
/// 必须快速失败而不是挂住。
#[test]
fn post_unix_returns_false_for_stale_socket_file() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("stale.sock");
    {
        let listener = UnixListener::bind(&sock).unwrap();
        drop(listener);
    }
    assert!(sock.exists(), "前置:陈旧 socket 文件仍存在");
    let started = std::time::Instant::now();
    assert!(!post_unix(&sock, "{}"));
    assert!(started.elapsed() < std::time::Duration::from_millis(200));
}

/// 行为契约护栏(brief `20261013-hook-no-wait-notify.md`):hook **必须**等到
/// server 的响应才关闭连接。app 侧是 axum/hyper,客户端提前关闭会取消
/// in-flight 的 `/notify` future,通知静默丢失。
///
/// 2026-10-13 对真实 app 实测对照(同一 `/notify`,running 事件,各 6 次):
/// - `write + 立即 close` → app 落库 0/6
/// - `write + shutdown(WR) + close` → 0/6
/// - `write + 保持连接 200ms(不读)` → 6/6
/// - `write + 读响应` → 6/6
///
/// 本用例用"慢 300ms 才回 200"的 stub 锁住因果:必须等到响应才算成功,
/// 不能改成 fire-and-forget / 短读窗口。
#[test]
fn post_unix_waits_for_the_server_response() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("slow.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        use std::io::Write as _;
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            // 模拟 app 的处理耗时(实测 0.2–1.9s):处理完才回响应
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n");
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });

    let started = std::time::Instant::now();
    assert!(post_unix(&sock, "{}"));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(250),
        "必须等到响应,而不是写完就返回;实际 {elapsed:?}"
    );

    let received = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default();
    assert!(
        received.starts_with("POST /notify"),
        "请求必须已送达 server,实际收到: {received:?}"
    );
    let _ = handle.join();
}
