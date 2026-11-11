//! 极简统一日志通道。
//!
//! 设计目标:
//! - 不引入新依赖(不上 tracing / log + env_logger),全部用 std。
//! - DEBUG 级是否落盘由 `POKEPOKE_DEBUG` 环境变量在 init 时一次性决定。
//!   `1` / `true` / `yes` / `on` 任一即启用,大小写不敏感。
//! - 写文件 + 打 stderr 双通道;release 用户没终端也能看到日志。
//! - 文件 fd 全局复用一份,避免每条日志重复 open/close;用 `Mutex<File>` 串行写。
//!
//! 不做的事(scope-audit §3.8 + §8.2):
//! - 不做日志轮转 / 大小上限 / 自动归档
//! - 不做按模块的级别过滤(DEBUG 总开关)
//! - 不接管 hook 子进程的日志(hook 是独立 binary,自己管自己)
//! - 不做运行时切换 DEBUG(原 settings UI dev_mode 已让出)

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

struct Logger {
    file: Mutex<Option<File>>,
    file_path: Mutex<Option<PathBuf>>,
    debug_enabled: AtomicBool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

fn instance() -> &'static Logger {
    LOGGER.get_or_init(|| Logger {
        file: Mutex::new(None),
        file_path: Mutex::new(None),
        debug_enabled: AtomicBool::new(false),
    })
}

/// 读 POKEPOKE_DEBUG 环境变量,识别启用值(1/true/yes/on,大小写不敏感)。
fn debug_enabled_from_env() -> bool {
    std::env::var("POKEPOKE_DEBUG")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// 日志轮转阈值:超过 5MB 则在 init 时 rotate 一次。常驻进程不轮转迟早
/// 累积到几十/上百 MB,既占磁盘也让"抓日志排查"变难(编辑器卡、tail 慢)。
/// 5MB 对应约几万行 INFO,够覆盖一轮典型排查窗口;超过就 rollover,只留
/// 1 份历史(`.log.1`),极简,不引入轮转库。
const ROTATE_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

/// 初始化 logger:打开/创建日志文件,DEBUG 启用与否由 POKEPOKE_DEBUG env 决定。
/// 多次调用幂等——文件 fd 会被替换,debug_enabled 会被重新评估。
/// 若已存在日志文件超过 `ROTATE_THRESHOLD_BYTES`,先 rotate 到 `.log.1`
/// 再开新文件,避免单文件无限增长。
pub fn init(file_path: PathBuf) {
    let logger = instance();
    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate_if_too_large(&file_path);
    let opened = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .ok();
    *logger.file.lock().unwrap_or_else(|p| p.into_inner()) = opened;
    *logger.file_path.lock().unwrap_or_else(|p| p.into_inner()) = Some(file_path);
    logger
        .debug_enabled
        .store(debug_enabled_from_env(), Ordering::Relaxed);
}

/// 若 `file_path` 存在且大小超过阈值,把它重命名成 `{file_path}.1`(覆盖已有
/// 的 .1,即只保留 1 份历史)。失败不致命——最坏是继续往大文件 append,
/// 仅打 stderr 提示(此时 logger 还没初始化,不能用 log_* 宏)。
fn rotate_if_too_large(file_path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(file_path) else {
        return;
    };
    if meta.len() < ROTATE_THRESHOLD_BYTES {
        return;
    }
    // 拼成 `pokepoke.log.1`(在原文件名后加 ".1",不动原扩展名)。
    // 只保留 1 份历史:已有 .1 会被 rename 覆盖。
    let rotated = {
        let mut s = file_path.as_os_str().to_owned();
        s.push(".1");
        std::path::PathBuf::from(s)
    };
    match std::fs::rename(file_path, &rotated) {
        Ok(()) => {
            eprintln!(
                "pokepoke logger: rotated {} ({} bytes) -> {}",
                file_path.display(),
                meta.len(),
                rotated.display()
            );
        }
        Err(e) => {
            eprintln!(
                "pokepoke logger: rotate failed for {}: {} (继续 append 到原文件)",
                file_path.display(),
                e
            );
        }
    }
}

/// 测试用:跳过 env 读取直接指定 debug 启用与否。
#[cfg(test)]
fn init_with_debug(file_path: PathBuf, debug: bool) {
    init(file_path);
    instance().debug_enabled.store(debug, Ordering::Relaxed);
}

fn is_debug_enabled() -> bool {
    instance().debug_enabled.load(Ordering::Relaxed)
}

/// 截断日志文件到 0 字节,保留 fd 不失效。
#[cfg(test)]
fn clear_log_file() -> std::io::Result<()> {
    let logger = instance();
    let mut guard = logger.file.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(file) = guard.as_mut() {
        file.set_len(0)?;
        if let Some(path) = logger
            .file_path
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            *guard = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .ok();
            if guard.is_some() {
                *guard = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .ok();
            }
        }
    }
    Ok(())
}

/// 核心写入。tag 用来标识来源(如 "panel" / "hook" / "frontend")。
pub fn log(level: Level, tag: &str, message: &str) {
    if matches!(level, Level::Debug) && !is_debug_enabled() {
        return;
    }

    // 日志时间用系统本地时区(带 `+HH:MM` 偏移),方便用户/排查者直接跟终端时间
    // 对账。之前用 UTC + `Z` 后缀,东八区用户每次都要减 8 小时才知道"刚刚"是
    // 哪条,排查体验不好。
    let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z");
    let line = format!("{} [{}] [{}] {}\n", ts, level.as_str(), tag, message);

    eprint!("{}", line);

    let logger = instance();
    if let Ok(mut guard) = logger.file.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// 从 panic payload 中提取可读文本（&str / String / 其它类型名）。
pub fn panic_payload_text(payload: &dyn std::any::Any) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        format!("<{}>", std::any::type_name_of_val(payload))
    }
}

/// 安装全局 panic hook：任何线程（含 tokio worker）panic 都落盘 pokepoke.log，
/// 让"静默线程死亡 / daemon 无痕退出"变为可诊断（payload + location + backtrace）。
///
/// 安全约定：写日志走 [`log`]（内部 try-lock）——panic 线程若正持 logger 锁会
/// 安全跳过文件写入（仍打 stderr），不会二次 panic / 死锁。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
        let payload = panic_payload_text(info.payload());
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(
            Level::Error,
            "panic",
            &format!(
                "thread={} payload={} location={}\nbacktrace:\n{}",
                thread_name, payload, location, backtrace
            ),
        );
    }));
}

/// 便捷宏。用法：`debug!("panel", "Focused({})", focused);`
#[macro_export]
macro_rules! log_debug {
    ($tag:expr, $($arg:tt)*) => {
        $crate::logger::log($crate::logger::Level::Debug, $tag, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($tag:expr, $($arg:tt)*) => {
        $crate::logger::log($crate::logger::Level::Info, $tag, &format!($($arg)*))
    };
}

#[macro_export]
#[allow(unused_macros)]
macro_rules! log_warn {
    ($tag:expr, $($arg:tt)*) => {
        $crate::logger::log($crate::logger::Level::Warn, $tag, &format!($($arg)*))
    };
}

#[macro_export]
#[allow(unused_macros)]
macro_rules! log_error {
    ($tag:expr, $($arg:tt)*) => {
        $crate::logger::log($crate::logger::Level::Error, $tag, &format!($($arg)*))
    };
}

/// 给前端 webview 用的入口：前端通过 invoke("log_from_frontend", ...) 调过来。
pub fn log_from_frontend(level: &str, tag: &str, message: &str) {
    let lvl = match level.to_ascii_lowercase().as_str() {
        "debug" => Level::Debug,
        "info" => Level::Info,
        "warn" => Level::Warn,
        "error" => Level::Error,
        _ => Level::Info,
    };
    log(lvl, tag, message);
}

/// 仅供测试用:重置 logger 状态,防止 cargo test 多用例互相污染。
#[cfg(test)]
fn _reset_for_test() {
    let logger = instance();
    *logger.file.lock().unwrap_or_else(|p| p.into_inner()) = None;
    *logger.file_path.lock().unwrap_or_else(|p| p.into_inner()) = None;
    logger.debug_enabled.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    // logger 是 OnceLock 全局单例,多个测试并发会互相覆盖 file fd / debug_enabled。
    // 用一个 test-only mutex 让 logger 相关测试串行跑,避免 cargo test 默认多线程下打架。
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn debug_is_skipped_when_env_off() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        _reset_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("pokepoke.log");
        init_with_debug(log_path.clone(), false);

        log(Level::Debug, "test", "should-not-appear");
        log(Level::Info, "test", "should-appear");

        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(
            !content.contains("should-not-appear"),
            "DEBUG leaked: {content}"
        );
        assert!(content.contains("should-appear"), "INFO missing: {content}");
    }

    #[test]
    fn debug_is_written_when_env_on() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        _reset_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("pokepoke.log");
        init_with_debug(log_path.clone(), true);

        log(Level::Debug, "test", "now-appears");

        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(content.contains("now-appears"), "DEBUG missing: {content}");
    }

    #[test]
    fn debug_enabled_from_env_recognises_truthy_values() {
        for v in ["1", "true", "TRUE", "yes", "Yes", "on", "ON"] {
            std::env::set_var("POKEPOKE_DEBUG", v);
            assert!(debug_enabled_from_env(), "expected truthy for {v}");
        }
        for v in ["0", "false", "no", "off", ""] {
            std::env::set_var("POKEPOKE_DEBUG", v);
            assert!(!debug_enabled_from_env(), "expected falsy for {v}");
        }
        std::env::remove_var("POKEPOKE_DEBUG");
        assert!(!debug_enabled_from_env(), "missing env should be falsy");
    }

    #[test]
    fn clear_log_file_truncates_to_zero() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        _reset_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("pokepoke.log");
        init_with_debug(log_path.clone(), true);

        log(Level::Info, "test", "first-line");
        assert!(std::fs::metadata(&log_path).unwrap().len() > 0);

        clear_log_file().unwrap();
        let len_after = std::fs::metadata(&log_path).unwrap().len();
        assert_eq!(len_after, 0, "expected truncated, got {len_after} bytes");

        log(Level::Info, "test", "after-clear");
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(!content.contains("first-line"));
        assert!(content.contains("after-clear"));
    }

    #[test]
    fn init_rotates_when_file_exceeds_threshold() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        _reset_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("pokepoke.log");
        let rotated_path = dir.path().join("pokepoke.log.1");

        // 预置一份超过阈值的旧日志
        let big = "x".repeat((ROTATE_THRESHOLD_BYTES + 1024) as usize);
        std::fs::write(&log_path, &big).unwrap();
        assert!(std::fs::metadata(&log_path).unwrap().len() > ROTATE_THRESHOLD_BYTES);

        init_with_debug(log_path.clone(), true);

        // 旧内容应被挪到 .1,主文件被重新打开(空,等待新写入)
        assert!(
            rotated_path.exists(),
            ".log.1 should hold the rotated content"
        );
        assert_eq!(
            std::fs::metadata(&rotated_path).unwrap().len(),
            big.len() as u64,
            "rotated file should hold the old oversized content"
        );

        // 新日志写到空的主文件,不含旧 big 内容
        log(Level::Info, "test", "fresh-after-rotate");
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(content.contains("fresh-after-rotate"));
        assert!(
            !content.contains('x'),
            "main file should be fresh, not the old big blob"
        );
    }

    #[test]
    fn init_does_not_rotate_when_file_under_threshold() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        _reset_for_test();
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("pokepoke.log");
        let rotated_path = dir.path().join("pokepoke.log.1");

        std::fs::write(&log_path, b"small").unwrap();
        init_with_debug(log_path.clone(), true);

        assert!(!rotated_path.exists(), "small file should not be rotated");
        // 原 small 内容仍在(append 模式打开,不截断)
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(content.contains("small"));
    }
}
