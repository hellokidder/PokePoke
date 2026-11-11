//! Pre-write snapshot + `~/.pokepoke/` 目录工具(从 pokehook.rs 抽出,AGENTS.md §4a)。
//!
//! `~/.pokepoke/snapshots/<source>-<unix_ts>.<ext>` 是 PokePoke 在写三端外部
//! 配置**之前**对原文件做的副本。Write-only 语义:本模块自己不读 snapshot,
//! 只做用户手动复原的兜底凭证(见 `docs/briefs/20260422-pre-write-snapshot.md`)。
//! 原文件用 `super::home_dir()`(dual-compile 限制下 inline 复刻 transport 语义)。

use std::fs;
use std::path::{Path, PathBuf};

use super::home_dir;

// ==================== Shared pokepoke dir helpers ====================
//
// hook binary 因 dual-compile 限制不能 `use poke_poke_lib::transport`,
// 这里 inline 实现两个 `~/.pokepoke/` 目录相关的最小工具。供 pre-write
// snapshot 等子模块复用。

/// 与 `transport::ensure_dir_0700` 同语义的 inline 版本。
fn ensure_pokepoke_dir_0700(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir)?;
    let mut perms = fs::metadata(dir)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(dir, perms)
}

/// 与 `transport::default_pokepoke_dir` 同语义;dual-compile 限制下 inline 复刻。
fn pokepoke_home_dir() -> PathBuf {
    home_dir().join(".pokepoke")
}

// ==================== Pre-write snapshot ====================
//
// `~/.pokepoke/snapshots/<source>-<unix_ts>.<ext>` 是 PokePoke 在写三端外部
// 配置**之前**对原文件做的副本。目的是：当 install / uninstall / repair
// 改坏用户的 settings.json / config.toml / hooks.json 时，用户可以从
// snapshots/ 里手动捞回前一版。
//
// **Write-only 语义**：PokePoke 自己**不读** snapshot 文件——不做自动回滚、
// 不做 UI 暴露、不加 Tauri command。snapshot 是用户手动复原的兜底凭证，
// 不是 drift / outdated / managed 决策输入。
//
// 设计决策(见 `docs/briefs/20260422-pre-write-snapshot.md`):
// - 每个 source 独立保留最近 5 份,滚动清理(按文件名时间戳排序)
// - 外部配置不存在时(首次 install)静默跳过,不算失败
// - `fs::copy` 失败只 eprintln warning,不阻塞 install/uninstall 主流程
// - 扩展名按 source 分:codex-config → .toml,其余 → .json
// - source 命名:cc / codex-hooks / codex-config / cursor。codex 拆两条
//   是因为同次 install 写两份外部文件,各自需要独立的保留池

pub const SNAPSHOT_DIR_NAME: &str = "snapshots";
/// 保留份数(经 pokehook.rs re-export 供测试与外部引用)。
pub(crate) const SNAPSHOT_KEEP_PER_SOURCE: usize = 5;

pub(crate) fn snapshot_dir(pokepoke_dir: &Path) -> PathBuf {
    pokepoke_dir.join(SNAPSHOT_DIR_NAME)
}

/// 规划 snapshot 文件落点:`<pokepoke_dir>/snapshots/<source>-<ts>.<ext>`。
/// 扩展名由 source 决定:codex-config → toml;其余 → json。
pub(crate) fn plan_snapshot_path(pokepoke_dir: &Path, source: &str, unix_ts: u64) -> PathBuf {
    let ext = match source {
        "codex-config" => "toml",
        "opencode" => "js",
        "pi" | "omp" => "ts",
        _ => "json",
    };
    snapshot_dir(pokepoke_dir).join(format!("{}-{}.{}", source, unix_ts, ext))
}

/// 给定同 source 现存 snapshot 路径列表 + 保留数，返回需删除的路径。
/// 排序依据：文件名中的时间戳数值（`<source>-<digits>.<ext>`），解析失败记为 0
/// 作为最老条目先被删——这与"解析失败的遗留文件优先清理"的诊断直觉一致。
pub(crate) fn prune_snapshots_for_source(entries: &[PathBuf], keep: usize) -> Vec<PathBuf> {
    if entries.len() <= keep {
        return Vec::new();
    }
    let mut sorted: Vec<(u64, &PathBuf)> =
        entries.iter().map(|p| (parse_snapshot_ts(p), p)).collect();
    // 按 ts 升序（最老在前），前 N 个要删掉
    sorted.sort_by_key(|(ts, _)| *ts);
    let delete_count = entries.len() - keep;
    sorted
        .into_iter()
        .take(delete_count)
        .map(|(_, p)| p.clone())
        .collect()
}

/// 从文件名 `<source>-<digits>.<ext>` 里提取时间戳数值。
/// 解析失败(非法命名 / 混入第三方文件)返回 0,让 prune 把它当最老条目先删。
fn parse_snapshot_ts(path: &Path) -> u64 {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // stem 形如 "cc-1745332800" 或 "codex-hooks-1745332800"
    // 以最后一个 '-' 右侧为 ts 候选
    let ts_part = stem.rsplit_once('-').map(|(_, r)| r).unwrap_or("");
    ts_part.parse::<u64>().unwrap_or(0)
}

/// 在写外部配置前对现有文件做一次 snapshot。
///
/// 返回：
/// - `Ok(None)`：src_path 不存在（首次 install），合法跳过
/// - `Ok(Some(dst))`：snapshot 已落地到 dst，同 source 超额条目已清理
/// - `Err(e)`：copy 失败；调用方决定降级（`record_pre_write_snapshot` wrapper 吞掉）
pub(crate) fn write_pre_write_snapshot(
    pokepoke_dir: &Path,
    source: &str,
    src_path: &Path,
) -> std::io::Result<Option<PathBuf>> {
    if !src_path.exists() {
        return Ok(None);
    }

    ensure_pokepoke_dir_0700(pokepoke_dir)?;
    let dir = snapshot_dir(pokepoke_dir);
    fs::create_dir_all(&dir)?;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dst = plan_snapshot_path(pokepoke_dir, source, now_secs);

    fs::copy(src_path, &dst)?;

    // 写完再清理超额：先列当前同 source 全部文件，再调纯函数计算删哪几份
    let existing = list_snapshots_for_source(&dir, source);
    let to_delete = prune_snapshots_for_source(&existing, SNAPSHOT_KEEP_PER_SOURCE);
    for p in to_delete {
        let _ = fs::remove_file(&p);
    }

    Ok(Some(dst))
}

/// 列出 snapshot 目录下同 source 的全部文件(按 prefix 精确匹配,以 "-<digits>"
/// 为边界,防止 "cc" 错匹"cc-extra" 这类潜在子前缀)。
fn list_snapshots_for_source(dir: &Path, source: &str) -> Vec<PathBuf> {
    let needle = format!("{}-", source);
    fs::read_dir(dir)
        .map(|iter| {
            iter.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name().to_string_lossy().starts_with(&needle)
                        && source_boundary_match(&e.file_name().to_string_lossy(), source)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// 判断 `name` 是否严格以 `<source>-<digits>` 为 prefix。
/// 防止潜在子前缀(如未来若加 "cc-extra" 这类 source)误匹配 "cc"。
fn source_boundary_match(name: &str, source: &str) -> bool {
    let needle = format!("{}-", source);
    if !name.starts_with(&needle) {
        return false;
    }
    // needle 后面第一个 char 必须是数字；否则说明后面跟的是子 source 字母
    let after = &name[needle.len()..];
    after.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// install / uninstall 写外部配置前的 snapshot 包装：
/// 失败仅 eprintln warning，不向上传播，不阻塞主流程。
pub(crate) fn record_pre_write_snapshot(source: &str, src_path: &Path) {
    let pokepoke_dir = pokepoke_home_dir();
    if let Err(e) = write_pre_write_snapshot(&pokepoke_dir, source, src_path) {
        eprintln!(
            "[poke-hook] warning: pre-write snapshot for {} ({}) failed: {}",
            source,
            src_path.display(),
            e
        );
    }
}
