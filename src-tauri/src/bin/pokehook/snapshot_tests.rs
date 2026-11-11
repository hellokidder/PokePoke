use super::super::snapshot::{
    plan_snapshot_path, prune_snapshots_for_source, snapshot_dir, write_pre_write_snapshot,
    SNAPSHOT_KEEP_PER_SOURCE,
};
use std::path::PathBuf;
use tempfile::tempdir;

// ==================== Pre-write snapshot tests ====================
// 详见 `docs/briefs/20260422-pre-write-snapshot.md`。
// 这组测试覆盖：
//   - plan_snapshot_path：路径规划（source + ts + ext）
//   - prune_snapshots_for_source：保留策略（按文件名时间戳排序，保留最新 N 份）
//   - write_pre_write_snapshot：IO 端到端（文件不存在兜底 / 累加 / 轮转 / 多 source 独立）

#[test]
fn plan_snapshot_path_cc_uses_json_extension() {
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().to_path_buf();
    let path = plan_snapshot_path(&pokepoke_dir, "cc", 1745332800);
    assert_eq!(
        path,
        pokepoke_dir.join("snapshots").join("cc-1745332800.json")
    );
}

#[test]
fn plan_snapshot_path_codex_config_uses_toml_extension() {
    let dir = tempdir().unwrap();
    let path_cc = plan_snapshot_path(dir.path(), "cc", 100);
    assert!(path_cc.to_string_lossy().ends_with("cc-100.json"));

    let path_codex_config = plan_snapshot_path(dir.path(), "codex-config", 100);
    assert!(path_codex_config
        .to_string_lossy()
        .ends_with("codex-config-100.toml"));

    let path_codex_hooks = plan_snapshot_path(dir.path(), "codex-hooks", 100);
    assert!(path_codex_hooks
        .to_string_lossy()
        .ends_with("codex-hooks-100.json"));

    let path_cursor = plan_snapshot_path(dir.path(), "cursor", 100);
    assert!(path_cursor.to_string_lossy().ends_with("cursor-100.json"));
}

#[test]
fn prune_snapshots_for_source_keeps_newest_n() {
    // 给 6 个 cc snapshot，保留 5 份 → 返回最老的 1 个
    let entries: Vec<PathBuf> = vec![
        PathBuf::from("cc-1000.json"),
        PathBuf::from("cc-1005.json"),
        PathBuf::from("cc-1003.json"),
        PathBuf::from("cc-1001.json"),
        PathBuf::from("cc-1004.json"),
        PathBuf::from("cc-1002.json"),
    ];
    let to_delete = prune_snapshots_for_source(&entries, 5);
    assert_eq!(to_delete.len(), 1);
    assert_eq!(to_delete[0], PathBuf::from("cc-1000.json"));
}

#[test]
fn prune_snapshots_for_source_noop_when_under_limit() {
    let entries: Vec<PathBuf> = vec![
        PathBuf::from("cc-1.json"),
        PathBuf::from("cc-2.json"),
        PathBuf::from("cc-3.json"),
    ];
    let to_delete = prune_snapshots_for_source(&entries, 5);
    assert!(to_delete.is_empty());
}

#[test]
fn prune_snapshots_for_source_handles_malformed_names() {
    // 文件名不含时间戳的不参与比较（解析失败记为 0），会被当成最老的先删
    let entries: Vec<PathBuf> = vec![
        PathBuf::from("cc-2.json"),
        PathBuf::from("cc-malformed.json"),
        PathBuf::from("cc-1.json"),
    ];
    let to_delete = prune_snapshots_for_source(&entries, 2);
    // 保留 cc-2 + cc-1；删 cc-malformed
    assert_eq!(to_delete.len(), 1);
    assert_eq!(to_delete[0], PathBuf::from("cc-malformed.json"));
}

#[test]
fn write_pre_write_snapshot_returns_none_when_src_missing() {
    // 首次 install 场景：外部配置文件尚不存在，snapshot 合法跳过
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    let src = dir.path().join("nonexistent-settings.json");
    let result = write_pre_write_snapshot(&pokepoke_dir, "cc", &src).unwrap();
    assert!(result.is_none());
    // 没 src 时 snapshot 目录也不应被创建（没必要的空目录）
    assert!(!snapshot_dir(&pokepoke_dir).exists());
}

#[test]
fn write_pre_write_snapshot_copies_content_and_returns_path() {
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    let src = dir.path().join("settings.json");
    let original = r#"{"hooks":{"SessionStart":[]}}"#;
    std::fs::write(&src, original).unwrap();

    let result = write_pre_write_snapshot(&pokepoke_dir, "cc", &src).unwrap();
    let snap_path = result.expect("snapshot should be created");
    assert!(snap_path.exists());
    assert_eq!(std::fs::read_to_string(&snap_path).unwrap(), original);
    // 落点应在 snapshots/ 子目录下
    assert_eq!(snap_path.parent().unwrap(), snapshot_dir(&pokepoke_dir));
    // 文件名以 cc- 开头，.json 结尾
    let name = snap_path.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("cc-"));
    assert!(name.ends_with(".json"));
}

#[test]
fn write_pre_write_snapshot_codex_dual_files_produce_separate_snapshots() {
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    let hooks_src = dir.path().join("hooks.json");
    let config_src = dir.path().join("config.toml");
    std::fs::write(&hooks_src, r#"{"hooks":{}}"#).unwrap();
    std::fs::write(&config_src, "hooks = \"./hooks.json\"").unwrap();

    let hooks_snapshot = write_pre_write_snapshot(&pokepoke_dir, "codex-hooks", &hooks_src)
        .unwrap()
        .expect("hooks snapshot");
    let config_snapshot = write_pre_write_snapshot(&pokepoke_dir, "codex-config", &config_src)
        .unwrap()
        .expect("config snapshot");

    assert!(hooks_snapshot.to_string_lossy().ends_with(".json"));
    assert!(config_snapshot.to_string_lossy().ends_with(".toml"));
    assert_ne!(hooks_snapshot, config_snapshot);
    assert!(hooks_snapshot.exists() && config_snapshot.exists());
}

#[test]
fn write_pre_write_snapshot_rotates_when_exceeding_keep_limit() {
    // 预置超额的旧 snapshot 文件，触发 write 后清理
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    std::fs::create_dir_all(snapshot_dir(&pokepoke_dir)).unwrap();

    // 预置 5 份旧 snapshot（ts 1..=5）
    for ts in 1..=5 {
        let p = snapshot_dir(&pokepoke_dir).join(format!("cc-{}.json", ts));
        std::fs::write(&p, format!("content-{}", ts)).unwrap();
    }

    // 写第 6 份 → 触发 prune，最老的 cc-1.json 被删
    let src = dir.path().join("settings.json");
    std::fs::write(&src, "new-content").unwrap();
    let _result = write_pre_write_snapshot(&pokepoke_dir, "cc", &src).unwrap();

    let remaining: Vec<String> = std::fs::read_dir(snapshot_dir(&pokepoke_dir))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("cc-"))
        .collect();
    assert_eq!(
        remaining.len(),
        SNAPSHOT_KEEP_PER_SOURCE,
        "should keep exactly {} cc snapshots after prune",
        SNAPSHOT_KEEP_PER_SOURCE
    );
    assert!(
        !remaining.iter().any(|n| n == "cc-1.json"),
        "oldest snapshot should be pruned; remaining: {:?}",
        remaining
    );
}

#[test]
fn write_pre_write_snapshot_isolates_sources() {
    // 不同 source 各自独立 5 份保留池，不互相污染
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    std::fs::create_dir_all(snapshot_dir(&pokepoke_dir)).unwrap();

    // 预置 cc 5 份 + cursor 5 份
    for ts in 1..=5 {
        let cc = snapshot_dir(&pokepoke_dir).join(format!("cc-{}.json", ts));
        let cur = snapshot_dir(&pokepoke_dir).join(format!("cursor-{}.json", ts));
        std::fs::write(&cc, "cc-old").unwrap();
        std::fs::write(&cur, "cursor-old").unwrap();
    }

    // 只写 cc 一次 → cursor 应不动
    let src = dir.path().join("settings.json");
    std::fs::write(&src, "cc-new").unwrap();
    write_pre_write_snapshot(&pokepoke_dir, "cc", &src).unwrap();

    let cursor_count = std::fs::read_dir(snapshot_dir(&pokepoke_dir))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("cursor-"))
        .count();
    assert_eq!(cursor_count, 5, "cursor snapshots should not be pruned");
}

#[test]
fn write_pre_write_snapshot_creates_parent_dir_0700() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let pokepoke_dir = dir.path().join(".pokepoke");
    let src = dir.path().join("settings.json");
    std::fs::write(&src, "x").unwrap();

    write_pre_write_snapshot(&pokepoke_dir, "cc", &src).unwrap();

    let mode = std::fs::metadata(&pokepoke_dir)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, ".pokepoke 父目录必须 0700");
}
