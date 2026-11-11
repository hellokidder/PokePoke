//! 各 agent 的外部配置路径(install/uninstall/check 共用)。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

use std::env;
use std::path::PathBuf;

pub(crate) fn home_dir() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

pub(crate) fn hook_bin_path() -> PathBuf {
    home_dir().join(".local/bin/poke-hook")
}

pub(crate) fn claude_settings_path() -> PathBuf {
    home_dir().join(".claude/settings.json")
}

pub(crate) fn codex_dir() -> PathBuf {
    home_dir().join(".codex")
}

/// Cursor 用户级全局 hooks 配置:`~/.cursor/hooks.json`。
/// 与 CC 的一键接入语义对称,不再按项目注入。
pub(crate) fn cursor_settings_path() -> PathBuf {
    home_dir().join(".cursor/hooks.json")
}

/// opencode 全局 plugin 目录:`${XDG_CONFIG_HOME:-~/.config}/opencode/plugins/`。
///
/// opencode 启动时 glob 加载 `{plugin,plugins}/*.{ts,js}`(单复数都吃,见 sst/opencode
/// `config/plugin.ts`)。实测 opencode 1.17.x 用户目录用的是**复数** `plugins/`
/// (社区 plugin 都落这里),故我们统一投放到复数目录。base 目录 honor
/// `XDG_CONFIG_HOME`(opencode 用 xdg-basedir 解析 config 根),缺省 `~/.config`。
/// 注:opencode 还有 `OPENCODE_CONFIG_DIR` 可整体覆盖 config 根,属罕见自定义,
/// 本接入不覆盖该分支——用户若设了它需手动放置 plugin(README 会提示)。
pub(crate) fn opencode_plugin_dir() -> PathBuf {
    let base = env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"));
    base.join("opencode/plugins")
}

/// PokePoke 投放的 opencode plugin 落点。
pub(crate) fn opencode_plugin_path() -> PathBuf {
    opencode_plugin_dir().join("pokepoke.js")
}

/// pi 资源目录:`~/.pi/agent`(PokePoke 侧硬编码,与既有 pi 接入保持一致;
/// pi 自身的 `PI_CODING_AGENT_DIR` 覆盖不在本接入的覆盖范围)。
pub(crate) fn pi_agent_dir() -> PathBuf {
    home_dir().join(".pi/agent")
}

/// pi 扩展投放目录:`~/.pi/agent/extensions/pokepoke`(独占目录)。
pub(crate) fn pi_extension_dir() -> PathBuf {
    pi_agent_dir().join("extensions/pokepoke")
}

/// pi 扩展落点:`<dir>/index.ts`(pi 自动发现 `<dir>/index.ts`)。
pub(crate) fn pi_extension_path() -> PathBuf {
    pi_extension_dir().join("index.ts")
}

/// omp 数据根目录名(omp 的 `CONFIG_DIR_NAME`;fork 自 pi 后由 `.pi` 改为 `.omp`)。
const OMP_CONFIG_DIR_NAME: &str = ".omp";

/// 当前 omp profile:优先 `OMP_PROFILE`,退 `PI_PROFILE`(omp 自身的兼容回退)。
/// 显式空值 = 主动选择默认 profile(与 omp `resolveProfileEnv` 一致)。
fn omp_profile() -> Option<String> {
    let raw = env::var("OMP_PROFILE")
        .ok()
        .or_else(|| env::var("PI_PROFILE").ok())?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 展开 `~/` / `~` 前缀(omp 侧 `expandTildePath` 的最小等价实现)。
fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if raw == "~" {
        return home_dir();
    }
    PathBuf::from(raw)
}

/// omp 资源目录(对齐 omp `getAgentDir()`,见 `@oh-my-pi/pi-utils/src/dirs.ts`):
/// profile 命中 → `~/.omp/profiles/<p>/agent`;否则 `$PI_CODING_AGENT_DIR` 覆盖;
/// 再否则 `~/.omp/agent`。profile 存在时 omp 会忽略 `PI_CODING_AGENT_DIR`。
///
/// 未覆盖:XDG 分支(`$XDG_DATA_HOME/omp/agent`)—— 罕见自定义,与 opencode 接入时
/// 对 `OPENCODE_CONFIG_DIR` 的处理一致,用户在 README 提示下手动放置即可。
pub(crate) fn omp_agent_dir() -> PathBuf {
    if let Some(profile) = omp_profile() {
        return home_dir()
            .join(OMP_CONFIG_DIR_NAME)
            .join("profiles")
            .join(profile)
            .join("agent");
    }
    if let Some(dir) = env::var("PI_CODING_AGENT_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return expand_tilde(dir.trim());
    }
    home_dir().join(OMP_CONFIG_DIR_NAME).join("agent")
}

/// PokePoke 投放的 omp 扩展目录(`<agent dir>/extensions/pokepoke`,独占目录)。
pub(crate) fn omp_extension_dir() -> PathBuf {
    omp_agent_dir().join("extensions/pokepoke")
}

/// omp 扩展落点:`<dir>/index.ts`(omp 自动发现扩展目录下的 `index.ts`)。
pub(crate) fn omp_extension_path() -> PathBuf {
    omp_extension_dir().join("index.ts")
}

/// Kimi 数据根:`$KIMI_CODE_HOME` 优先(与 kimi 自身解析一致),缺省 `~/.kimi-code`。
pub(crate) fn kimi_home_dir() -> PathBuf {
    env::var("KIMI_CODE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".kimi-code"))
}

pub(crate) fn kimi_config_path() -> PathBuf {
    kimi_home_dir().join("config.toml")
}

/// DSH home:`DSH_HOME` 优先(与 DSH 自身 `resolveDshHome` 一致),缺省 `~/.dsh`。
pub(crate) fn dsh_home() -> PathBuf {
    env::var("DSH_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".dsh"))
}

/// DSH profile 根目录(`$DSH_HOME/profiles`)。
pub(crate) fn dsh_profiles_dir() -> PathBuf {
    dsh_home().join("profiles")
}
