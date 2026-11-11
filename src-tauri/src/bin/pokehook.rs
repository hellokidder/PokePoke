//! poke-hook — Universal hook handler for Poke Poke.
//!
//! Supports Claude Code, Codex CLI, Cursor, opencode, pi, omp, and Kimi Code as managed integrations.
//!
//! Two modes:
//!   1. Hook mode (no args): reads JSON from stdin, auto-detects source, handles events
//!   2. CLI mode: manages integration for each tool
//!      CC:       --install / --uninstall / --check
//!      Codex:    --install-codex / --uninstall-codex / --check-codex
//!      Cursor:   --install-cursor / --uninstall-cursor / --check-cursor
//!     (全局接入,写入 `~/.cursor/hooks.json`,不再按项目)
//!      opencode: --install-opencode / --uninstall-opencode / --check-opencode
//!     (投放 JS plugin 到 `~/.config/opencode/plugins/pokepoke.js`)
//!      pi:       --install-pi / --uninstall-pi / --check-pi
//!     (投放 TS 扩展到 `~/.pi/agent/extensions/pokepoke/index.ts`)
//!      omp:      --install-omp / --uninstall-omp / --check-omp
//!     (投放 TS 扩展到 omp 的 agent dir,默认 `~/.omp/agent/extensions/pokepoke/index.ts`;
//!      honor `PI_CODING_AGENT_DIR` / `OMP_PROFILE` / `PI_PROFILE`,见 omp.rs)
//!      kimi:     --install-kimi / --uninstall-kimi / --check-kimi
//!     (向 `~/.kimi-code/config.toml` 的 `[[hooks]]` array-of-tables 注入;
//!      kimi 的 PermissionRequest 事件映射 Pending)
//!      grok:     --install-grok / --uninstall-grok / --check-grok
//!     (投放 JSON 到 `~/.grok/hooks/pokepoke.json`;Grok 会读 CC/Cursor 兼容 hook,
//!      hook mode 用 `GROK_HOOK_EVENT` 环境变量纠偏 source)
//!      TOML 编辑用 toml_edit 保留用户既有 providers/models 等配置)

use serde_json::Value;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

// ==================== Submodules(按 agent / 职责拆分,AGENTS.md §4a)====================
//
// install/uninstall/check 按 agent 拆:cc.rs / codex.rs / cursor.rs / opencode.rs /
// pi.rs / omp.rs / kimi.rs(grok 已独立为 grok.rs)。共享设施:constants.rs(事件表 + CLI 参数)、
// source.rs(Source 判定 + task_id 推导)、paths.rs(外部配置路径)、env_collect.rs
// (host-agent 环境采集)、deploy.rs(二进制部署)、transaction.rs(L1 回滚)、drift.rs
// (hook 配置过期判定)、events.rs(hook 事件处理)。各 agent 子文件的函数大多仅在本
// 模块内使用;下方 pub(crate) use 是为 main 的 dispatch、hook_mode 以及既有子模块
// (tests.rs / grok.rs / build_payload.rs / snapshot.rs / subagents.rs 经 `super::*`
// 或具名 `super::x` 引用)保留的可见面,行为零变化。
//
// hook binary 因 dual-compile 限制不能 `use poke_poke_lib::hook_deploy`,
// 与 transport 同理 inline 同一份 hook_deploy.rs。同文件被 lib + hook bin 各编译
// 一次,clippy::duplicate_mod 会报 —— 这是 dual-compile 的已知权衡,allow。
#[path = "../hook_deploy.rs"]
#[allow(clippy::duplicate_mod)]
mod hook_deploy;

#[path = "pokehook/constants.rs"]
mod constants;
pub(crate) use constants::{HOOK_CLI_ARGS, LOCK_DIR};
// 事件表常量仅供 tests.rs 经 `super::*` 引用(各 agent 子文件直接 `super::constants::`)。
#[cfg(test)]
pub(crate) use constants::{CODEX_HOOK_EVENTS, CURSOR_HOOK_EVENTS, KIMI_HOOK_EVENTS};

#[path = "pokehook/source.rs"]
mod source;
pub(crate) use source::{
    build_task_id, detect_source, external_session_id, has_running_background_tasks,
    parse_source_arg, pick_notification_type, pick_stop_failure_detail, pick_stop_failure_reason,
    pick_str, project_name, resolve_workspace_path, Source,
};

#[path = "pokehook/paths.rs"]
mod paths;
pub(crate) use paths::{home_dir, hook_bin_path};

#[path = "pokehook/env_collect.rs"]
mod env_collect;
// build_payload.rs 经 `use super::*` 引用 host-agent 环境采集(otty 仅 events.rs 用,
// 直接经 `super::env_collect::` 引用,不走 crate root)。
pub(crate) use env_collect::{collect_cmux_env, collect_herdr_env, collect_wezterm_env};

#[path = "pokehook/deploy.rs"]
mod deploy;
pub(crate) use deploy::{cmd_deploy_self, ensure_binary_installed, print_json};

#[path = "pokehook/transaction.rs"]
mod transaction;
pub(crate) use transaction::WriteTransaction;

#[path = "pokehook/drift.rs"]
mod drift;
pub(crate) use drift::expected_hook_command;
// 各 agent 的 outdated 判定仅 tests.rs 经 `super::*` 引用(cmd_check_* 在各自
// 子文件内直接调用,不走 crate root)。
#[cfg(test)]
pub(crate) use drift::{
    cc_hooks_outdated, codex_feature_enabled, codex_hooks_outdated, cursor_hooks_outdated,
    hook_entry_outdated,
};

#[path = "pokehook/cc.rs"]
mod cc;
pub(crate) use cc::{cmd_check_cc, cmd_install_cc, cmd_uninstall_cc};
#[cfg(test)]
pub(crate) use cc::{contains_poke_hook, drop_poke_hook_entries_from_groups};

#[path = "pokehook/codex.rs"]
mod codex;
#[cfg(test)]
pub(crate) use codex::merge_poke_hooks_into_codex;
pub(crate) use codex::{cmd_check_codex, cmd_install_codex, cmd_uninstall_codex};

#[path = "pokehook/cursor.rs"]
mod cursor;
pub(crate) use cursor::{cmd_check_cursor, cmd_install_cursor, cmd_uninstall_cursor};
#[cfg(test)]
pub(crate) use cursor::{
    cursor_entry_has_poke_hook, cursor_hooks_configured, install_cursor_to_path,
    uninstall_cursor_from_path,
};

#[path = "pokehook/opencode.rs"]
mod opencode;
pub(crate) use opencode::{cmd_check_opencode, cmd_install_opencode, cmd_uninstall_opencode};
#[cfg(test)]
pub(crate) use opencode::{
    opencode_plugin_configured, opencode_plugin_contents, opencode_plugin_outdated,
};

#[path = "pokehook/pi.rs"]
mod pi;
pub(crate) use pi::{cmd_check_pi, cmd_install_pi, cmd_uninstall_pi};
#[cfg(test)]
pub(crate) use pi::{pi_extension_configured, pi_extension_contents, pi_extension_outdated};

#[path = "pokehook/omp.rs"]
mod omp;
pub(crate) use omp::{cmd_check_omp, cmd_install_omp, cmd_uninstall_omp};

#[path = "pokehook/kimi.rs"]
mod kimi;
pub(crate) use kimi::{cmd_check_kimi, cmd_install_kimi, cmd_uninstall_kimi};
#[cfg(test)]
pub(crate) use kimi::{
    kimi_command_is_ours, kimi_hooks_configured, kimi_hooks_outdated, merge_poke_hooks_into_kimi,
    remove_poke_hooks_from_kimi,
};

#[path = "pokehook/events.rs"]
mod events;
#[cfg(test)]
pub(crate) use events::session_end_payload;
pub(crate) use events::{normalize_event, terminal_app};

// L1 写后回环验证模块(`docs/briefs/20260507-write-then-verify-l1.md`)。
// install/uninstall 写完外部配置后调本模块,确认宿主能加载;失败由调用
// 方触发回滚 + 返回 error JSON。
#[path = "pokehook/verify.rs"]
mod verify;

// Pre-write snapshot + `~/.pokepoke/` 目录工具(2026-08-12 收尾抽 mod,见
// AGENTS.md §4a;调用方遍及三端 install/uninstall 与 grok.rs)。
// 其余函数(pub(crate) 级)供 tests.rs 经 `super::snapshot::` 直接引用。
#[path = "pokehook/snapshot.rs"]
mod snapshot;
pub(crate) use snapshot::record_pre_write_snapshot;

// Otty 环境采集(从本文件抽出,见 AGENTS.md §4a:新终端适配用 mod 目录)。
#[path = "pokehook/build_payload.rs"]
mod build_payload;
#[path = "pokehook/grok.rs"]
mod grok;
// DeepSeek Harness(2026-11-04):cordis 插件包 + profile patch managed block。
#[path = "pokehook/dsh.rs"]
mod dsh;
pub(crate) use dsh::{cmd_check_dsh, cmd_install_dsh, cmd_uninstall_dsh};
#[path = "pokehook/orca_env.rs"]
mod orca_env;
#[path = "pokehook/tmux_env.rs"]
mod tmux_env;
// Kimi subagent 跨进程计数(flock 串行化 + stale 自愈,见 AGENTS.md §4a)。
#[path = "pokehook/subagents.rs"]
mod subagents;

// 测试模块拆到独立文件,避免本文件 2000+ 行单体过大(scope-audit §4.1)。
// 视为 hook 模块的 #[cfg(test)] 子模块,所有 `super::*` 引用照常工作。
#[cfg(test)]
#[path = "pokehook/tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use build_payload::build_payload;
pub(crate) use build_payload::{accepts_permission_request_source, accepts_stop_failure_source};

// ==================== Entry point ====================

fn is_hook_invocation(exe_name: Option<&str>, first_arg: Option<&str>) -> bool {
    exe_name.is_some_and(|name| name.starts_with("poke-hook"))
        || first_arg.is_some_and(|arg| HOOK_CLI_ARGS.contains(&arg))
}

pub fn should_run_from_current_process() -> bool {
    let exe_name = env::current_exe().ok().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });
    let first_arg = env::args().nth(1);

    is_hook_invocation(exe_name.as_deref(), first_arg.as_deref())
}

pub fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        // Claude Code (backward-compatible)
        Some("--install") => cmd_install_cc(),
        Some("--uninstall") => cmd_uninstall_cc(),
        // --check 是历史参数名；app integration 层实际调用 --check-cc（2026-08-02 验证时发现
        // --check-cc 落入 hook_mode 默认分支输出 {}，cc drift 检测从未生效）
        Some("--check") | Some("--check-cc") => cmd_check_cc(),
        // Codex CLI
        Some("--install-codex") => cmd_install_codex(),
        Some("--uninstall-codex") => cmd_uninstall_codex(),
        Some("--check-codex") => cmd_check_codex(),
        // Cursor(全局接入:`~/.cursor/hooks.json`,不再按项目)
        Some("--install-cursor") => cmd_install_cursor(),
        Some("--uninstall-cursor") => cmd_uninstall_cursor(),
        Some("--check-cursor") => cmd_check_cursor(),
        // opencode(投放 JS plugin 到 `~/.config/opencode/plugin/pokepoke.js`)
        Some("--install-opencode") => cmd_install_opencode(),
        Some("--uninstall-opencode") => cmd_uninstall_opencode(),
        Some("--check-opencode") => cmd_check_opencode(),
        // pi(投放 TS 扩展到 `~/.pi/agent/extensions/pokepoke/index.ts`)
        Some("--install-pi") => cmd_install_pi(),
        Some("--uninstall-pi") => cmd_uninstall_pi(),
        Some("--check-pi") => cmd_check_pi(),
        // omp(投放 TS 扩展到 omp 的 agent dir,默认 `~/.omp/agent/extensions/pokepoke/index.ts`)
        Some("--install-omp") => cmd_install_omp(),
        Some("--uninstall-omp") => cmd_uninstall_omp(),
        Some("--check-omp") => cmd_check_omp(),
        // kimi(向 `~/.kimi-code/config.toml` 的 `[[hooks]]` 注入 hook 条目)
        Some("--install-kimi") => cmd_install_kimi(),
        Some("--uninstall-kimi") => cmd_uninstall_kimi(),
        Some("--check-kimi") => cmd_check_kimi(),
        // grok(投放 JSON 到 `~/.grok/hooks/pokepoke.json`)
        Some("--install-grok") => grok::cmd_install_grok(),
        Some("--uninstall-grok") => grok::cmd_uninstall_grok(),
        Some("--check-grok") => grok::cmd_check_grok(),
        // DeepSeek Harness(cordis 插件包 + profile patch managed block)
        Some("--install-dsh") => cmd_install_dsh(),
        Some("--uninstall-dsh") => cmd_uninstall_dsh(),
        Some("--check-dsh") => cmd_check_dsh(),
        Some("--deploy-self") => cmd_deploy_self(),
        // 版本自报,用于 bootstrap 首启比对 bundled vs deployed 是否一致(Bug 5)。
        // 输出格式: 单行 "poke-hook <version>\n"。
        Some("--version") | Some("-V") => {
            println!("poke-hook {}", env!("CARGO_PKG_VERSION"));
        }
        // Hook mode
        _ => hook_mode(),
    }
}

// ==================== Hook mode ====================

fn hook_mode() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .unwrap_or_default();

    // Always output valid JSON for the caller
    let _guard = PrintOnDrop;

    let data: Value = match serde_json::from_str(input.trim()) {
        Ok(v) => v,
        Err(_) => return,
    };

    let argv: Vec<String> = env::args().collect();
    let arg_source = parse_source_arg(&argv);
    let source = detect_source(arg_source, &data);

    // Extract event name (CC/Codex: hook_event_name, Cursor: hookEventName or hook_event_name)
    let raw_event = pick_str(&data, &["hook_event_name", "hookEventName"]).unwrap_or_default();

    // Normalize Cursor camelCase events to internal names
    let event = normalize_event(&raw_event);

    // Build stable task_id
    let task_id = build_task_id(&data, source);

    // Extract workspace path
    let cwd = resolve_workspace_path(&data, source);
    let project = project_name(&cwd);

    // cc/codex 一律带 transcript_path,
    // cursor 用 camelCase 兜底。空 / 缺失则 None,server 端不覆盖现有值。
    let transcript_path = pick_str(&data, &["transcript_path", "transcriptPath"]);
    let tp = transcript_path.as_deref();

    // 事件去重已让出到 server 端 100ms 窗口(scope-audit §3.2 / §8.2),
    // hook binary 保持无状态。Cursor 开 Third-party hooks 双触发的去重
    // 由 `http_server::DedupeWindow` 在单进程内消化。

    match event.as_str() {
        "SessionStart" => events::handle_session_start(
            &task_id,
            external_session_id(&data, source).as_deref(),
            &project,
            &cwd,
            source,
            tp,
        ),
        "UserPromptSubmit" => events::handle_user_prompt_submit(
            &task_id,
            external_session_id(&data, source).as_deref(),
            &project,
            &cwd,
            source,
            tp,
        ),
        "Notification" => {
            // CC + opencode 走此路径进 Pending。
            // - CC:Notification 复合事件(permission / idle_prompt),按 notification_type 过滤。
            // - opencode:plugin 把 `permission.updated` 归一成 Notification + message=权限标题。
            //   opencode payload 不带 notification_type,自然跳过 idle_prompt 过滤直达 Pending。
            // - grok:仅 permission_prompt / elicitation_dialog 升 Pending。
            // Codex/Cursor 没有此内部事件(Codex 用 PermissionRequest,Cursor 无权限事件)。
            if matches!(source, Source::ClaudeCode | Source::OpenCode | Source::Dsh) {
                if pick_notification_type(&data).as_deref() == Some("idle_prompt") {
                    return;
                }
                let message = data["message"].as_str().unwrap_or("");
                events::handle_notification(
                    &task_id,
                    external_session_id(&data, source).as_deref(),
                    &project,
                    message,
                    &cwd,
                    source,
                    tp,
                );
            } else if source == Source::Grok {
                if let Some(message) = grok::grok_notification_message(&data) {
                    events::handle_notification(
                        &task_id,
                        external_session_id(&data, source).as_deref(),
                        &project,
                        &message,
                        &cwd,
                        source,
                        tp,
                    );
                }
            }
        }
        "PermissionRequest" => {
            // Codex + Kimi + CC — 跑命令需要审批时触发("agent 卡住等用户决策"),
            // 复用 handle_notification 进入 Pending。
            // CC(2026-08-19 起订阅):弹权限确认时立即触发,是"等用户确认"的主信号;
            // Notification.permission_prompt 只在 ~6s 无人理后才 fire,不能只靠它。
            // Cursor 没有对应事件(beforeShellExecution 拦截 hook,我们不订)。
            if accepts_permission_request_source(source) {
                // tool_name 是 Codex PermissionRequest payload 必含字段(参 codex hooks docs:
                // PermissionRequest 额外 stdin = tool_name + tool_input)。Kimi 字段名未真机
                // 确认,额外兜底 `tool` key。预留驼峰 fallback 防 schema 漂移;缺失时退化为
                // 通用"awaiting approval",流程不断。
                let tool_name = pick_str(&data, &["tool_name", "toolName", "tool"]);
                let message = match tool_name.as_deref() {
                    Some(name) if !name.is_empty() => format!("Awaiting approval: {}", name),
                    _ => "Awaiting approval".to_string(),
                };
                events::handle_notification(
                    &task_id,
                    external_session_id(&data, source).as_deref(),
                    &project,
                    &message,
                    &cwd,
                    source,
                    tp,
                );
            }
        }
        "Stop" => {
            if source == Source::Cursor {
                // Cursor subagent(SubagentStart/SubagentStop 计数 >0)仍跑时,
                // main stop 只是 turn 边界、不是真完成 → suppress(对齐 Kimi
                // 语义,grok review §4-②)。stale 计数自愈放行,与 CC/Kimi 同。
                let stale = subagents::purge_stale_count(&task_id);
                let suppress = !stale && subagents::active_subagent_count(&task_id) > 0;
                // hookStatus 缺失不再默认 completed(曾把 error 报成"完成",
                // grok review §2.4);空值落入 handle_cursor_stop 的 last_failed 分支。
                let hook_status = pick_str(&data, &["status", "hookStatus"]).unwrap_or_default();
                events::handle_cursor_stop(
                    &task_id,
                    external_session_id(&data, source).as_deref(),
                    &project,
                    &cwd,
                    &hook_status,
                    suppress,
                );
            } else {
                let stale = subagents::purge_stale_count(&task_id);
                let suppress = !stale
                    && (has_running_background_tasks(&data)
                        || subagents::active_subagent_count(&task_id) > 0
                        || (source == Source::Grok && !grok::grok_stop_is_turn_end(&data)));
                // pi 扩展(brief 20260820-pi-aborted-no-completion-sound)在用户取消时
                // 随 Stop 上报 aborted:true;其它 source 目前不带(缺省 false)。
                let aborted = data
                    .get("aborted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                events::handle_stop(
                    &task_id,
                    external_session_id(&data, source).as_deref(),
                    &project,
                    &cwd,
                    source,
                    tp,
                    suppress,
                    aborted,
                );
            }
        }
        "StopFailure" => {
            // CC + opencode + Kimi：一轮对话因 API/provider 错误终止。
            // - CC:StopFailure hook,reason 走 matcher 值。
            // - opencode:plugin 把 `session.error` 归一成 StopFailure + reason=error 类型。
            // - Kimi:StopFailure hook,payload 字段未真机确认,pick_* 多 key 兜底。
            // Codex/Cursor 无此内部事件;收到也跳过,避免误触发。
            if accepts_stop_failure_source(source) {
                // 兜底 key 名（CC 目前用 reason，预留别名以防官方变更）
                let reason = pick_stop_failure_reason(&data);
                let detail = pick_stop_failure_detail(&data);
                events::handle_stop_failure(
                    &task_id,
                    external_session_id(&data, source).as_deref(),
                    &project,
                    &cwd,
                    source,
                    &reason,
                    &detail,
                    tp,
                );
            }
        }
        "SubagentStart" if matches!(source, Source::Kimi | Source::Cursor) => {
            subagents::adjust_subagent_count(&task_id, 1);
        }
        "SubagentStop" if matches!(source, Source::Kimi | Source::Cursor) => {
            subagents::adjust_subagent_count(&task_id, -1);
        }
        "SessionEnd" => {
            events::handle_session_end(
                &task_id,
                external_session_id(&data, source).as_deref(),
                &project,
                &cwd,
                source,
            );
        }
        _ => {}
    }
}

// ==================== Low-level helpers ====================

pub(crate) fn flag_path(task_id: &str, suffix: &str) -> PathBuf {
    PathBuf::from(LOCK_DIR).join(format!("pokepoke-{}.{}", task_id, suffix))
}

/// 上报一条 payload 到 server,通过固定路径 UDS(`~/.pokepoke/sock`)。
/// 连接失败 → 返回 false(PrintOnDrop 仍输出合法 `{}` 满足 hook host 契约)。
///
/// 由 `docs/briefs/20260423-uds-fixed-path.md` 把 §3.1 的 TCP localhost 修正
/// 回极简 UDS;§3.1 多实例让出决议保留(只有一个 sock,无 endpoints,无路由)。
pub(crate) fn post_notify(payload: &Value) -> bool {
    let body = payload.to_string();
    post_unix(&sock_path(), &body)
}

/// 手写 HTTP/1.1 over UnixStream。沿用 §3.1 之前 post_uds 的策略:
/// - 不引入 hyper / ureq 这类大 client(hook 是高频 cold-start 的小 binary)
/// - 端点只有一个 POST /notify,server 响应 body 不被消费,2xx 即成功
/// - 严格只检查 status-line 前 12 字节 `HTTP/1.1 2` 开头
/// - read/write 各 2s 超时,避免 server 卡死时 hook 僵死
///
/// ⚠️ **不要"优化"成 fire-and-forget 或短读窗口**(2026-10-13 实测,详见 brief
/// `20261013-hook-no-wait-notify.md`):app 侧是 axum/hyper,客户端提前关闭连接会让
/// in-flight 的 `/notify` future 被取消 —— 通知会**静默丢失**。
/// 实测对照(同一 `/notify` 请求,running 事件,各 6 次):
///
/// | 客户端行为 | app 落库 |
/// | --- | --- |
/// | write + 立即 close | 0/6 |
/// | write + `shutdown(WR)` + close | 0/6 |
/// | write + 保持连接 200ms(不读) | 6/6 |
/// | write + 读响应(本实现) | 6/6 |
///
/// 所以这里的 2s 不是防卡死而已,它同时是"让 server 把已受理的请求做完"的保底。
/// 要缩短宿主延迟,正确方向是**让 app 先回响应、再把 focus / capture 等慢探测丢给
/// 后台任务**(brief `20261013-focus-off-response-path.md`),而不是让 hook 提前退出。
pub(crate) fn post_unix(sock: &Path, body: &str) -> bool {
    let mut stream = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let req = format!(
        "POST /notify HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body,
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }

    let mut buf = [0u8; 16];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    n >= 12 && buf.starts_with(b"HTTP/1.1 2")
}

// hook binary 不能 `use poke_poke_lib::transport` —— hook.rs 是 dual-compile,
// 这里 inline sock 路径(必须与 transport.rs::sock_path 同语义)。
fn sock_path() -> PathBuf {
    home_dir().join(".pokepoke").join("sock")
}

pub(crate) fn get_tty() -> Option<String> {
    // Opt-in 诊断：`touch ~/.pokepoke/hook-debug` 启用，rm 关闭。
    // 启用后每次 get_tty 把祖先链落到 ~/.pokepoke/hook.log，便于
    // 复现"tty=None 被探活线程 reap"的根因（setsid / GUI 启动 /
    // 中间 helper 进程导致 ancestor chain 看不到 tty）。
    let debug = hook_debug_enabled();
    let mut walk: Vec<String> = Vec::new();

    let mut pid = std::process::id();
    let result = (|| -> Option<String> {
        for depth in 0..8 {
            let output = Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "tty=,ppid=,comm="])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                if debug {
                    walk.push(format!("[{}] pid={} <ps empty>", depth, pid));
                }
                break;
            }
            // tty=,ppid= 都是不带头的，第三列 comm 可能含空格（路径中罕见但语义保留），
            // 先按前两列做 split，剩余作为 comm。
            let mut iter = trimmed.split_whitespace();
            let tty_name = iter.next().unwrap_or("");
            let ppid: u32 = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let comm: String = iter.collect::<Vec<_>>().join(" ");

            if debug {
                walk.push(format!(
                    "[{}] pid={} tty={} ppid={} comm={}",
                    depth, pid, tty_name, ppid, comm,
                ));
            }

            if !tty_name.is_empty() && tty_name != "??" {
                return Some(format!("/dev/{}", tty_name));
            }
            if ppid <= 1 {
                break;
            }
            pid = ppid;
        }
        None
    })();

    if debug {
        let outcome = result
            .as_deref()
            .map(|t| format!("FOUND {}", t))
            .unwrap_or_else(|| "NONE".into());
        write_hook_debug(&format!("get_tty -> {}\n  {}", outcome, walk.join("\n  ")));
    }

    result
}

fn hook_debug_dir() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".pokepoke"))
}

fn hook_debug_enabled() -> bool {
    hook_debug_dir()
        .map(|d| d.join("hook-debug").exists())
        .unwrap_or(false)
}

fn write_hook_debug(message: &str) {
    let Some(dir) = hook_debug_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("hook.log");
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    let line = format!("{} [DEBUG] [hook] {}\n", ts, message);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// Prints `{}` to stdout when dropped (even on early return).
struct PrintOnDrop;
impl Drop for PrintOnDrop {
    fn drop(&mut self) {
        println!("{{}}");
    }
}
