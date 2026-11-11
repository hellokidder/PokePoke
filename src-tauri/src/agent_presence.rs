//! 检测本机是否已安装各 managed agent,供首启 / auto-connect 决定是否写 hook。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// 给定 source 是否应视为「本机已安装」。
pub fn is_agent_present(source: &str) -> bool {
    match source {
        "cc" => cli_tool_present("claude"),
        "codex" => cli_tool_present("codex"),
        "cursor" => cursor_present(),
        "opencode" => cli_tool_present("opencode"),
        "pi" => pi_present(),
        "omp" => omp_present(),
        "kimi" => kimi_present(),
        "grok" => grok_present(),
        "dsh" => dsh_present(),
        _ => false,
    }
}

fn cli_tool_present(name: &str) -> bool {
    resolve_cli_binary(name).is_some()
}

/// 解析 CLI 二进制完整路径（common paths → PATH which → login shell），供 spawn 使用。
/// 与 `cli_tool_present` 同源，但返回路径而非布尔。
/// 沙箱（`POKEPOKE_AGENT_PRESENCE_ROOT` 生效）下只查 `{root}/.local/bin`，不查宿主
/// which / login shell（47924b4 曾把 sandbox 检出整体短路，04-first-run 三端安装回归）。
pub(crate) fn resolve_cli_binary(name: &str) -> Option<PathBuf> {
    for path in common_cli_paths(name) {
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    if optional_presence_root().is_some() {
        return None;
    }
    if let Some(path) = which_path(name) {
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    // GUI 从 Dock/Spotlight 启动时,launchd 给的 PATH 是极简的 /usr/bin:/bin:...,
    // 拿不到 nvm/volta/fnm/pnpm/yarn/asdf 的目录。这里跑一次 login+interactive
    // shell,让用户 rc 文件里配的 PATH 生效。仅生产环境启用,沙箱测试不走。
    if let Some(path) = login_shell_which(name) {
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    None
}

/// 沙箱隔离是否激活（E2E 测试设置 `POKEPOKE_AGENT_PRESENCE_ROOT`）。
pub(crate) fn sandbox_root_active() -> bool {
    optional_presence_root().is_some()
}

fn cursor_present() -> bool {
    if cli_tool_present("cursor") {
        return true;
    }
    for path in cursor_app_paths() {
        if path.is_dir() {
            return true;
        }
    }
    false
}

/// pi 安装痕迹:配置目录存在即视为已安装(fnm/源码装 PATH 不稳定,不做 which 依赖)。
fn pi_present() -> bool {
    if let Some(root) = optional_presence_root() {
        return root.join(".pi/agent").is_dir();
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".pi/agent").is_dir())
        .unwrap_or(false)
}

/// kimi 安装痕迹:默认装到 `~/.kimi-code/bin/kimi` 并把自己的 bin 加进 PATH。
/// 先 cli_tool_present(覆盖 which / login-shell PATH / 常见 bin 目录),再兜底
/// 默认安装位 `~/.kimi-code/bin/kimi` 文件存在(与 pi 一样不纯靠 which)。
fn kimi_present() -> bool {
    if let Some(root) = optional_presence_root() {
        return root.join(".kimi-code").is_dir();
    }
    if cli_tool_present("kimi") {
        return true;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".kimi-code/bin/kimi").is_file())
        .unwrap_or(false)
}

/// grok 安装痕迹:官方装到 `~/.local/bin/grok` 或 `~/.grok/` 配置目录。
fn grok_present() -> bool {
    if let Some(root) = optional_presence_root() {
        return root.join(".grok").is_dir();
    }
    if cli_tool_present("grok") {
        return true;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".grok").is_dir())
        .unwrap_or(false)
}

/// DSH 安装痕迹:桌面 App 或 `~/.dsh/profiles` 存在。DSH 没有独立 PATH 入口
/// (CLI 内嵌在 Electron app 里),所以不查 which。
fn dsh_present() -> bool {
    if let Some(root) = optional_presence_root() {
        return root.join(".dsh/profiles").is_dir();
    }
    if Path::new("/Applications/DeepSeek Harness.app").is_dir() {
        return true;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".dsh/profiles").is_dir())
        .unwrap_or(false)
}

/// omp 安装痕迹(omp 是 pi 的 fork,CLI 名 `omp`,数据根 `~/.omp`):
/// 先看 CLI 是否在 PATH / 常见安装位 / login shell 里(fnm、bun 全局装常见
/// PATH 不稳定,与 kimi 同思路),再兜底数据根 `~/.omp` 存在。
fn omp_present() -> bool {
    if let Some(root) = optional_presence_root() {
        return root.join(".omp").is_dir();
    }
    if cli_tool_present("omp") {
        return true;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".omp").is_dir())
        .unwrap_or(false)
}

fn which_path(name: &str) -> Option<PathBuf> {
    for which in ["/usr/bin/which", "which"] {
        let output = Command::new(which).arg(name).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// login+interactive shell 兜底:让 `~/.zshrc` / `~/.bash_profile` 里配的
/// nvm/volta/fnm/asdf/pnpm/yarn 目录生效。仅在别的探测全失败时才走。
///
/// - 2s 硬超时:防止用户 rc 文件卡死(网络请求 / 冗长初始化)拖垮首启。
/// - 名字白名单:只允许 [a-zA-Z0-9_-],防止 shell 元字符注入。
/// - 输出校验:必须是绝对路径且指向可执行文件,过滤掉 alias/function/builtin。
fn login_shell_which(name: &str) -> Option<PathBuf> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || name.is_empty()
    {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    // 用 `-l -i -c` 走 login+interactive 让 zshrc 里的 PATH 补齐生效。
    // `command -v` 会把 alias/function 也一并输出,后面用 is_executable_file 兜住。
    let script = format!("command -v {name} 2>/dev/null");
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = stdout;
        let mut buf = String::new();
        let ok = reader.read_to_string(&mut buf).is_ok();
        let _ = tx.send(if ok { Some(buf) } else { None });
    });
    let output = match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Some(s)) => s,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let _ = child.wait();
    let path = output.trim();
    if path.is_empty() || !path.starts_with('/') {
        // 空 / 相对路径 / alias 定义(`name: aliased to ...`)一律丢弃。
        return None;
    }
    Some(PathBuf::from(path))
}

/// E2E / 沙箱隔离:仅在该 HOME 树下查 `~/.local/bin` 与 `~/Applications/Cursor.app`,
/// 跳过 Homebrew、`which` 与系统 `/Applications`。生产环境不设此变量。
fn optional_presence_root() -> Option<PathBuf> {
    std::env::var("POKEPOKE_AGENT_PRESENCE_ROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn common_cli_paths(name: &str) -> Vec<PathBuf> {
    if let Some(root) = optional_presence_root() {
        return vec![root.join(".local/bin").join(name)];
    }
    let mut paths = vec![
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
    ];
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".local/bin").join(name));
        paths.push(home.join(".cargo/bin").join(name));
    }
    paths
}

fn cursor_app_paths() -> Vec<PathBuf> {
    if let Some(root) = optional_presence_root() {
        return vec![root.join("Applications/Cursor.app")];
    }
    let mut paths = vec![PathBuf::from("/Applications/Cursor.app")];
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join("Applications/Cursor.app"));
    }
    paths
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    struct EnvGuard {
        home: Option<String>,
        path: Option<String>,
        presence_root: Option<String>,
    }

    impl EnvGuard {
        fn set(home: &Path, path: &Path) -> Self {
            let guard = Self {
                home: std::env::var("HOME").ok(),
                path: std::env::var("PATH").ok(),
                presence_root: std::env::var("POKEPOKE_AGENT_PRESENCE_ROOT").ok(),
            };
            // SAFETY: single-threaded unit tests; restored on Drop.
            unsafe {
                std::env::set_var("HOME", home);
                std::env::set_var("PATH", path);
                std::env::remove_var("POKEPOKE_AGENT_PRESENCE_ROOT");
            }
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.home {
                    Some(h) => std::env::set_var("HOME", h),
                    None => std::env::remove_var("HOME"),
                }
                match &self.path {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
                match &self.presence_root {
                    Some(r) => std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", r),
                    None => std::env::remove_var("POKEPOKE_AGENT_PRESENCE_ROOT"),
                }
            }
        }
    }

    fn touch_exe(dir: &Path, name: &str) {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").expect("fs::write 失败");
        let mut perms = fs::metadata(&path)
            .expect("fs::metadata 失败")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set_permissions 失败");
    }

    #[test]
    #[serial]
    fn detects_cli_in_home_local_bin() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        touch_exe(&bin, "claude");
        let _guard = EnvGuard::set(dir.path(), &bin);
        assert!(cli_tool_present("claude"));
    }

    #[test]
    #[serial]
    fn detects_cursor_app_bundle_directory() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let app = dir.path().join("Applications/Cursor.app");
        fs::create_dir_all(&app).expect("create_dir_all 失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(dir.path(), &bin);
        assert!(cursor_present());
    }

    #[test]
    fn unknown_source_is_not_present() {
        assert!(!is_agent_present("warp"));
    }

    #[test]
    fn login_shell_which_rejects_shell_metachars() {
        // 防注入白名单:含空格 / 分号 / $ 一律拒绝,避免拼进 sh -c 后执行任意脚本。
        assert!(login_shell_which("claude; rm -rf /").is_none());
        assert!(login_shell_which("claude foo").is_none());
        assert!(login_shell_which("$(whoami)").is_none());
        assert!(login_shell_which("").is_none());
    }

    #[test]
    #[serial]
    fn login_shell_which_finds_tool_via_login_shell() {
        // 用 dummy shell 模拟一个只在 login shell 才 export 的 PATH,验证兜底能命中。
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        touch_exe(&bin, "faketool");

        // 假 shell:接受 -l -i -c '<cmd>' 参数,把最后一个 arg 当 sh 脚本跑,PATH 硬编码到 tempdir/bin。
        let fake_shell = dir.path().join("fake-shell");
        let bin_str = bin.to_string_lossy();
        fs::write(
            &fake_shell,
            format!("#!/bin/sh\nexport PATH={bin_str}\nexec /bin/sh -c \"$4\"\n"),
        )
        .expect("预期成功(测试环境)");
        let mut perms = fs::metadata(&fake_shell)
            .expect("fs::metadata 失败")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_shell, perms).expect("set_permissions 失败");

        let prev_shell = std::env::var("SHELL").ok();
        unsafe {
            std::env::set_var("SHELL", &fake_shell);
        }
        let found = login_shell_which("faketool");
        unsafe {
            match prev_shell {
                Some(s) => std::env::set_var("SHELL", s),
                None => std::env::remove_var("SHELL"),
            }
        }
        assert_eq!(found.as_deref(), Some(bin.join("faketool").as_path()));
    }

    #[test]
    #[serial]
    fn detects_seeded_codex_when_home_and_path_isolated() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        touch_exe(&bin, "codex");
        let _guard = EnvGuard::set(dir.path(), &bin);
        assert!(is_agent_present("codex"));
    }

    #[test]
    #[serial]
    fn resolve_cli_binary_returns_seeded_path_in_home_local_bin() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let home = dir.path();
        let local_bin = home.join(".local/bin");
        fs::create_dir_all(&local_bin).expect("create_dir_all 失败");
        touch_exe(&local_bin, "codex");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(home, &bin_dir);
        // 沙箱下 resolve_cli_binary 与 cli_tool_present 同源：home/.local/bin 命中
        assert_eq!(
            resolve_cli_binary("codex").as_deref(),
            Some(local_bin.join("codex").as_path()),
        );
        // 未播种的名字不该被解析出来。不能裸断言 None:`common_cli_paths()` 里
        // `/opt/homebrew/bin` / `/usr/local/bin` 是无条件候选(设计如此——GUI 极简 PATH
        // 下靠它们找 CLI),开发机装了 claude 就会命中,断言会随宿主环境飘红。
        // 只有 presence root(测试沙箱)生效时才真正与宿主隔离。
        unsafe {
            std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", home);
        }
        assert_eq!(resolve_cli_binary("claude"), None);
    }

    #[test]
    #[serial]
    fn presence_root_ignores_host_homebrew_when_sandbox_empty() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        let local_bin = dir.path().join(".local/bin");
        fs::create_dir_all(&local_bin).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(dir.path(), &bin);
        unsafe {
            std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", dir.path());
        }
        assert!(!is_agent_present("cc"));
        assert!(!is_agent_present("codex"));
        assert!(!cursor_present());
    }

    #[test]
    #[serial]
    fn presence_root_detects_seeded_local_bin_cli() {
        // 04-first-run-install 语义：sandbox 内 seed ~/.local/bin/{claude,codex,opencode}
        // 应被检出并 auto-install（47924b4 曾把 sandbox 检出整体短路导致 04 回归）。
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let local_bin = dir.path().join(".local/bin");
        fs::create_dir_all(&local_bin).expect("create_dir_all 失败");
        touch_exe(&local_bin, "claude");
        touch_exe(&local_bin, "codex");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(dir.path(), &bin);
        unsafe {
            std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", dir.path());
        }
        assert!(is_agent_present("cc"));
        assert!(is_agent_present("codex"));
        // sandbox 无 seed 的源不因宿主 which/login-shell 误报
        assert!(!is_agent_present("opencode"));
        // appserver 在 sandbox 下不应解析到 fake（spawn 由 codex_appserver 侧门控）
        assert!(resolve_cli_binary("codex").is_some());
    }

    #[test]
    #[serial]
    fn detects_pi_config_dir() {
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let pi_agent = dir.path().join(".pi/agent");
        fs::create_dir_all(&pi_agent).expect("create_dir_all 失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(dir.path(), &bin);
        unsafe {
            std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", dir.path());
        }
        assert!(is_agent_present("pi"));
    }

    #[test]
    #[serial]
    fn detects_omp_config_root() {
        // omp 数据根 `~/.omp`(CLI 在 PATH 不稳定时也能靠目录痕迹检出)
        let dir = tempfile::tempdir().expect("tempdir 创建失败");
        let omp_root = dir.path().join(".omp/agent");
        fs::create_dir_all(&omp_root).expect("create_dir_all 失败");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("create_dir_all 失败");
        let _guard = EnvGuard::set(dir.path(), &bin);
        unsafe {
            std::env::set_var("POKEPOKE_AGENT_PRESENCE_ROOT", dir.path());
        }
        assert!(is_agent_present("omp"));
        // 未经 seed 的源不因宿主 which 误报
        assert!(!is_agent_present("kimi"));
    }
}
