//! 聚焦路径上的外部 CLI 调用超时封装,避免 Orca/Otty hang 占满 blocking 线程。

use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// Orca / Otty CLI 默认超时(与 agent_presence 的 2s 略长,JSON list 可能更慢)。
pub(crate) const FOCUS_CLI_TIMEOUT: Duration = Duration::from_secs(3);

/// `Command::output()` 带超时;超时后 kill 子进程并记日志。
pub(crate) fn command_output_with_timeout(
    cmd: Command,
    timeout: Duration,
    label: &str,
) -> Option<Output> {
    command_output_with_timeout_inner(cmd, timeout, label)
}

#[cfg(unix)]
fn command_output_with_timeout_inner(
    mut cmd: Command,
    timeout: Duration,
    label: &str,
) -> Option<Output> {
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::Instant;

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout_pipe = child.stdout.take()?;
    let mut stderr_pipe = child.stderr.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut stdout);
        let _ = stderr_pipe.read_to_end(&mut stderr);
        let _ = tx.send((stdout, stderr));
    });

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().ok().flatten() {
            Some(status) => {
                let (stdout, stderr) = rx.recv().unwrap_or((Vec::new(), Vec::new()));
                return Some(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                crate::log_info!("focus", "cli timeout label={} after {:?}", label, timeout);
                return None;
            }
            None => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(not(unix))]
fn command_output_with_timeout_inner(
    cmd: Command,
    _timeout: Duration,
    _label: &str,
) -> Option<Output> {
    cmd.output().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_with_timeout_returns_output_for_fast_command() {
        // 本用例只验证"快命令能正常取回输出",不验证"快命令能跑赢 3s 产品超时"。
        // 用绝对路径 + 宽松超时:并发测试(agent_presence 的 EnvGuard 会临时改
        // PATH)会踩掉裸命令名的 PATH 解析,绑死 FOCUS_CLI_TIMEOUT 又会在 FULL
        // 测试负载下误报超时。
        let out = command_output_with_timeout(
            Command::new("/usr/bin/true"),
            Duration::from_secs(30),
            "test/true",
        );
        assert!(out.is_some());
        assert!(out.unwrap().status.success());
    }

    #[cfg(unix)]
    #[test]
    fn command_output_with_timeout_kills_slow_command() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        let out = command_output_with_timeout(cmd, Duration::from_millis(200), "test/sleep");
        assert!(out.is_none());
    }
}
