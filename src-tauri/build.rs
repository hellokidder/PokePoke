use std::fs;
use std::process::Command;

/// 本地开发时把仓库根的 `.env.local` 注入编译期环境,让 `option_env!`
/// 读到本地配置(免去每次手动 `source .env.local`)。
///
/// - 文件不存在直接跳过 —— CI 没有 `.env.local`(被 gitignore),照旧走
///   `release.yml` 的 `env:` 注入,不受影响。
/// - 2026-11 移除 license 后,项目已无编译期密钥;保留本机制是为了后续可选配置。
/// - 已存在于进程环境的变量不覆盖 —— CI 传进来的 `env:` 优先级更高,本地
///   文件只填空缺。
/// - `rerun-if-changed` 让改 env 后自动重编,避免增量编译命中旧值(test
///   兜底 product_id → 404 购买链接就是这么来的)。
fn load_dotenv() {
    // build.rs 的工作目录是 src-tauri/,`.env.local` 在其上一级仓库根。
    let path = "../.env.local";
    println!("cargo:rerun-if-changed={path}");

    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        // 进程环境已有则不覆盖(CI 的 env: 优先)。
        if std::env::var_os(key).is_none() {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}

fn main() {
    load_dotenv();
    inject_local_build_hash();
    tauri_build::build()
}

/// 本地构建时把当前 HEAD 的 short sha(+ dirty 标记)注入编译期常量
/// `POKE_BUILD_HASH`,给 About / Diagnostics 展示用。**只在本地跑**:
///
/// - `CI=true`(GitHub Actions 自动注入)时跳过 —— 发出去的正式版本必须
///   干净,不带 hash,否则 updater 的语义版本比对 / 用户看到"1.0.1 (xxx)"
///   会疑惑。
/// - 拿不到 git(tarball 编译 / git 没装)时静默跳过,commands::app_version
///   fallback 到纯 CARGO_PKG_VERSION。
/// - `rerun-if-changed=.git/HEAD` + `.git/index` 保证换 commit / stage 改动
///   时 build.rs 重跑,hash 不会残旧。
fn inject_local_build_hash() {
    // CI 变化(取消/设置)必须触发 build.rs 重跑,否则 target/ 缓存里旧
    // hash 会跟着到 CI(反之亦然)。
    println!("cargo:rerun-if-env-changed=CI");

    // CI 上一律不注入。GitHub Actions 默认设 CI=true;本地 shell 不该有。
    if std::env::var_os("CI").is_some() {
        return;
    }

    // 让 build.rs 在 HEAD / index 变化时重跑。文件不存在(非 git 仓库)
    // cargo 会警告但不失败,这里不额外守护。
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    let Some(short) = run_git(&["rev-parse", "--short=8", "HEAD"]) else {
        return;
    };
    // `git status --porcelain --untracked-files=no` 空行 = 干净。用
    // `--untracked-files=no` 排除新增的未跟踪文件(比如临时草稿 / 新
    // plan 文档),这些不构成"当前 build 跟 HEAD 不同"。
    let dirty = run_git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let value = if dirty {
        format!("{short}-dirty")
    } else {
        short
    };
    println!("cargo:rustc-env=POKE_BUILD_HASH={value}");
}

fn run_git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir("..") // build.rs cwd = src-tauri/,git 命令得在仓库根跑
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}
