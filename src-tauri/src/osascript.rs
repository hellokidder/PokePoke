//! 带**超时**的 osascript 执行 helper。
//!
//! 存在的理由(2026-07-01 主线程挂死事故):`osascript` 会去跟别的 App(iTerm2 /
//! Terminal / Ghostty)做 Apple Event 往返,一旦目标 App 卡住 / 不响应,裸
//! `Command::output()` 会**无限期阻塞**。历史上 focus 命令是同步 `#[tauri::command]`,
//! Tauri v2 里同步命令在**主线程**跑 → 一次挂住的 osascript 直接把主线程钉死,
//! `run_on_main_thread` 派进去的 popup 建/销闭包全部排队饿死,表现为整个 UI 卡死。
//!
//! 两道防线:
//!   1. focus 命令改 `#[tauri::command(async)]` + `spawn_blocking`,把 osascript 挪出主线程;
//!   2. **这里**给每次 osascript 加硬超时,即便挪出主线程也不让某个后台线程被永久占住,
//!      并把耗时落日志,下次真出问题一眼能看到是哪次调用慢/超时。
//!
//! 实现不引第三方 crate:`spawn` 后轮询 `try_wait`,超时 `kill`。osascript 输出都是
//! 极小字符串(id / bool / bounds),piped stdout 不会撑爆管道 buffer。

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// focus / 枚举类 osascript 的默认超时。正常一次 ~50–500ms(focus_ghostty_by_id
/// 带 3×重试 + delay 也就 ~0.5s),5s 足够覆盖慢机 / 多窗口枚举,又不至于让
/// 挂死的 App 拖太久。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// 超过这个耗时就把该次调用升级到 WARN —— 正常 osascript 不该这么慢,慢了就是
/// 目标 App 响应迟钝的前兆(下次挂死前的预警信号)。
const SLOW_THRESHOLD: Duration = Duration::from_millis(2000);

/// try_wait 轮询间隔。10ms 给快速调用带来的额外延迟可忽略(相对 osascript 本身
/// 几十上百 ms),又能让超时判定足够及时。
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 跑一段 AppleScript,最多等 `timeout`。返回:
///   - `Some(stdout_trimmed)`:进程在超时内正常退出且 exit status 成功
///   - `None`:spawn 失败 / 超时被 kill / 非零退出 / stdout 读取失败
///
/// `tag` / `op` 只用于日志(如 tag="focus", op="focus_iterm2"),便于排查定位是
/// 哪条链路的 osascript 慢或挂。
pub fn run(script: &str, timeout: Duration, tag: &str, op: &str) -> Option<String> {
    let mut cmd = Command::new("osascript");
    cmd.arg("-e").arg(script);
    run_command_with_timeout(cmd, timeout, tag, op)
}

/// 核心:spawn 一个已配好参数的 `Command`,最多等 `timeout`,超时 kill。抽出来是
/// 为了让单测能用 `sleep` / `printf` 之类可控命令验证超时 / 正常退出 / 非零退出
/// 三条路径(单测环境跑不了真 osascript)。语义同 [`run`]。
fn run_command_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    tag: &str,
    op: &str,
) -> Option<String> {
    let start = Instant::now();
    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            if let Some(m) = crate::runtime_metrics::global() {
                m.record_osascript_spawn_failed();
            }
            crate::log_warn!(tag, "osascript spawn failed op={} err={}", op, e);
            return None;
        }
    };

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let elapsed = start.elapsed();
                let mut buf = String::new();
                if let Some(mut out) = stdout_pipe.take() {
                    let _ = out.read_to_string(&mut buf);
                }
                if elapsed >= SLOW_THRESHOLD {
                    if let Some(m) = crate::runtime_metrics::global() {
                        m.record_osascript_slow();
                    }
                    crate::log_warn!(
                        tag,
                        "osascript SLOW op={} elapsed_ms={} success={}",
                        op,
                        elapsed.as_millis(),
                        status.success(),
                    );
                } else {
                    crate::log_debug!(
                        tag,
                        "osascript op={} elapsed_ms={} success={}",
                        op,
                        elapsed.as_millis(),
                        status.success(),
                    );
                }
                if !status.success() {
                    // 非零退出 = AppleScript 抛了运行时异常(如 focus_iterm2 的
                    // `select s` 撞上目标 App 正忙)。历史上 stderr 被 Stdio::null()
                    // 丢弃,导致"select 偶发失败"被吞成 return false 且无任何线索
                    // (2026-07-14 herdr-in-iTerm2 回跳偶发失败事故)。现在捕获 stderr
                    // 落 WARN,让这类瞬时竞态可定位。
                    let mut errbuf = String::new();
                    if let Some(mut err) = stderr_pipe.take() {
                        let _ = err.read_to_string(&mut errbuf);
                    }
                    let err_trim = errbuf.trim();
                    if let Some(m) = crate::runtime_metrics::global() {
                        m.record_osascript_failed();
                    }
                    crate::log_warn!(
                        tag,
                        "osascript FAILED op={} exit={} stderr={}",
                        op,
                        status.code().unwrap_or(-1),
                        if err_trim.is_empty() {
                            "<empty>"
                        } else {
                            err_trim
                        },
                    );
                    return None;
                }
                return Some(buf.trim().to_string());
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    // 目标 App 没在超时内应答 —— 杀掉子进程,绝不让调用方(哪怕是
                    // 后台线程)被永久占住。这是挂死事故的兜底防线。
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some(m) = crate::runtime_metrics::global() {
                        m.record_osascript_timeout(op);
                    }
                    crate::log_warn!(
                        tag,
                        "osascript TIMEOUT op={} after_ms={} killed (target app unresponsive?)",
                        op,
                        timeout.as_millis(),
                    );
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                crate::log_warn!(tag, "osascript try_wait failed op={} err={}", op, e);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// 便捷封装:跑 script,stdout trim 后是否等于 "true"。超时 / 失败都算 false。
pub fn run_bool(script: &str, tag: &str, op: &str) -> bool {
    run(script, DEFAULT_TIMEOUT, tag, op)
        .map(|s| s == "true")
        .unwrap_or(false)
}

/// 当前 macOS 前台 app 进程名(如 `cmux` / `Cursor` / `iTerm2`)。失败返回 None。
pub fn frontmost_app_name() -> Option<String> {
    run(
        r#"tell application "System Events" to name of first application process whose frontmost is true"#,
        DEFAULT_TIMEOUT,
        "osascript",
        "frontmost_app",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // 用 sleep / printf / false 这些无处不在的命令验证核心超时逻辑,不依赖真 osascript。

    #[test]
    fn returns_trimmed_stdout_on_success() {
        // 绝对路径:agent_presence 的 EnvGuard 测试会临时改 PATH,裸命令名会
        // 偶发解析失败导致 spawn None(flaky)。
        let mut cmd = Command::new("/usr/bin/printf");
        cmd.arg("hello\\n");
        let out = run_command_with_timeout(cmd, Duration::from_secs(5), "test", "printf");
        assert_eq!(out.as_deref(), Some("hello"));
    }

    #[test]
    fn kills_and_returns_none_on_timeout() {
        // sleep 10s,但只给 200ms → 必然超时被 kill,且函数在超时附近就返回,
        // 不会真等满 10s。
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("10");
        let start = Instant::now();
        let out = run_command_with_timeout(cmd, Duration::from_millis(200), "test", "sleep");
        let elapsed = start.elapsed();
        assert_eq!(out, None, "timeout must yield None");
        assert!(
            elapsed < Duration::from_secs(2),
            "must return promptly after timeout, took {:?}",
            elapsed
        );
    }

    #[test]
    fn returns_none_on_nonzero_exit() {
        // `false` 立即以非零退出 → None(区别于超时,这条是"命令失败")。
        let cmd = Command::new("/usr/bin/false");
        let out = run_command_with_timeout(cmd, Duration::from_secs(5), "test", "false");
        assert_eq!(out, None);
    }

    #[test]
    fn returns_none_on_spawn_failure() {
        let cmd = Command::new("this-binary-does-not-exist-pokepoke");
        let out = run_command_with_timeout(cmd, Duration::from_secs(5), "test", "missing");
        assert_eq!(out, None);
    }
}
