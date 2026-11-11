//! DeepSeek Harness(DSH)接入:投放 `pokepoke-dsh` cordis 插件包,并在每个
//! 已存在的 web-app profile 的 `cordis.patch.yml` 里维护 managed block。
//!
//! DSH 的接入单元不是"改一份共享配置",而是:
//!   1. 一个插件包目录(`index.js` host 半 + `client.js` 渲染半 + `package.json`),
//!      必须落在 profile 目录内 —— DSH 的 HMR 只加载 profile 内的插件文件;
//!   2. profile 的 patch 层里一条 `insert` 行,指向该目录的 `index.js`。
//!
//! host 半把 `session/event` 归一成 poke-hook 认识的事件名,渲染半负责
//! `uiWorkspace.openSession`(DSH 没有 per-session deep link,这是唯一精确入口)。
//! 详见 `docs/briefs/20261104-dsh-integration.md`。

use super::deploy::{ensure_binary_installed, print_json};
use super::paths::{dsh_profiles_dir, hook_bin_path};
use super::snapshot::record_pre_write_snapshot;
use super::transaction::WriteTransaction;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const PLUGIN_ID: &str = "pokepoke-dsh";
pub(crate) const PLUGIN_DIR_NAME: &str = "pokepoke-dsh";
pub(crate) const BLOCK_START: &str =
    "# >>> pokepoke-dsh managed block >>> do not edit; remove the whole block to uninstall";
pub(crate) const BLOCK_END: &str = "# <<< pokepoke-dsh managed block <<<";
pub(crate) const PLUGIN_MARKER: &str = "pokepoke-plugin-marker";

const PLUGIN_HOST_JS: &str = include_str!("dsh-plugin.js");
const PLUGIN_CLIENT_JS: &str = include_str!("dsh-client.js");

#[derive(Debug, Clone)]
pub(crate) struct DshProfile {
    pub name: String,
    pub dir: PathBuf,
}

/// 一个有 web 界面的 DSH profile 才需要接入(desktop / web);headless / sdk /
/// acp 没有渲染进程,装了也跳不回来。
pub(crate) fn is_web_profile(manifest: &serde_json::Value) -> bool {
    manifest["dsh"]["profile"]["bundles"]
        .as_array()
        .is_some_and(|bundles| {
            bundles
                .iter()
                .any(|b| b.as_str() == Some("@deepseek-ai/dsh-web-app"))
        })
}

/// 扫描 `$DSH_HOME/profiles` 下所有带 web bundle 的 profile,按名字排序。
pub(crate) fn dsh_profiles() -> Vec<DshProfile> {
    let Ok(entries) = fs::read_dir(dsh_profiles_dir()) else {
        return Vec::new();
    };
    let mut profiles = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(text) = fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if !is_web_profile(&manifest) {
            continue;
        }
        profiles.push(DshProfile {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir,
        });
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    profiles
}

/// 插件 spawn `poke-hook --source=dsh` 时通过环境变量传下来的本地控制通道。
/// 注意这是每次事件运行时读的,不做缓存(同一 session 可能跨 DSH 进程)。
pub(crate) fn nav_socket_from_env() -> Option<String> {
    std::env::var("POKEPOKE_DSH_NAV_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
}

pub(crate) fn plugin_dir(profile_dir: &Path) -> PathBuf {
    profile_dir.join(PLUGIN_DIR_NAME)
}

pub(crate) fn plugin_host_path(profile_dir: &Path) -> PathBuf {
    plugin_dir(profile_dir).join("index.js")
}

fn plugin_client_path(profile_dir: &Path) -> PathBuf {
    plugin_dir(profile_dir).join("client.js")
}

fn plugin_manifest_path(profile_dir: &Path) -> PathBuf {
    plugin_dir(profile_dir).join("package.json")
}

pub(crate) fn patch_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join("cordis.patch.yml")
}

/// `package.json` 是固定内容,`dsh.client` 声明渲染半的存在。
pub(crate) fn plugin_manifest() -> String {
    serde_json::json!({
        "name": PLUGIN_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "private": true,
        "type": "module",
        "exports": {
            ".": "./index.js",
            "./client": "./client.js"
        },
        "dsh": {
            "client": {
                "platform": "web",
                "immediately": true
            }
        }
    })
    .to_string()
}

/// host 半:安装时把 `__POKE_HOOK_BIN__` 替换成绝对路径(与 opencode 插件同款)。
pub(crate) fn plugin_host_contents() -> String {
    PLUGIN_HOST_JS.replace("__POKE_HOOK_BIN__", &hook_bin_path().display().to_string())
}

pub(crate) fn plugin_client_contents() -> &'static str {
    PLUGIN_CLIENT_JS
}

/// 插件内容指纹(FNV-1a):写进 managed block 的注释行。插件内容一变,block 文本
/// 就变 → DSH 的 profile watcher 才会触发 recomposition 并换掉已加载的模块。
/// 只重写插件文件而不动 patch 时,DSH 不会主动重载(2026-11-04 实测)。
pub(crate) fn plugin_fingerprint() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in plugin_manifest()
        .bytes()
        .chain(plugin_host_contents().bytes())
        .chain(plugin_client_contents().bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// `cordis.patch.yml` 的 managed block。path 用单引号包住,单引号按 YAML 规则双写。
/// `# version:` 是 YAML 注释,不参与插件加载,只用来在升级时改变 patch 内容。
pub(crate) fn managed_block(host_path: &Path) -> String {
    let path = host_path.display().to_string().replace('\'', "''");
    format!(
        "{BLOCK_START}\n# version: {}\n- insert:\n    - id: {PLUGIN_ID}\n      name: '{path}'\n{BLOCK_END}\n",
        plugin_fingerprint()
    )
}

/// 去掉我们自己的 managed block(保留用户其它内容,含相邻空行外的原样)。
/// 标记不成对(用户手改坏了 / 只删了一半)时原样返回,绝不吞掉后半份配置。
pub(crate) fn strip_managed_block(content: &str) -> String {
    let Some(start) = content.find(BLOCK_START) else {
        return content.to_string();
    };
    if content[start..].find(BLOCK_END).is_none() {
        return content.to_string();
    }
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if !skipping && line.trim() == BLOCK_START {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == BLOCK_END {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// 应用 managed block:先摘掉旧的再追加,保证重复执行幂等、路径变更可修复。
pub(crate) fn apply_managed_block(content: &str, block: &str) -> String {
    let mut base = strip_managed_block(content);
    if !base.is_empty() && !base.ends_with('\n') {
        base.push('\n');
    }
    if !base.is_empty() {
        base.push('\n');
    }
    base.push_str(block);
    base
}

/// patch 文件里是否有一条指向当前插件路径的 managed block。
pub(crate) fn patch_has_block(content: &str, host_path: &Path) -> bool {
    if !content.contains(BLOCK_START) || !content.contains(BLOCK_END) {
        return false;
    }
    let expected = host_path.display().to_string().replace('\'', "''");
    content.contains(&format!("name: '{}'", expected))
}

/// 插件包是否完整:三个文件都在,且 host/client 都带 PokePoke marker。
pub(crate) fn plugin_configured(profile_dir: &Path) -> bool {
    let host = fs::read_to_string(plugin_host_path(profile_dir)).unwrap_or_default();
    let client = fs::read_to_string(plugin_client_path(profile_dir)).unwrap_or_default();
    let manifest = fs::read_to_string(plugin_manifest_path(profile_dir)).unwrap_or_default();
    host.contains(PLUGIN_MARKER)
        && host.contains("--source=dsh")
        && client.contains(PLUGIN_MARKER)
        && manifest.contains(PLUGIN_ID)
}

/// 已投放但内容与当前内置插件不一致 → drift。比较 host + client 全文:
/// 既覆盖 hook 路径变化,也覆盖 PokePoke 发版时插件逻辑本身的升级 —— 否则
/// 用户会永远停在首次安装的那份 JS 上。
///
/// manifest 里的 version 跟随 App 版本,不参与比较(否则每次发版都误报 drift)。
pub(crate) fn plugin_outdated(profile_dir: &Path) -> bool {
    let host = fs::read_to_string(plugin_host_path(profile_dir)).unwrap_or_default();
    let client = fs::read_to_string(plugin_client_path(profile_dir)).unwrap_or_default();
    if !host.contains(PLUGIN_MARKER) {
        return false;
    }
    host != plugin_host_contents() || client != plugin_client_contents()
}

fn write_file(tx: &mut WriteTransaction, path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        record_pre_write_snapshot("dsh", path);
    }
    tx.snapshot(path);
    fs::write(path, contents).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn install_profile(profile: &DshProfile, tx: &mut WriteTransaction) -> Result<(), String> {
    let dir = plugin_dir(&profile.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
    write_file(tx, &plugin_host_path(&profile.dir), &plugin_host_contents())?;
    write_file(
        tx,
        &plugin_client_path(&profile.dir),
        plugin_client_contents(),
    )?;
    write_file(tx, &plugin_manifest_path(&profile.dir), &plugin_manifest())?;

    let patch = patch_path(&profile.dir);
    let original = fs::read_to_string(&patch).unwrap_or_default();
    let next = apply_managed_block(&original, &managed_block(&plugin_host_path(&profile.dir)));
    write_file(tx, &patch, &next)?;

    let written = fs::read_to_string(&patch).unwrap_or_default();
    if !patch_has_block(&written, &plugin_host_path(&profile.dir)) {
        return Err(format!(
            "DSH install verify failed: managed block missing in {}",
            patch.display()
        ));
    }
    if !plugin_configured(&profile.dir) {
        return Err(format!(
            "DSH install verify failed: plugin package incomplete in {}",
            dir.display()
        ));
    }
    Ok(())
}

pub(crate) fn cmd_install_dsh() {
    if !ensure_binary_installed() {
        return;
    }
    let profiles = dsh_profiles();
    if profiles.is_empty() {
        print_json(&serde_json::json!({
            "status": "error",
            "message": format!(
                "No DeepSeek Harness profile found under {} (launch DeepSeek Harness once, then retry)",
                dsh_profiles_dir().display()
            ),
        }));
        return;
    }

    let mut tx = WriteTransaction::new();
    for profile in &profiles {
        if let Err(message) = install_profile(profile, &mut tx) {
            tx.rollback();
            print_json(&serde_json::json!({
                "status": "error",
                "message": format!("DSH install failed for profile '{}' (rolled back): {}", profile.name, message),
            }));
            return;
        }
    }

    let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    print_json(&serde_json::json!({
        "status": "ok",
        "message": format!("Installed PokePoke DSH plugin in profile(s): {}", names.join(", ")),
    }));
}

pub(crate) fn cmd_uninstall_dsh() {
    let profiles = dsh_profiles();
    if profiles.is_empty() {
        print_json(&serde_json::json!({
            "status": "ok",
            "message": "No DeepSeek Harness profile found, nothing to do",
        }));
        return;
    }

    let mut tx = WriteTransaction::new();
    let mut touched = 0usize;
    for profile in &profiles {
        let dir = plugin_dir(&profile.dir);
        if dir.exists() {
            if let Err(e) = fs::remove_dir_all(&dir) {
                tx.rollback();
                print_json(&serde_json::json!({
                    "status": "error",
                    "message": format!("Failed to remove {}: {}", dir.display(), e),
                }));
                return;
            }
            touched += 1;
        }

        let patch = patch_path(&profile.dir);
        let Ok(original) = fs::read_to_string(&patch) else {
            continue;
        };
        let next = strip_managed_block(&original);
        if next == original {
            continue;
        }
        if patch.exists() {
            record_pre_write_snapshot("dsh", &patch);
        }
        tx.snapshot(&patch);
        if let Err(e) = fs::write(&patch, next) {
            tx.rollback();
            print_json(&serde_json::json!({
                "status": "error",
                "message": format!("Failed to update {}: {}", patch.display(), e),
            }));
            return;
        }
        touched += 1;
    }

    print_json(&serde_json::json!({
        "status": "ok",
        "message": if touched == 0 {
            "No PokePoke DSH plugin found, nothing to do".to_string()
        } else {
            "Removed PokePoke DSH plugin".to_string()
        },
    }));
}

pub(crate) fn cmd_check_dsh() {
    let installed = hook_bin_path().exists();
    let profiles = dsh_profiles();
    let mut configured = false;
    let mut outdated = false;
    for profile in &profiles {
        let patch = fs::read_to_string(patch_path(&profile.dir)).unwrap_or_default();
        let has_block = patch_has_block(&patch, &plugin_host_path(&profile.dir));
        if plugin_configured(&profile.dir) && has_block {
            configured = true;
            if plugin_outdated(&profile.dir) {
                outdated = true;
            }
        }
    }
    print_json(&serde_json::json!({
        "installed": installed,
        "hooks_configured": configured,
        "hooks_outdated": outdated,
        "connected": installed && configured,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "- id: ui-theme\n  config:\n    preference: dark\n";

    #[test]
    fn web_profile_requires_web_bundle() {
        let web = serde_json::json!({
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"] } }
        });
        let headless = serde_json::json!({
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-headless"] } }
        });
        assert!(is_web_profile(&web));
        assert!(!is_web_profile(&headless));
        assert!(!is_web_profile(&serde_json::json!({})));
    }

    #[test]
    fn managed_block_is_idempotent_and_moves_path() {
        let old_path = PathBuf::from("/tmp/old/index.js");
        let new_path = PathBuf::from("/tmp/new/index.js");
        // 复用同一份 block:plugin_fingerprint 含 HOME 下的 hook 路径,并行测试
        // (agent_presence / custom_sounds)会临时改 HOME,两次生成可能不同。
        let old_block = managed_block(&old_path);
        let first = apply_managed_block(PATCH, &old_block);
        assert!(patch_has_block(&first, &old_path));
        let second = apply_managed_block(&first, &old_block);
        assert_eq!(
            first, second,
            "repeated install must not duplicate the block"
        );
        let moved = apply_managed_block(&first, &managed_block(&new_path));
        assert!(patch_has_block(&moved, &new_path));
        assert!(!patch_has_block(&moved, &old_path));
        assert!(
            moved.contains("- id: ui-theme"),
            "user content must survive"
        );
    }

    #[test]
    fn strip_removes_only_our_block() {
        let with_block = apply_managed_block(PATCH, &managed_block(Path::new("/x/index.js")));
        let stripped = strip_managed_block(&with_block);
        assert_eq!(stripped, PATCH);
        assert!(!stripped.contains(BLOCK_START));
        assert!(!stripped.contains(BLOCK_END));
    }

    #[test]
    fn strip_leaves_broken_block_untouched() {
        let broken = format!("{PATCH}{BLOCK_START}\n- insert:\n");
        assert_eq!(strip_managed_block(&broken), broken);
        let no_markers = "- id: ui-theme\n";
        assert_eq!(strip_managed_block(no_markers), no_markers);
    }

    #[test]
    fn outdated_detects_plugin_logic_drift() {
        let dir = std::env::temp_dir().join(format!("dsh-outdated-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let plugin = dir.join(PLUGIN_DIR_NAME);
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("index.js"), plugin_host_contents()).unwrap();
        fs::write(plugin.join("client.js"), plugin_client_contents()).unwrap();
        assert!(!plugin_outdated(&dir), "fresh content must not be outdated");

        fs::write(
            plugin.join("client.js"),
            plugin_client_contents().replace("pokepokeDsh", "oldDsh"),
        )
        .unwrap();
        assert!(
            plugin_outdated(&dir),
            "changed client bundle must be outdated"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_block_carries_content_fingerprint() {
        // 不跨调用比较 plugin_fingerprint():它含 hook 绝对路径,而并行测试
        // (agent_presence / custom_sounds)会临时改 HOME,两次读取可能不同。
        let block = managed_block(Path::new("/x/index.js"));
        let version = block
            .lines()
            .find_map(|line| line.strip_prefix("# version: "))
            .expect("patch must carry a version line so DSH reloads on upgrade");
        assert_eq!(version.len(), 16);
        assert!(version.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(block.contains("name: '/x/index.js'"));
    }

    #[test]
    fn plugin_manifest_declares_client_half() {
        let manifest: serde_json::Value =
            serde_json::from_str(&plugin_manifest()).expect("manifest is valid json");
        assert_eq!(manifest["name"], PLUGIN_ID);
        assert_eq!(manifest["exports"]["./client"], "./client.js");
        assert_eq!(manifest["dsh"]["client"]["platform"], "web");
    }

    #[test]
    fn plugin_sources_carry_markers_and_hook_source() {
        let host = plugin_host_contents();
        assert!(host.contains(PLUGIN_MARKER));
        assert!(host.contains("--source=dsh"));
        assert!(!host.contains("__POKE_HOOK_BIN__"));
        assert!(host.contains("/poke-hook'"), "hook path must be baked in");
        assert!(plugin_client_contents().contains(PLUGIN_MARKER));
    }
}
