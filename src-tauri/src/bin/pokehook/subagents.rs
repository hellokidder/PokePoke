//! Kimi subagent 计数。
//!
//! 背景:pokehook 每次 hook 事件是全新进程,`SubagentStart` / `SubagentStop`
//! 通过 `/tmp/pokepoke-<task_id>.subagents` 文件跨进程传递计数。朴素实现是
//! 跨进程无锁 read-modify-write(读 → 算 → 写),两个 Start 并发到达时各自
//! 读到旧值、各写新值,计数丢一次自增;Stop 事件丢失则计数永不为零、
//! 后续 main Stop 全被 suppress,popup 卡 Running。
//!
//! 本模块的修复:
//! - `flock(LOCK_EX)` 串行化整个 RMW。计数文件常驻(0 时写 "0" 不删),
//!   保证并发进程 flock 始终作用同一 inode;`clear_subagent_count` 只在
//!   会话结束等无并发时删文件。
//! - 计数文件超过 `STALE_AFTER_SECS` 未更新 → 判定 SubagentStop 事件流断裂,
//!   `purge_stale_count` 清空计数并放行 Stop(自愈,不永久卡 Running)。

use std::fs;
use std::path::PathBuf;

use super::LOCK_DIR;

const SUFFIX: &str = "subagents";
/// 计数文件超过该时长未更新 → 判定子 agent 事件流断裂,Stop 自愈放行。
const STALE_AFTER_SECS: u64 = 60 * 60;

fn flag_path(task_id: &str) -> PathBuf {
    PathBuf::from(LOCK_DIR).join(format!("pokepoke-{task_id}.{SUFFIX}"))
}

pub fn active_subagent_count(task_id: &str) -> u32 {
    fs::read_to_string(flag_path(task_id))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// 计数文件是否超过 `STALE_AFTER_SECS` 未更新。
fn count_file_stale(task_id: &str) -> bool {
    let Ok(meta) = fs::metadata(flag_path(task_id)) else {
        return false;
    };
    meta.modified()
        .map(|t| {
            t.elapsed()
                .map(|e| e.as_secs() > STALE_AFTER_SECS)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// 子 agent 事件流断裂自愈:计数 stale → 清空并返回 true,调用方不再 suppress Stop。
pub fn purge_stale_count(task_id: &str) -> bool {
    if count_file_stale(task_id) {
        clear_subagent_count(task_id);
        true
    } else {
        false
    }
}

/// 整个 read-modify-write 持 `flock(LOCK_EX)` 串行化。
/// 计数 0 时写 "0" 而非删文件——文件常驻才能保证并发进程 flock 同一 inode。
pub fn adjust_subagent_count(task_id: &str, delta: i32) -> u32 {
    let path = flag_path(task_id);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path);
    let Ok(file) = file else {
        // 打开失败(目录异常等)退化为无锁原逻辑,不让 hook 流程因计数挂掉
        return (active_subagent_count(task_id) as i32 + delta).max(0) as u32;
    };

    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: fd 来自刚 open 的本地文件,flock 参数为合法 fd + 标准锁常量。
    let locked = unsafe { libc::flock(fd, libc::LOCK_EX) == 0 };
    let next = {
        let current: u32 = fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        (current as i32 + delta).max(0) as u32
    };
    let _ = fs::write(&path, next.to_string());
    if locked {
        // SAFETY: 同一 fd 的解锁调用,参数合法。
        let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
    }
    next
}

/// 会话结束等确定性清理(无并发事件),直接删文件。
pub fn clear_subagent_count(task_id: &str) {
    let _ = fs::remove_file(flag_path(task_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn unique_task_id() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("kimi-subagent-test-{}-{n}", std::process::id())
    }

    #[test]
    fn count_tracks_active_subagents() {
        let id = unique_task_id();
        clear_subagent_count(&id);
        assert_eq!(active_subagent_count(&id), 0);
        assert_eq!(adjust_subagent_count(&id, 1), 1);
        assert_eq!(adjust_subagent_count(&id, 1), 2);
        assert_eq!(adjust_subagent_count(&id, -1), 1);
        assert_eq!(adjust_subagent_count(&id, -1), 0);
        assert_eq!(active_subagent_count(&id), 0);
        clear_subagent_count(&id);
    }

    #[test]
    fn count_never_goes_below_zero() {
        let id = unique_task_id();
        clear_subagent_count(&id);
        assert_eq!(adjust_subagent_count(&id, -3), 0);
        clear_subagent_count(&id);
    }

    #[test]
    fn stale_count_is_purged_and_reported() {
        let id = unique_task_id();
        clear_subagent_count(&id);
        adjust_subagent_count(&id, 2);
        // 手工把 mtime 拨老,绕过 60min 等待
        let old = std::time::SystemTime::now() - Duration::from_secs(STALE_AFTER_SECS + 1);
        set_mtime(&flag_path(&id), old).expect("set mtime");
        assert!(purge_stale_count(&id));
        assert_eq!(active_subagent_count(&id), 0);
        // 非 stale → 不清不清报
        adjust_subagent_count(&id, 1);
        assert!(!purge_stale_count(&id));
        clear_subagent_count(&id);
    }

    fn set_mtime(path: &std::path::Path, t: std::time::SystemTime) -> std::io::Result<()> {
        let file = fs::File::options().write(true).open(path)?;
        let times = std::fs::FileTimes::new().set_modified(t);
        file.set_times(times)
    }
}
