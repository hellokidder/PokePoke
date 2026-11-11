//! Write-then-verify transaction (L1)
//!
//! install/uninstall 写多份外部配置后,需要原子地"写完一起 verify,失败一起回滚"。
//! `WriteTransaction` 在写入前 snapshot 每个目标文件的原内容到内存,verify 失败
//! 时把所有文件回写到 snapshot 的状态(原本不存在的文件被删回去)。
//!
//! 与 `record_pre_write_snapshot` 不冲突:那是写到 ~/.pokepoke/snapshots 的
//! write-only 持久化兜底(详见 `docs/briefs/20260422-pre-write-snapshot.md`),
//! 用户手动 cp 复原用;本 transaction 是同一进程内的内存事务,失败立刻回滚。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct WriteTransaction {
    snapshots: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl WriteTransaction {
    pub(crate) fn new() -> Self {
        Self {
            snapshots: Vec::new(),
        }
    }

    /// 在调用 fs::write 之前 snapshot;原文件不存在时记 None(回滚 = 删掉新写的)。
    pub(crate) fn snapshot(&mut self, path: &Path) {
        let original = fs::read(path).ok();
        self.snapshots.push((path.to_path_buf(), original));
    }

    /// verify 失败时调,把所有 snapshot 文件回写到原内容(或删掉)。
    /// 单文件回滚失败仅 stderr warn,不阻止其他文件回滚。
    pub(crate) fn rollback(&self) {
        for (path, original) in &self.snapshots {
            match original {
                Some(bytes) => {
                    if let Err(e) = fs::write(path, bytes) {
                        eprintln!(
                            "WriteTransaction rollback: write {} failed: {}",
                            path.display(),
                            e
                        );
                    }
                }
                None => {
                    // 文件原本不存在 → 删掉新写的
                    if path.exists() {
                        if let Err(e) = fs::remove_file(path) {
                            eprintln!(
                                "WriteTransaction rollback: remove {} failed: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }
}
