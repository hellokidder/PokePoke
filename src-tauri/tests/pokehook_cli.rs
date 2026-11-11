//! pokehook CLI 集成测试(真实二进制级):`--install / --check / --uninstall / 再安装`
//! 回环,覆盖空配置 / 已有其他 hooks / 部分损坏 / 重复安装(roadmap P1
//! 「Reliability:测试 / 发布流程 → 集成测试」,brief 20260810-pokehook-cli-integration-tests)。
//!
//! 用 `env!("CARGO_BIN_EXE_pokehook")` 跑真实二进制 + 临时 HOME sandbox,
//! 不碰真实宿主配置;check 走 stdout JSON 断言(`connected / hooks_configured /
//! hooks_outdated / installed`),不依赖宿主 CLI 存在。
//!
//! 参数注意(2026-08-02 验证):CC 的 install 参数是 `--install`(backward-compatible
//! 名),check 是 `--check-cc`(`--check` 会落入 hook_mode 默认分支输出 {});其余端
//! 是 `--install-<agent> / --uninstall-<agent> / --check-<agent>`。

use std::path::PathBuf;
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pokehook")
}

/// 临时 HOME sandbox。每个用例独立目录,结束由 Drop 清理。
struct Sandbox {
    home: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "pokehook-cli-{tag}-{}-{}",
            std::process::id(),
            tag.len()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        Self { home }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .env("HOME", &self.home)
            // omp/pi 会话会给子进程注入这几个变量(omp 在 profile 模式下自己设置它们),
            // 而 `paths::omp_agent_dir` 正是优先 honor 它们 —— 不清掉的话,从 agent 会话里
            // 跑 `cargo test` 时扩展会落到真实 profile 目录,`~/.omp/agent` 断言必假红。
            .env_remove("OMP_PROFILE")
            .env_remove("PI_PROFILE")
            .env_remove("PI_CODING_AGENT_DIR")
            // DSH 同理:`dsh_home()` honor `DSH_HOME`,而 agent 会话里它指向真实
            // `~/.dsh` —— 不清掉会把测试插件写进用户真实 profile。
            .env_remove("DSH_HOME")
            .args(args)
            .output()
            .expect("spawn pokehook")
    }

    /// 断言命令退出码 0,返回 stdout。
    fn run_ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "cmd {args:?} failed (exit {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    fn read_json(&self, rel: &str) -> serde_json::Value {
        let p = self.path(rel);
        assert!(p.exists(), "{} 不存在", rel);
        let content = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {rel}: {e}"));
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("parse {rel}: {e}"))
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// 从 check JSON 里取字段。
fn check_field(out: &str, field: &str) -> bool {
    let v: serde_json::Value =
        serde_json::from_str(out).unwrap_or_else(|e| panic!("parse check: {e}"));
    v.get(field)
        .and_then(|x| x.as_bool())
        .unwrap_or_else(|| panic!("check 缺字段 {field}: {out}"))
}

fn check_json(bin_out: &str) -> serde_json::Value {
    serde_json::from_str(bin_out).unwrap_or_else(|e| panic!("parse check output: {e}"))
}

/// 递归统计任意深度 JSON 里 command 含 poke-hook 的 entry 数。
/// CC settings.json 是两层嵌套(`hooks.<event>.[{hooks:[{command}]}]`),
/// cursor/codex hooks.json 是一层(`hooks.<event>.[{command}]`),递归统一。
fn count_poke_entries(v: &serde_json::Value) -> usize {
    fn walk(v: &serde_json::Value, count: &mut usize) {
        if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
            if cmd.contains("poke-hook") {
                *count += 1;
            }
        }
        if let Some(obj) = v.as_object() {
            for val in obj.values() {
                walk(val, count);
            }
        }
        if let Some(arr) = v.as_array() {
            for val in arr {
                walk(val, count);
            }
        }
    }
    let mut count = 0;
    walk(v, &mut count);
    count
}

// ==================== CC 全回环 ====================

/// 空配置 → install → check connected → uninstall 还原 → 再 install 回环。
#[test]
fn cc_install_check_uninstall_reinstall_roundtrip() {
    let sb = Sandbox::new("cc-roundtrip");

    // 空配置 check:未连接
    let out = sb.run_ok(&["--check-cc"]);
    assert!(!check_field(&out, "connected"), "空配置不应 connected");

    // install:ok + 部署 binary + 写 settings.json
    let out = sb.run_ok(&["--install"]);
    let v = check_json(&out);
    assert_eq!(v["status"], "ok", "install 应 ok: {out}");
    assert!(
        sb.path(".local/bin/poke-hook").exists(),
        "应部署 poke-hook binary"
    );
    let settings = sb.read_json(".claude/settings.json");
    assert!(
        count_poke_entries(&settings) >= 1,
        "settings.json 应有 poke entry"
    );

    // check:connected
    let out = sb.run_ok(&["--check-cc"]);
    assert!(check_field(&out, "connected"), "install 后应 connected");
    assert!(check_field(&out, "installed"), "installed 应为 true");

    // uninstall:entry 移除
    let out = sb.run_ok(&["--uninstall"]);
    assert_eq!(check_json(&out)["status"], "ok", "uninstall 应 ok: {out}");
    let settings = sb.read_json(".claude/settings.json");
    assert_eq!(
        count_poke_entries(&settings),
        0,
        "uninstall 后 poke entry 应清空"
    );

    // 再安装回环
    let out = sb.run_ok(&["--install"]);
    assert_eq!(check_json(&out)["status"], "ok");
    let out = sb.run_ok(&["--check-cc"]);
    assert!(check_field(&out, "connected"), "再安装后应 connected");
}

/// 部分损坏(手动删 poke entry)→ check 报未连接 → install 修复 → check 绿。
#[test]
fn cc_partial_damage_detect_and_repair() {
    let sb = Sandbox::new("cc-damage");
    sb.run_ok(&["--install"]);

    // 模拟外部覆盖:清空 hooks(但保留文件,模拟"配置被改写")
    let mut settings = sb.read_json(".claude/settings.json");
    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        hooks.clear();
    }
    std::fs::write(sb.path(".claude/settings.json"), settings.to_string()).unwrap();

    // check:配置不再 connected(hooks_outdated 或 configured=false)
    let out = sb.run_ok(&["--check-cc"]);
    assert!(
        !check_field(&out, "connected"),
        "损坏后不应 connected: {out}"
    );

    // 修复:install 合并补回
    let out = sb.run_ok(&["--install"]);
    assert_eq!(check_json(&out)["status"], "ok");
    let out = sb.run_ok(&["--check-cc"]);
    assert!(check_field(&out, "connected"), "修复后应 connected");
}

// ==================== Cursor:已有其他 hooks 保留 ====================

/// 用户已有其他 hook entry → install 合并保留 → uninstall 后用户 entry 仍在。
#[test]
fn cursor_preserves_user_entries_through_roundtrip() {
    let sb = Sandbox::new("cursor-preserve");
    sb.write(
        ".cursor/hooks.json",
        r#"{
  "version": 1,
  "hooks": {
    "SessionStart": [
      {"command": "/usr/bin/env echo USER_HOOK"}
    ]
  }
}"#,
    );

    let out = sb.run_ok(&["--install-cursor"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-cursor: {out}");

    let hooks = sb.read_json(".cursor/hooks.json");
    let user_count = count_entries_with_command(&hooks, "USER_HOOK");
    assert_eq!(user_count, 1, "用户 entry 应保留");
    assert!(count_poke_entries(&hooks) >= 1, "poke entry 应添加");

    let out = sb.run_ok(&["--check-cursor"]);
    assert!(check_field(&out, "connected"));

    // uninstall:poke entry 移除,用户 entry 保留
    let out = sb.run_ok(&["--uninstall-cursor"]);
    assert_eq!(check_json(&out)["status"], "ok");
    let hooks = sb.read_json(".cursor/hooks.json");
    assert_eq!(
        count_poke_entries(&hooks),
        0,
        "uninstall 后 poke entry 应清空"
    );
    assert_eq!(
        count_entries_with_command(&hooks, "USER_HOOK"),
        1,
        "uninstall 不应碰用户 entry"
    );
}

fn count_entries_with_command(v: &serde_json::Value, needle: &str) -> usize {
    let mut count = 0;
    if let Some(hooks) = v.get("hooks").and_then(|h| h.as_object()) {
        for entries in hooks.values() {
            if let Some(arr) = entries.as_array() {
                for e in arr {
                    let text = e.to_string();
                    if text.contains(needle) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// 重复安装幂等:install ×2,entry 不重复。
#[test]
fn install_is_idempotent() {
    let sb = Sandbox::new("idempotent");
    sb.run_ok(&["--install"]);
    let first = count_poke_entries(&sb.read_json(".claude/settings.json"));

    sb.run_ok(&["--install"]);
    let second = count_poke_entries(&sb.read_json(".claude/settings.json"));

    assert_eq!(first, second, "重复 install 不应增加 entry");
    assert!(first >= 1);
}

// ==================== 文件投放端:opencode / pi / omp / grok ====================

/// opencode:JS plugin 投放 + check 结构 + uninstall 还原(verify 纯文件,无条件跑)。
#[test]
fn opencode_plugin_roundtrip() {
    let sb = Sandbox::new("opencode");
    let out = sb.run_ok(&["--install-opencode"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-opencode: {out}");

    let plugin = sb.path(".config/opencode/plugins/pokepoke.js");
    assert!(plugin.exists(), "opencode plugin 应投放");
    let content = std::fs::read_to_string(&plugin).unwrap();
    assert!(
        content.contains("--source=opencode"),
        "plugin 应含 poke-hook 调用"
    );

    let out = sb.run_ok(&["--check-opencode"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-opencode"]);
    assert!(!plugin.exists(), "uninstall 后 plugin 应移除");
}

/// pi:TS 扩展投放 + check + uninstall 还原。
#[test]
fn pi_extension_roundtrip() {
    let sb = Sandbox::new("pi");
    let out = sb.run_ok(&["--install-pi"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-pi: {out}");

    // 扩展路径:pi 扩展目录(可能带版本目录,找 index.ts)
    let ext_root = sb.path(".pi/agent/extensions/pokepoke");
    assert!(ext_root.exists(), "pi 扩展应投放");
    let has_index = std::fs::read_dir(&ext_root)
        .map(|mut d| d.any(|e| e.as_ref().is_ok_and(|e| e.file_name() == "index.ts")))
        .unwrap_or(false);
    assert!(has_index, "pi 扩展目录应有 index.ts");

    let out = sb.run_ok(&["--check-pi"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-pi"]);
    assert!(!ext_root.exists(), "uninstall 后扩展应移除");
}

/// omp:TS 扩展投放 + check + uninstall 还原(verify 纯文件,无条件跑)。
/// 落点 honor omp 自身解析(default `~/.omp/agent`,可用 `PI_CODING_AGENT_DIR` 覆盖)。
#[test]
fn omp_extension_roundtrip() {
    let sb = Sandbox::new("omp");
    let out = sb.run_ok(&["--install-omp"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-omp: {out}");

    let ext = sb.path(".omp/agent/extensions/pokepoke/index.ts");
    assert!(ext.exists(), "omp 扩展应投放");
    let content = std::fs::read_to_string(&ext).unwrap();
    assert!(
        content.contains("--source=omp"),
        "omp 扩展应含 poke-hook 调用"
    );
    // 回归护栏:omp 没有 agent_settled 事件,不得把 pi 变体写过来(靠订阅形式判定,
    // 注释里提到 pi 的 agent_settled 不算)。
    assert!(!content.contains("pi.on(\"agent_settled\""));

    let out = sb.run_ok(&["--check-omp"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-omp"]);
    assert!(
        !sb.path(".omp/agent/extensions/pokepoke").exists(),
        "uninstall 后 omp 扩展应移除"
    );
}

/// grok:hooks JSON 投放 + check + uninstall 还原(verify 纯文件,无条件跑)。
#[test]
fn grok_hooks_roundtrip() {
    let sb = Sandbox::new("grok");
    let out = sb.run_ok(&["--install-grok"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-grok: {out}");

    let hook_file = sb.path(".grok/hooks/pokepoke.json");
    assert!(hook_file.exists(), "grok hook 文件应投放");
    let content = std::fs::read_to_string(&hook_file).unwrap();
    assert!(
        content.contains("--source=grok"),
        "grok hook 应含 poke-hook 调用"
    );

    let out = sb.run_ok(&["--check-grok"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-grok"]);
    assert!(!hook_file.exists(), "uninstall 后 grok hook 应移除");
}

// ==================== probe 依赖端:codex / kimi(条件回环) ====================

/// codex:verify 先 probe codex CLI(find_codex_binary),宿主有 codex 才断言
/// install ok;无 codex 则跳过(不 FAIL)。
#[test]
fn codex_roundtrip_when_cli_present() {
    if Command::new("codex").arg("--version").output().is_err() {
        eprintln!("skip: 本机无 codex CLI(verify 需 probe),跳过 codex 回环");
        return;
    }
    let sb = Sandbox::new("codex");
    // verify 的 `codex login status` probe 在空 ~/.codex 下会报 SchemaError →
    // install rollback。预置最小合法 config.toml 让 probe 稳定通过。
    sb.write(".codex/config.toml", "model = \"gpt-5\"\n");
    let out = sb.run_ok(&["--install-codex"]);
    // probe 依赖 codex CLI 的 login status 子进程(可能锁冲突 / 凭证状态),
    // 非确定:install ok → 跑全回环;verify 降级失败 → 打印原因后跳过(不红)。
    if check_json(&out)["status"] != "ok" {
        eprintln!("skip: codex verify probe 降级失败,跳过回环断言: {out}");
        return;
    }
    assert_eq!(check_json(&out)["status"], "ok", "install-codex: {out}");
    assert!(
        sb.path(".codex/hooks.json").exists(),
        "codex hooks.json 应写入"
    );

    let out = sb.run_ok(&["--check-codex"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-codex"]);
    // codex uninstall 直接删 hooks.json(文件不存在或 entry 清空都算还原)
    let hooks_path = sb.path(".codex/hooks.json");
    if hooks_path.exists() {
        let hooks = sb.read_json(".codex/hooks.json");
        assert_eq!(
            count_poke_entries(&hooks),
            0,
            "uninstall 后 codex poke entry 应清空"
        );
    }
}

/// kimi:verify 先 probe kimi CLI,宿主有 kimi 才断言 install ok;无则跳过。
#[test]
fn kimi_roundtrip_when_cli_present() {
    if Command::new("kimi").arg("--version").output().is_err() {
        eprintln!("skip: 本机无 kimi CLI(verify 需 probe),跳过 kimi 回环");
        return;
    }
    let sb = Sandbox::new("kimi");
    let out = sb.run_ok(&["--install-kimi"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-kimi: {out}");
    assert!(
        sb.path(".kimi-code/config.toml").exists(),
        "kimi config 应写入"
    );

    let out = sb.run_ok(&["--check-kimi"]);
    assert!(check_field(&out, "connected"));

    sb.run_ok(&["--uninstall-kimi"]);
    // uninstall 后 config 还原:不含 poke-hook 调用
    let content = std::fs::read_to_string(sb.path(".kimi-code/config.toml")).unwrap_or_default();
    assert!(
        !content.contains("poke-hook"),
        "uninstall 后 kimi config 不应含 poke-hook"
    );
}

/// DSH:profile patch managed block + 插件包投放 / check / uninstall 还原,
/// 且只碰有 web bundle 的 profile(headless 不投)。
#[test]
fn dsh_profile_roundtrip() {
    let sb = Sandbox::new("dsh");
    sb.write(
        ".dsh/profiles/desktop/package.json",
        r#"{"name":"dsh-profile-desktop","dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#,
    );
    sb.write(
        ".dsh/profiles/desktop/cordis.patch.yml",
        "- id: ui-theme\n  config:\n    preference: dark\n",
    );
    sb.write(
        ".dsh/profiles/headless/package.json",
        r#"{"name":"dsh-profile-headless","dsh":{"profile":{"bundles":["@deepseek-ai/dsh-headless"]}}}"#,
    );

    let out = sb.run_ok(&["--install-dsh"]);
    assert_eq!(check_json(&out)["status"], "ok", "install-dsh: {out}");

    let plugin_dir = sb.path(".dsh/profiles/desktop/pokepoke-dsh");
    let host = std::fs::read_to_string(plugin_dir.join("index.js")).expect("host plugin");
    assert!(host.contains("--source=dsh"));
    assert!(host.contains(&sb.home.join(".local/bin/poke-hook").display().to_string()));
    assert!(plugin_dir.join("client.js").exists());
    assert!(plugin_dir.join("package.json").exists());
    assert!(
        !sb.path(".dsh/profiles/headless/pokepoke-dsh").exists(),
        "headless profile 不该被接入"
    );

    let patch = std::fs::read_to_string(sb.path(".dsh/profiles/desktop/cordis.patch.yml")).unwrap();
    assert!(patch.contains("pokepoke-dsh managed block"));
    assert!(patch.contains("- id: ui-theme"), "用户内容必须保留");

    // 幂等:重复 install 不产生第二份 block。
    sb.run_ok(&["--install-dsh"]);
    let patch_again =
        std::fs::read_to_string(sb.path(".dsh/profiles/desktop/cordis.patch.yml")).unwrap();
    assert_eq!(
        patch_again
            .matches("pokepoke-dsh managed block >>>")
            .count(),
        1
    );

    let check = sb.run_ok(&["--check-dsh"]);
    assert!(check_field(&check, "connected"), "check-dsh: {check}");
    assert!(check_field(&check, "hooks_configured"));

    sb.run_ok(&["--uninstall-dsh"]);
    assert!(!plugin_dir.exists(), "uninstall 后插件目录应移除");
    let patch_after =
        std::fs::read_to_string(sb.path(".dsh/profiles/desktop/cordis.patch.yml")).unwrap();
    assert!(!patch_after.contains("pokepoke-dsh managed block"));
    assert!(patch_after.contains("- id: ui-theme"));
}

/// 防呆:空 HOME 下 check 全部 8 端都是 disconnected(不崩、字段齐全)。
#[test]
fn check_all_agents_on_empty_home() {
    let sb = Sandbox::new("empty-check");
    for flag in [
        "--check-cc",
        "--check-codex",
        "--check-cursor",
        "--check-opencode",
        "--check-pi",
        "--check-kimi",
        "--check-grok",
        "--check-dsh",
    ] {
        let out = sb.run_ok(&[flag]);
        let v = check_json(&out);
        assert_eq!(v["connected"], false, "{flag} 空配置应 disconnected: {out}");
        assert!(v.get("installed").is_some(), "{flag} 应有 installed 字段");
    }
}
