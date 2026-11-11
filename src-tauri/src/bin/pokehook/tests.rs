use super::snapshot::plan_snapshot_path;

// UDS 传输契约测试(post_unix 跨进程契约)单列一个文件:见 uds_tests.rs 头注释。
#[path = "uds_tests.rs"]
mod uds_tests;
use super::{
    accepts_permission_request_source, accepts_stop_failure_source, build_payload, build_task_id,
    contains_poke_hook, cursor_entry_has_poke_hook, cursor_hooks_configured,
    drop_poke_hook_entries_from_groups, expected_hook_command, external_session_id, flag_path,
    install_cursor_to_path, is_hook_invocation, kimi_command_is_ours, kimi_hooks_configured,
    kimi_hooks_outdated, merge_poke_hooks_into_codex, merge_poke_hooks_into_kimi, normalize_event,
    opencode_plugin_configured, opencode_plugin_contents, opencode_plugin_outdated,
    parse_source_arg, pi_extension_configured, pi_extension_contents, pi_extension_outdated,
    pick_str, project_name, remove_poke_hooks_from_kimi, resolve_workspace_path,
    session_end_payload, terminal_app, uninstall_cursor_from_path, Source, CODEX_HOOK_EVENTS,
    CURSOR_HOOK_EVENTS, KIMI_HOOK_EVENTS,
};
use serde_json::json;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// 单测用沙盒目录：每个 case 独立路径，避免并发互相污染。
fn sandbox_hooks_path(tag: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("pokepoke-cursor-test-{}-{}-{}", tag, pid, n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("hooks.json")
}

#[test]
fn parse_source_arg_extracts_known_value_at_any_position() {
    let argv = vec!["poke-hook".to_string(), "--source=cc".to_string()];
    assert_eq!(parse_source_arg(&argv), Some(Source::ClaudeCode));

    let argv = vec!["poke-hook".to_string(), "--source=codex".to_string()];
    assert_eq!(parse_source_arg(&argv), Some(Source::Codex));

    let argv = vec!["poke-hook".to_string(), "--source=cursor".to_string()];
    assert_eq!(parse_source_arg(&argv), Some(Source::Cursor));

    // 位置任意：argv 末尾也行
    let argv = vec![
        "poke-hook".to_string(),
        "--other".to_string(),
        "--source=cursor".to_string(),
    ];
    assert_eq!(parse_source_arg(&argv), Some(Source::Cursor));
}

#[test]
fn parse_source_arg_returns_none_for_missing_or_unknown_value() {
    let argv: Vec<String> = vec!["poke-hook".to_string()];
    assert_eq!(parse_source_arg(&argv), None);

    let argv = vec!["poke-hook".to_string(), "--source=foobar".to_string()];
    assert_eq!(parse_source_arg(&argv), None);

    // `--source` 必须是 `=value` 形式；没有等号不解析（不引入空格分隔的 arg 解析）
    let argv = vec![
        "poke-hook".to_string(),
        "--source".to_string(),
        "cc".to_string(),
    ];
    assert_eq!(parse_source_arg(&argv), None);
}

#[test]
fn hook_invocation_detects_poke_hook_binary_name_or_known_cli_args() {
    assert!(is_hook_invocation(Some("poke-hook"), None));
    assert!(is_hook_invocation(
        Some("poke-hook-aarch64-apple-darwin"),
        None
    ));
    assert!(is_hook_invocation(Some("PokePoke"), Some("--install")));
    assert!(is_hook_invocation(
        Some("PokePoke"),
        Some("--install-codex")
    ));
    assert!(!is_hook_invocation(Some("PokePoke"), Some("-psn_0_12345")));
    assert!(!is_hook_invocation(Some("PokePoke"), None));
}

#[test]
fn normalize_event_maps_cursor_event_names() {
    assert_eq!(normalize_event("sessionStart"), "SessionStart");
    assert_eq!(normalize_event("beforeSubmitPrompt"), "UserPromptSubmit");
    assert_eq!(normalize_event("stop"), "Stop");
    assert_eq!(normalize_event("Notification"), "Notification");
}

#[test]
fn pick_str_returns_first_non_empty_candidate() {
    let data = json!({
        "first": "",
        "second": "value",
        "third": "later",
    });

    assert_eq!(
        pick_str(&data, &["first", "second", "third"]).as_deref(),
        Some("value"),
    );
}

#[test]
fn pick_str_returns_none_when_all_candidates_missing_or_empty() {
    let data = json!({
        "first": "",
        "second": null,
    });

    assert_eq!(pick_str(&data, &["first", "second", "third"]), None);
}

#[test]
fn contains_poke_hook_detects_command_presence() {
    let yes = json!({
        "hooks": [
            {"command": "/Users/me/.local/bin/poke-hook"}
        ]
    });
    let no = json!({
        "hooks": [
            {"command": "/usr/bin/other"}
        ]
    });

    assert!(contains_poke_hook(&yes));
    assert!(!contains_poke_hook(&no));
}

#[test]
fn contains_poke_hook_still_matches_when_command_has_source_arg() {
    // Reinstall / uninstall 路径靠"含 poke-hook 子串"找老条目，新设计加了
    // `--source=cc` 后缀；这条测试守门 contains_poke_hook 不被收紧成"完全等于"
    // 之类的判断而漏掉带 arg 的命令字符串。
    let cc = json!({
        "hooks": [
            {"command": "/Users/me/.local/bin/poke-hook --source=cc"}
        ]
    });
    let codex = json!({
        "hooks": [
            {"command": "/Users/me/.local/bin/poke-hook --source=codex"}
        ]
    });
    assert!(contains_poke_hook(&cc));
    assert!(contains_poke_hook(&codex));
}

#[test]
fn cursor_entry_has_poke_hook_still_matches_when_command_has_source_arg() {
    let entry = json!({"command": "/Users/me/.local/bin/poke-hook --source=cursor"});
    assert!(cursor_entry_has_poke_hook(&entry));

    let other = json!({"command": "/opt/user/other-hook"});
    assert!(!cursor_entry_has_poke_hook(&other));
}

#[test]
fn flag_path_uses_tmp_lock_dir_convention() {
    assert_eq!(
        flag_path("cc-1234", "pending"),
        PathBuf::from("/tmp/pokepoke-cc-1234.pending"),
    );
}

#[test]
fn build_helpers_derive_task_id_and_workspace_path() {
    let cursor = json!({
        "conversation_id": "cursor-session-abcdefgh",
        "workspace_roots": ["/Users/tester/proj-a"]
    });
    let codex = json!({
        "session_id": "abcdef123456",
        "cwd": "/Users/tester/proj-b"
    });

    assert_eq!(build_task_id(&cursor, Source::Cursor), "cursor-cursor-s");
    assert_eq!(
        external_session_id(&cursor, Source::Cursor).as_deref(),
        Some("cursor-session-abcdefgh"),
    );
    assert_eq!(
        resolve_workspace_path(&cursor, Source::Cursor),
        "/Users/tester/proj-a"
    );
    assert_eq!(build_task_id(&codex, Source::Codex), "codex-abcdef12");
    assert_eq!(
        external_session_id(&codex, Source::Codex).as_deref(),
        Some("abcdef123456"),
    );
    assert_eq!(project_name("/Users/tester/proj-b"), "proj-b");
}

// ---- terminal_app 字段(brief 20260527-ghostty-capture-retry 扩展)----
// b20520 当时只让 SessionStart 携带 TERM_PROGRAM,因为 capture 只在
// SessionStart 触发一次。20260527 brief 推翻该 Non-goal:实测 SessionStart
// capture 仅 72% 命中(多 tab 并行 / cc daemon resume / cwd-mismatch reject)。
// server 端需要在 UserPromptSubmit / Notification / Stop 上补一次幂等 capture,
// 因此**所有 cc/codex 事件都必须带 terminal_app**(让 server 知道这是 Ghostty)。
// Cursor 事件不带(Cursor 一般不在 Ghostty)。

// 临时 set/unset env var 跑闭包,跑完恢复。
/// env 是进程全局,cargo test 并行跑会互相污染 → 用 Mutex 把所有 env 改动串行起来。
/// Mutex 只锁 env 改动期间;不改 env 的其他 test 不受影响。
pub(super) fn with_env<F: FnOnce()>(key: &str, val: Option<&str>, f: F) {
    with_envs(&[(key, val)], f);
}

/// `with_env` 的多 key 版本。需要同时控制多个 env(如 Ghostty 兜底测试要
/// 同时锁定 TERM_PROGRAM + TERM + GHOSTTY_*)。单次锁 Mutex 设置所有 key,
/// 跑完按原值恢复。**不能嵌套调用**(std::sync::Mutex 非 reentrant)。
///
/// 未在 `pairs` 里点名的 host-agent env(`CMUX_*` / `HERDR_*`)会主动清掉,
/// 避免在 cmux/herdr pane 里跑 `cargo test` 时污染 terminal_app 断言。
pub(super) fn with_envs<F: FnOnce()>(pairs: &[(&str, Option<&str>)], f: F) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static ENV_MUTEX: Mutex<()> = Mutex::new(());
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    const HOST_AGENT_KEYS: &[&str] = &[
        "CMUX_SURFACE_ID",
        "CMUX_WORKSPACE_ID",
        "HERDR_PANE_ID",
        "HERDR_WORKSPACE_ID",
        "HERDR_SOCKET_PATH",
        "WEZTERM_PANE",
        "WEZTERM_UNIX_SOCKET",
        "WEZTERM_EXECUTABLE",
        "TMUX",
        "TMUX_PANE",
        // brief 20260810-warp-support:Warp 专属信号,同样会污染 terminal_app 断言
        "WARP_IS_LOCAL_SHELL_SESSION",
    ];
    let pair_keys: HashSet<&str> = pairs.iter().map(|(k, _)| *k).collect();
    let mut all_keys: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
    for k in HOST_AGENT_KEYS {
        if !pair_keys.contains(k) {
            all_keys.push(k);
        }
    }
    let originals: Vec<(String, Option<String>)> = all_keys
        .iter()
        .map(|k| ((*k).to_string(), std::env::var(k).ok()))
        .collect();
    for (k, v) in pairs {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    for k in HOST_AGENT_KEYS {
        if !pair_keys.contains(k) {
            std::env::remove_var(k);
        }
    }
    f();
    for (k, original) in originals {
        match original {
            Some(v) => std::env::set_var(&k, v),
            None => std::env::remove_var(&k),
        }
    }
}

// ---- terminal_app Ghostty 兜底(brief 20260520-ghostty-terminal-focus)----
// cc resume / compact / background reconnect 等场景 spawn hook 时
// TERM_PROGRAM 经常丢(env 来自 cc daemon 不是用户原终端),Ghostty 专属
// env 更稳。日志实测:2026-05-21 同 task_id 早上 capture 成功,下午全
// terminal_app=None,导致 click 跳转走 L2 cwd first-match 偏到错 tab。

#[test]
fn terminal_app_prefers_term_program_over_ghostty_fallback() {
    // TERM_PROGRAM 在场 → 优先采纳,即便 Ghostty 信号也存在
    with_envs(
        &[
            ("TERM_PROGRAM", Some("iTerm.app")),
            ("TERM", Some("xterm-ghostty")),
            (
                "GHOSTTY_RESOURCES_DIR",
                Some("/Applications/Ghostty.app/..."),
            ),
            ("GHOSTTY_SHELL_FEATURES", Some("cursor")),
        ],
        || {
            assert_eq!(super::terminal_app(), Some("iterm.app".to_string()));
        },
    );
}

#[test]
fn terminal_app_detects_ghostty_via_term_var() {
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("xterm-ghostty")),
            ("GHOSTTY_RESOURCES_DIR", None),
            ("GHOSTTY_SHELL_FEATURES", None),
        ],
        || {
            assert_eq!(super::terminal_app(), Some("ghostty".to_string()));
        },
    );
}

#[test]
fn terminal_app_detects_ghostty_via_resources_dir() {
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("xterm-256color")), // 通用 TERM,不该误命中
            (
                "GHOSTTY_RESOURCES_DIR",
                Some("/Applications/Ghostty.app/Contents/Resources/ghostty"),
            ),
            ("GHOSTTY_SHELL_FEATURES", None),
        ],
        || {
            assert_eq!(super::terminal_app(), Some("ghostty".to_string()));
        },
    );
}

#[test]
fn terminal_app_detects_ghostty_via_shell_features() {
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", None),
            ("GHOSTTY_RESOURCES_DIR", None),
            (
                "GHOSTTY_SHELL_FEATURES",
                Some("cursor:blink,path,sudo,title"),
            ),
        ],
        || {
            assert_eq!(super::terminal_app(), Some("ghostty".to_string()));
        },
    );
}

#[test]
fn terminal_app_returns_none_when_no_signals() {
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("xterm-256color")),
            ("GHOSTTY_RESOURCES_DIR", None),
            ("GHOSTTY_SHELL_FEATURES", None),
        ],
        || {
            assert!(super::terminal_app().is_none());
        },
    );
}

#[test]
fn terminal_app_term_check_is_case_insensitive() {
    with_envs(
        &[
            ("TERM_PROGRAM", None),
            ("TERM", Some("Xterm-Ghostty")),
            ("GHOSTTY_RESOURCES_DIR", None),
            ("GHOSTTY_SHELL_FEATURES", None),
        ],
        || {
            assert_eq!(super::terminal_app(), Some("ghostty".to_string()));
        },
    );
}

// ---- Codex PermissionRequest: pending 状态 + tool_name 落进 message ----
// Codex 0.131+ 跑命令需要审批时触发 PermissionRequest 事件,与 Claude
// Notification.permission_prompt 同语义,映射到 Pending 状态弹 popup。

// 下面两条锁 Cursor stop / session_end 带 tty 时 payload 的契约。
// 生产路径（handle_cursor_stop / handle_session_end）手写 JSON 不走
// build_payload，所以这里只是建契约基准，真正的回归由 smoke 覆盖。
// 详见 docs/briefs/20260420-cursor-stop-tty.md。

// ============ Cursor 全局安装 / 卸载 / 检查 ============

fn read_json(path: &std::path::Path) -> serde_json::Value {
    let content = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}

/// 统计某事件下 command 为 `cmd` 的条目数，用来验证"幂等 / 不重复"。
fn count_entries_with_command(val: &serde_json::Value, event: &str, cmd: &str) -> usize {
    val["hooks"][event]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|e| e["command"].as_str() == Some(cmd))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn install_cursor_writes_version_and_all_registered_events() {
    let path = sandbox_hooks_path("install-fresh");
    let cmd = "/tmp/fake-poke-hook";

    install_cursor_to_path(&path, cmd).unwrap();

    let val = read_json(&path);
    assert_eq!(val["version"], 1);
    for event in CURSOR_HOOK_EVENTS {
        let arr = val["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("missing event {}", event));
        assert_eq!(
            arr.len(),
            1,
            "event {} should have exactly one entry",
            event
        );
        assert_eq!(arr[0]["command"], cmd);
        assert_eq!(arr[0]["timeout"], 8);
    }
}

#[test]
fn install_cursor_is_idempotent_across_repeated_runs() {
    let path = sandbox_hooks_path("install-idempotent");
    let cmd = "/tmp/fake-poke-hook";

    install_cursor_to_path(&path, cmd).unwrap();
    install_cursor_to_path(&path, cmd).unwrap();

    let val = read_json(&path);
    for event in CURSOR_HOOK_EVENTS {
        assert_eq!(
            count_entries_with_command(&val, event, cmd),
            1,
            "event {} should not accumulate duplicates",
            event,
        );
    }
}

#[test]
fn install_cursor_preserves_user_custom_entries() {
    let path = sandbox_hooks_path("install-merge");
    let cmd = "/tmp/fake-poke-hook";

    // 用户原本在 sessionStart 自己注册了一个 hook，同时另外定义了
    // afterFileEdit（我们完全不覆盖）。
    let pre_existing = json!({
        "version": 1,
        "hooks": {
            "sessionStart": [
                {"command": "/opt/user/other-hook", "timeout": 5}
            ],
            "afterFileEdit": [
                {"command": "/opt/user/edit-hook"}
            ]
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&pre_existing).unwrap()).unwrap();

    install_cursor_to_path(&path, cmd).unwrap();

    let val = read_json(&path);
    // 用户 sessionStart 条目保留
    let session_start = val["hooks"]["sessionStart"].as_array().unwrap();
    assert!(session_start
        .iter()
        .any(|e| e["command"] == "/opt/user/other-hook"));
    // PokePoke 自己的条目存在且仅一条
    assert_eq!(count_entries_with_command(&val, "sessionStart", cmd), 1);
    // 用户完全不相关的 afterFileEdit 原样保留
    assert_eq!(
        val["hooks"]["afterFileEdit"][0]["command"],
        "/opt/user/edit-hook"
    );
}

#[test]
fn merge_codex_preserves_user_hooks_in_other_and_same_events() {
    // 用户已有 Codex hooks:一个完全不相关的事件,以及某个 PokePoke 也用的
    // 事件里手写的一条 group。merge 后两者都必须保留,PokePoke 只追加自己那条。
    let poke_cmd = "/tmp/fake-poke-hook --source=codex";
    let shared_event = CODEX_HOOK_EVENTS[0];

    let mut hooks_json = json!({
        "hooks": {
            "someUserOnlyEvent": [
                {"hooks": [{"type": "command", "command": "/opt/user/audit-hook"}]}
            ],
            shared_event: [
                {"hooks": [{"type": "command", "command": "/opt/user/other-hook"}]}
            ]
        }
    });

    merge_poke_hooks_into_codex(&mut hooks_json, poke_cmd);

    // 用户独占事件原样保留
    assert_eq!(
        hooks_json["hooks"]["someUserOnlyEvent"][0]["hooks"][0]["command"],
        "/opt/user/audit-hook"
    );
    // 共享事件里:用户 group 保留 + PokePoke group 追加(共两条)
    let shared = hooks_json["hooks"][shared_event].as_array().unwrap();
    assert_eq!(shared.len(), 2, "user group + poke group");
    assert!(shared
        .iter()
        .any(|g| g["hooks"][0]["command"] == "/opt/user/other-hook"));
    assert!(shared.iter().any(contains_poke_hook));
    // 所有 PokePoke 事件都写进了自己的 group
    for event in CODEX_HOOK_EVENTS {
        let arr = hooks_json["hooks"][*event].as_array().unwrap();
        assert!(
            arr.iter().any(contains_poke_hook),
            "event {} should have poke hook",
            event
        );
    }
}

#[test]
fn merge_codex_is_idempotent() {
    // 重复 merge 不应累积多条 PokePoke group。
    let poke_cmd = "/tmp/fake-poke-hook --source=codex";
    let mut hooks_json = json!({});

    merge_poke_hooks_into_codex(&mut hooks_json, poke_cmd);
    merge_poke_hooks_into_codex(&mut hooks_json, poke_cmd);

    for event in CODEX_HOOK_EVENTS {
        let arr = hooks_json["hooks"][*event].as_array().unwrap();
        let poke_count = arr.iter().filter(|g| contains_poke_hook(g)).count();
        assert_eq!(
            poke_count, 1,
            "event {} should have exactly one poke group",
            event
        );
    }
}

// ---- H2: drop_poke_hook_entries_from_groups (CC / Codex 用) ----
// review followup:此前 install/uninstall 用 group 级 retain,若用户手动把自己
// entry 塞进 PokePoke group 会被误删。改成 entry 级 retain,顺手清空 group。

#[test]
fn drop_poke_hook_entries_removes_only_our_entry_from_shared_group() {
    // 一个 group 里同时有 poke-hook + 用户自定义 entry
    let mut arr: Vec<serde_json::Value> = vec![json!({
        "hooks": [
            {"type": "command", "command": "/tmp/fake-poke-hook --source=cc"},
            {"type": "command", "command": "/opt/user/audit-hook"}
        ]
    })];

    drop_poke_hook_entries_from_groups(&mut arr);

    // group 仍在(用户 entry 撑着),poke entry 已删,用户 entry 保留
    assert_eq!(arr.len(), 1, "group with surviving user entry should stay");
    let hooks = arr[0]["hooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["command"], "/opt/user/audit-hook");
}

#[test]
fn drop_poke_hook_entries_removes_group_when_only_our_entry() {
    // group 里只有 poke-hook entry → 清空后整个 group 也从数组里删掉
    let mut arr: Vec<serde_json::Value> = vec![
        json!({
            "hooks": [
                {"type": "command", "command": "/tmp/fake-poke-hook --source=cc"}
            ]
        }),
        json!({
            "hooks": [
                {"type": "command", "command": "/opt/user/other-hook"}
            ]
        }),
    ];

    drop_poke_hook_entries_from_groups(&mut arr);

    // 只留用户 group
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["hooks"][0]["command"], "/opt/user/other-hook",
        "user group survives"
    );
}

#[test]
fn drop_poke_hook_entries_is_idempotent() {
    // 反复 drop 幂等
    let mut arr: Vec<serde_json::Value> = vec![json!({
        "hooks": [
            {"type": "command", "command": "/tmp/fake-poke-hook --source=cc"},
            {"type": "command", "command": "/opt/user/other-hook"}
        ]
    })];
    drop_poke_hook_entries_from_groups(&mut arr);
    let snapshot = arr.clone();
    drop_poke_hook_entries_from_groups(&mut arr);
    assert_eq!(arr, snapshot);
}

#[test]
fn merge_codex_preserves_user_entry_in_shared_group() {
    // review H2 for Codex:用户把自定义 entry 塞进 PokePoke 的 group 应保留
    let poke_cmd = "/tmp/fake-poke-hook --source=codex";
    let mut hooks_json = json!({
        "hooks": {
            "Stop": [{
                "hooks": [
                    {"type": "command", "command": "/tmp/fake-poke-hook --source=codex"},
                    {"type": "command", "command": "/opt/user/audit-hook"}
                ]
            }]
        }
    });

    merge_poke_hooks_into_codex(&mut hooks_json, poke_cmd);

    let stop_arr = hooks_json["hooks"]["Stop"].as_array().unwrap();
    // 应保留用户 entry;不应因 group 被整体清掉而丢失
    let all_cmds: Vec<&str> = stop_arr
        .iter()
        .flat_map(|g| g["hooks"].as_array().unwrap())
        .filter_map(|h| h["command"].as_str())
        .collect();
    assert!(
        all_cmds.contains(&"/opt/user/audit-hook"),
        "user entry lost after merge; cmds={:?}",
        all_cmds
    );
    assert!(
        all_cmds.contains(&poke_cmd),
        "poke entry missing after merge; cmds={:?}",
        all_cmds
    );
}

#[test]
fn install_cursor_aborts_on_corrupt_existing_file() {
    let path = sandbox_hooks_path("install-corrupt");
    std::fs::write(&path, "{ this is not valid json").unwrap();

    // 破损配置禁止静默覆盖，必须返回错误（见 brief 风险 1）。
    let err = install_cursor_to_path(&path, "/tmp/fake-poke-hook").unwrap_err();
    assert!(err.contains("Failed to parse"), "error was: {}", err);

    // 文件内容保持原状，没有被覆写。
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(after, "{ this is not valid json");
}

#[test]
fn uninstall_cursor_only_removes_our_entries() {
    let path = sandbox_hooks_path("uninstall-mixed");
    let cmd = "/tmp/fake-poke-hook";

    // 先装一次，再手动混入一个用户自定义条目
    install_cursor_to_path(&path, cmd).unwrap();
    {
        let mut val = read_json(&path);
        val["hooks"]["sessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({"command": "/opt/user/other-hook"}));
        val["hooks"]["afterFileEdit"] = json!([{"command": "/opt/user/edit-hook"}]);
        std::fs::write(&path, serde_json::to_string_pretty(&val).unwrap()).unwrap();
    }

    uninstall_cursor_from_path(&path).unwrap();

    let val = read_json(&path);
    // 版本字段保留（与 CC 卸载风格一致）
    assert_eq!(val["version"], 1);
    // PokePoke 自己的 stop / beforeSubmitPrompt / sessionEnd 都清空，空事件被移除
    assert!(val["hooks"].get("stop").is_none());
    assert!(val["hooks"].get("beforeSubmitPrompt").is_none());
    assert!(val["hooks"].get("sessionEnd").is_none());
    // 用户的 sessionStart / afterFileEdit 不动
    let session_start = val["hooks"]["sessionStart"].as_array().unwrap();
    assert_eq!(session_start.len(), 1);
    assert_eq!(session_start[0]["command"], "/opt/user/other-hook");
    assert_eq!(
        val["hooks"]["afterFileEdit"][0]["command"],
        "/opt/user/edit-hook"
    );
}

#[test]
fn uninstall_cursor_leaves_only_version_when_only_pokepoke_entries_existed() {
    let path = sandbox_hooks_path("uninstall-pokepoke-only");
    install_cursor_to_path(&path, "/tmp/fake-poke-hook").unwrap();

    uninstall_cursor_from_path(&path).unwrap();

    let val = read_json(&path);
    assert_eq!(val["version"], 1);
    // 对齐 CC 卸载：文件保留，仅留 version 字段
    assert!(val.get("hooks").is_none() || val["hooks"].as_object().unwrap().is_empty());
}

#[test]
fn uninstall_cursor_handles_missing_file_gracefully() {
    let path = sandbox_hooks_path("uninstall-missing");
    // 不预写任何文件
    let msg = uninstall_cursor_from_path(&path).unwrap();
    assert!(msg.contains("Cursor"), "msg: {}", msg);
    assert!(!path.exists(), "should not create file when none existed");
}

#[test]
fn cursor_hooks_configured_recognizes_three_states() {
    // 1) 未安装：文件不存在
    let missing = sandbox_hooks_path("check-missing");
    assert!(!cursor_hooks_configured(&missing));

    // 2) 部分安装：仅 sessionStart 有我们的条目——视为未接入
    let partial = sandbox_hooks_path("check-partial");
    let partial_val = json!({
        "version": 1,
        "hooks": {
            "sessionStart": [{"command": "/tmp/fake-poke-hook", "timeout": 8}],
        }
    });
    std::fs::write(
        &partial,
        serde_json::to_string_pretty(&partial_val).unwrap(),
    )
    .unwrap();
    assert!(!cursor_hooks_configured(&partial));

    // 3) 完整接入：四个事件都有 PokePoke 条目
    let full = sandbox_hooks_path("check-full");
    install_cursor_to_path(&full, "/tmp/fake-poke-hook").unwrap();
    assert!(cursor_hooks_configured(&full));
}

// ==================== UDS transport ====================

use super::post_unix;
use std::io::Read as _;
use std::os::unix::net::UnixListener;
use tempfile::tempdir;

/// 起一个临时 UnixListener,accept 一次后用 `response` 决定回什么。
/// 阻塞接受,handler 在新线程跑,让主线程能继续调 post_unix。
fn spawn_uds_stub(sock: PathBuf, response: Vec<u8>) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::Write as _;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&response);
        }
    })
}

#[test]
fn post_unix_returns_true_on_2xx() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("ok.sock");
    let h = spawn_uds_stub(
        sock.clone(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    );
    assert!(post_unix(&sock, "{}"));
    let _ = h.join();
}

#[test]
fn post_unix_returns_false_on_5xx() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("err.sock");
    let h = spawn_uds_stub(
        sock.clone(),
        b"HTTP/1.1 500 Internal Server Error\r\n\r\n".to_vec(),
    );
    assert!(!post_unix(&sock, "{}"));
    let _ = h.join();
}

#[test]
fn post_unix_returns_false_when_server_closes_without_response() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("close.sock");
    let h = spawn_uds_stub(sock.clone(), Vec::new());
    assert!(!post_unix(&sock, "{}"));
    let _ = h.join();
}

#[test]
fn post_unix_returns_false_when_sock_missing() {
    let dir = tempdir().unwrap();
    let sock = dir.path().join("nope.sock");
    // 没人 listen
    assert!(!post_unix(&sock, "{}"));
}

// ==================== opencode integration ====================

#[test]
fn source_opencode_roundtrips_arg_and_labels() {
    assert_eq!(Source::from_arg("opencode"), Some(Source::OpenCode));
    assert_eq!(Source::OpenCode.as_str(), "opencode");
    assert_eq!(Source::OpenCode.task_id_prefix(), "opencode");

    let argv = vec!["poke-hook".to_string(), "--source=opencode".to_string()];
    assert_eq!(parse_source_arg(&argv), Some(Source::OpenCode));
}

#[test]
#[serial]
fn opencode_plugin_contents_bakes_absolute_hook_path() {
    let contents = opencode_plugin_contents();
    // 占位符必须被替换,不能残留
    assert!(!contents.contains("__POKE_HOOK_BIN__"));
    // 必须含 PokePoke 标记 + source 调用(check/verify 都靠这两个子串判定)
    assert!(contents.contains("PokePoke opencode integration"));
    assert!(contents.contains("--source=opencode"));
    // 烘焙进去的是绝对路径 poke-hook
    assert!(contents.contains("/.local/bin/poke-hook"));
}

#[test]
#[serial]
fn opencode_plugin_guards_error_round_debounce_stop() {
    // 回归护栏:同一 error 轮的 idle debounce 到期后不得再补发 Stop(完成音),
    // 否则用户先听到失败音、10s 后再听到一声完成音(语义错乱)。
    // include_str! 只嵌入字符串不校验 JS,内容断言是唯一编译期防回归手段。
    let contents = opencode_plugin_contents();
    // error 轮标记 Set 存在
    assert!(contents.contains("const erroredSessions = new Set()"));
    // session.error 分支先标记再 forward StopFailure
    assert!(contents.contains("remember(erroredSessions, p.sessionID)"));
    // idle debounce 回调开头有 guard:命中标记则丢弃,不发 Stop
    assert!(contents.contains("if (erroredSessions.has(sid))"));
    // error 到达时清理 pending idle 定时器
    assert!(contents.contains("idleTimers.has(p.sessionID)"));
    // 新一轮开始清标记(session.created / 用户新消息),不污染后续正常完成音
    assert!(contents.contains("erroredSessions.delete(p.info.id)"));
    assert!(contents.contains("erroredSessions.delete(mid)"));
}

#[test]
#[serial]
fn opencode_plugin_configured_requires_both_markers() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("pokepoke.js");

    // 我方 plugin 全文 → configured
    std::fs::write(&path, opencode_plugin_contents()).unwrap();
    assert!(opencode_plugin_configured(&path));

    // 用户自己的无关 plugin → 未接入
    std::fs::write(&path, "export const Other = async () => ({})\n").unwrap();
    assert!(!opencode_plugin_configured(&path));

    // 不存在 → 未接入
    std::fs::remove_file(&path).unwrap();
    assert!(!opencode_plugin_configured(&path));
}

#[test]
#[serial]
fn opencode_plugin_outdated_when_baked_path_differs() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("pokepoke.js");

    // 当前烘焙路径 → 不过期
    std::fs::write(&path, opencode_plugin_contents()).unwrap();
    assert!(!opencode_plugin_outdated(&path));

    // 含标记但 hook 路径是别的(模拟旧安装/搬家)→ 过期
    let stale = opencode_plugin_contents().replace("/.local/bin/poke-hook", "/old/path/poke-hook");
    std::fs::write(&path, stale).unwrap();
    assert!(opencode_plugin_outdated(&path));

    // 非我方 plugin → 不判过期(不是我们的,不管)
    std::fs::write(&path, "export const Other = async () => ({})\n").unwrap();
    assert!(!opencode_plugin_outdated(&path));
}

#[test]
fn plan_snapshot_path_uses_js_ext_for_opencode() {
    let dir = PathBuf::from("/tmp/pp");
    let path = plan_snapshot_path(&dir, "opencode", 1745332800);
    assert_eq!(
        path,
        dir.join("snapshots/opencode-1745332800.js"),
        "opencode snapshot 应为 .js 扩展名"
    );
}

#[test]
fn plan_snapshot_path_uses_ts_ext_for_pi() {
    let dir = PathBuf::from("/tmp/pp");
    let path = plan_snapshot_path(&dir, "pi", 1745332800);
    assert_eq!(
        path,
        dir.join("snapshots/pi-1745332800.ts"),
        "pi snapshot 应为 .ts 扩展名"
    );
}

// ==================== pi integration ====================

#[test]
#[serial]
fn pi_extension_contents_bakes_absolute_hook_path() {
    let contents = pi_extension_contents();
    // 占位符必须被替换,不能残留
    assert!(!contents.contains("__POKE_HOOK_BIN__"));
    // 必须含 PokePoke 标记 + source 调用(check/verify 都靠这两个子串判定)
    assert!(contents.contains("PokePoke pi integration"));
    assert!(contents.contains("--source=pi"));
    // 烘焙进去的是绝对路径 poke-hook
    assert!(contents.contains("/.local/bin/poke-hook"));
    // 事件映射关键字(防模板漂移)
    assert!(contents.contains("agent_settled"));
    assert!(contents.contains("session_start"));
    assert!(contents.contains("session_shutdown"));
}

#[test]
#[serial]
fn pi_extension_configured_requires_both_markers() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("index.ts");

    // 我方扩展全文 -> configured
    std::fs::write(&path, pi_extension_contents()).unwrap();
    assert!(pi_extension_configured(&path));

    // 用户自己的无关扩展 -> 未接入
    std::fs::write(&path, "export default function () {}\n").unwrap();
    assert!(!pi_extension_configured(&path));

    // 不存在 -> 未接入
    std::fs::remove_file(&path).unwrap();
    assert!(!pi_extension_configured(&path));
}

#[test]
#[serial]
fn pi_extension_outdated_when_baked_path_differs() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("index.ts");

    // 当前烘焙路径 -> 不过期
    std::fs::write(&path, pi_extension_contents()).unwrap();
    assert!(!pi_extension_outdated(&path));

    // 含标记但 hook 路径是别的(模拟旧安装/搬家)-> 过期
    let stale = pi_extension_contents().replace("/.local/bin/poke-hook", "/old/path/poke-hook");
    std::fs::write(&path, stale).unwrap();
    assert!(pi_extension_outdated(&path));

    // 非我方扩展 -> 不判过期(不是我们的,不管)
    std::fs::write(&path, "export default function () {}\n").unwrap();
    assert!(!pi_extension_outdated(&path));
}

// ==================== cmux env collection ====================
// brief 20260709-cmux-jump-support:cmux 用 libghostty,pane 内 `TERM_PROGRAM=ghostty`
// 会误触发真 Ghostty capture。terminal_app() 顶部靠 CMUX_SURFACE_ID 短路成 "cmux",
// server gate 只匹配 "ghostty" 天然过掉;payload 里同时上报 cmux_surface_id / workspace_id
// 触发 short-circuit `cmux focus-panel`。

#[test]
fn terminal_app_short_circuits_to_cmux_when_env_present() {
    // CMUX_SURFACE_ID 非空 → 覆盖 TERM_PROGRAM=ghostty
    with_envs(
        &[
            ("CMUX_SURFACE_ID", Some("SURF-UUID-1")),
            ("TERM_PROGRAM", Some("ghostty")),
        ],
        || {
            assert_eq!(terminal_app().as_deref(), Some("cmux"));
        },
    );
}

#[test]
fn terminal_app_falls_back_to_ghostty_when_cmux_env_empty() {
    // CMUX_SURFACE_ID 空串 → 视作不在 cmux
    with_envs(
        &[
            ("CMUX_SURFACE_ID", Some("")),
            ("TERM_PROGRAM", Some("ghostty")),
        ],
        || {
            assert_eq!(terminal_app().as_deref(), Some("ghostty"));
        },
    );
}

#[test]
fn terminal_app_falls_back_when_cmux_env_unset() {
    with_envs(
        &[
            ("CMUX_SURFACE_ID", None),
            ("TERM_PROGRAM", Some("iTerm.app")),
        ],
        || {
            assert_eq!(terminal_app().as_deref(), Some("iterm.app"));
        },
    );
}

// ==================== tmux env passthrough ====================
// brief 20260804-tmux-binding:TMUX / TMUX_PANE 两件套全有才上报 tmux binding;
// herdr/cmux/wezterm/otty env 存在时不采集(优先级最低)。

// ==================== pi agent ====================

#[test]
fn pi_source_roundtrip() {
    assert_eq!(Source::Pi.as_str(), "pi");
    assert_eq!(Source::Pi.task_id_prefix(), "pi");
    assert_eq!(Source::from_arg("pi"), Some(Source::Pi));
}

#[test]
fn pi_session_start_payload_maps_running() {
    let data = json!({
        "session_id": "019f89bd-6cee-7635-a2e9-aa5886a5bff1",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("SessionStart", &data, Source::Pi, None).unwrap();
    assert_eq!(payload["task_id"], "pi-019f89bd");
    assert_eq!(payload["event_type"], "running");
    assert_eq!(payload["status"], "running");
    assert_eq!(payload["source"], "pi");
    assert_eq!(payload["workspace_path"], "/Users/edy/proj");
    assert_eq!(
        payload["external_session_id"],
        "019f89bd-6cee-7635-a2e9-aa5886a5bff1"
    );
}

#[test]
fn pi_stop_payload_maps_idle() {
    let data = json!({
        "session_id": "019f89bd-6cee-7635-a2e9-aa5886a5bff1",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("Stop", &data, Source::Pi, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["source"], "pi");
    assert_eq!(payload["message"], "Session completed");
    // 正常完成不带 aborted 标记(server 会播完成音)。
    assert!(payload.get("aborted").is_none());
}

#[test]
fn pi_stop_payload_aborted_carries_flag() {
    // brief 20260820-pi-aborted-no-completion-sound:pi 扩展在用户取消时随 Stop
    // 上报 aborted:true,透传到 server 的 /notify payload。
    let data = json!({
        "session_id": "019f89bd-6cee-7635-a2e9-aa5886a5bff1",
        "cwd": "/Users/edy/proj",
        "aborted": true
    });

    let payload = build_payload("Stop", &data, Source::Pi, None).unwrap();
    assert_eq!(payload["event_type"], "stop");
    assert_eq!(payload["status"], "idle");
    assert_eq!(payload["source"], "pi");
    assert_eq!(payload["aborted"], true);
}

#[test]
fn pi_stop_payload_aborted_false_omits_flag() {
    // 显式 false 也不输出字段(只有 true 才有语义)。
    let data = json!({
        "session_id": "019f89bd-6cee-7635-a2e9-aa5886a5bff1",
        "cwd": "/Users/edy/proj",
        "aborted": false
    });

    let payload = build_payload("Stop", &data, Source::Pi, None).unwrap();
    assert!(payload.get("aborted").is_none());
}

#[test]
fn pi_session_end_payload_maps_session_end() {
    let data = json!({
        "session_id": "019f89bd-6cee-7635-a2e9-aa5886a5bff1",
        "cwd": "/Users/edy/proj"
    });

    let payload = build_payload("SessionEnd", &data, Source::Pi, None).unwrap();
    assert_eq!(payload["event_type"], "session_end");
    assert_eq!(payload["source"], "pi");
}

#[test]
fn session_end_production_payload_carries_source_for_pi() {
    let payload = session_end_payload(
        "pi-019f89bd",
        "proj",
        "/Users/edy/proj",
        Source::Pi,
        Some("/dev/ttys012"),
        Some("019f89bd-6cee-7635-a2e9-aa5886a5bff1"),
    );
    assert_eq!(payload["event_type"], "session_end");
    assert_eq!(payload["source"], "pi");
    assert_eq!(payload["terminal_tty"], "/dev/ttys012");
    assert_eq!(
        payload["external_session_id"],
        "019f89bd-6cee-7635-a2e9-aa5886a5bff1"
    );
}

// ==================== kimi code ====================

#[test]
fn parse_source_arg_recognizes_kimi() {
    assert_eq!(
        parse_source_arg(&["--source=kimi".into()]),
        Some(Source::Kimi)
    );
    assert_eq!(Source::Kimi.as_str(), "kimi");
    assert_eq!(Source::Kimi.task_id_prefix(), "kimi");
    assert_eq!(Source::from_arg("kimi"), Some(Source::Kimi));
}

#[test]
fn kimi_build_payload_session_start_is_running() {
    let payload =
        json!({"hook_event_name":"SessionStart","session_id":"abc12345","cwd":"/tmp/proj"});
    let out = build_payload("SessionStart", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["source"], "kimi");
    assert_eq!(out["status"], "running");
    assert_eq!(out["task_id"], "kimi-abc12345");
}

#[test]
fn kimi_task_id_strips_session_prefix_before_shortening() {
    let payload = json!({
        "hook_event_name": "SessionStart",
        "session_id": "session_79ecd469-2724-40f4-9335-33d0f5c3d7cc",
        "cwd": "/tmp/proj"
    });
    assert_eq!(
        build_task_id(&payload, Source::Kimi),
        "kimi-79ecd469-2724-40f4-9335-33d0f5c3d7cc"
    );
}

#[test]
fn normalize_event_maps_subagent_hooks() {
    // Kimi 用 SubagentStart/SubagentStop,Cursor 用 subagentStart/subagentStop。
    assert_eq!(normalize_event("subagent_start"), "SubagentStart");
    assert_eq!(normalize_event("subagentStart"), "SubagentStart");
    assert_eq!(normalize_event("subagent_stop"), "SubagentStop");
    assert_eq!(normalize_event("subagentStop"), "SubagentStop");
}

#[test]
fn kimi_build_payload_user_prompt_is_running() {
    let payload =
        json!({"hook_event_name":"UserPromptSubmit","session_id":"abc12345","cwd":"/tmp/proj"});
    let out = build_payload("UserPromptSubmit", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["status"], "running");
}

#[test]
fn kimi_build_payload_stop_is_idle() {
    let payload = json!({"hook_event_name":"Stop","session_id":"abc12345","cwd":"/tmp/proj"});
    let out = build_payload("Stop", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["event_type"], "stop");
    assert_eq!(out["status"], "idle");
}

#[test]
fn kimi_build_payload_permission_request_is_pending_with_tool_name() {
    let payload = json!({"hook_event_name":"PermissionRequest","session_id":"abc12345","cwd":"/tmp/proj","tool_name":"Bash"});
    let out = build_payload("PermissionRequest", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["status"], "pending");
    assert!(out["message"].as_str().unwrap().contains("Bash"));
}

#[test]
fn kimi_build_payload_permission_request_without_tool_name_falls_back() {
    let payload =
        json!({"hook_event_name":"PermissionRequest","session_id":"abc12345","cwd":"/tmp/proj"});
    let out = build_payload("PermissionRequest", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["message"], "Awaiting approval");
}

#[test]
fn hook_permission_request_source_gate_includes_cc_kimi_and_codex() {
    assert!(accepts_permission_request_source(Source::ClaudeCode));
    assert!(accepts_permission_request_source(Source::Codex));
    assert!(accepts_permission_request_source(Source::Kimi));
    assert!(!accepts_permission_request_source(Source::Cursor));
    assert!(!accepts_permission_request_source(Source::OpenCode));
    assert!(!accepts_permission_request_source(Source::Pi));
}

#[test]
fn hook_stop_failure_source_gate_includes_kimi_and_pi() {
    assert!(accepts_stop_failure_source(Source::ClaudeCode));
    assert!(accepts_stop_failure_source(Source::OpenCode));
    assert!(accepts_stop_failure_source(Source::Kimi));
    assert!(accepts_stop_failure_source(Source::Pi));
    assert!(!accepts_stop_failure_source(Source::Codex));
    assert!(!accepts_stop_failure_source(Source::Cursor));
}

#[test]
fn kimi_build_payload_stop_failure_is_last_failed() {
    let payload = json!({"hook_event_name":"StopFailure","session_id":"abc12345","cwd":"/tmp/proj","error":"timeout"});
    let out = build_payload("StopFailure", &payload, Source::Kimi, None).unwrap();
    assert_eq!(out["status"], "last_failed");
    assert_eq!(out["failure_reason"], "timeout");
}

#[test]
fn pi_build_payload_stop_failure_is_last_failed() {
    // pi 扩展(2026-08-04)在 agent_settled 时按 stopReason=error 转发
    // StopFailure:reason=provider_error + detail=截断错误原文(≤300 字符)。
    let payload = json!({
        "hook_event_name": "StopFailure",
        "session_id": "pi-session-1",
        "cwd": "/tmp/proj",
        "reason": "provider_error",
        "detail": "HTTP 429 · Account quota exceeded"
    });
    let out = build_payload("StopFailure", &payload, Source::Pi, None).unwrap();
    assert_eq!(out["status"], "last_failed");
    assert_eq!(out["failure_reason"], "provider_error");
    assert_eq!(out["message"], "HTTP 429 · Account quota exceeded");
    assert_eq!(out["source"], "pi");
    assert_eq!(out["task_id"], "pi-pi-sessi");
}

#[test]
#[serial]
fn kimi_merge_into_empty_config_creates_hooks() {
    let mut doc = toml_edit::DocumentMut::new();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    let arr = doc
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
        .unwrap();
    assert_eq!(arr.len(), KIMI_HOOK_EVENTS.len());
    assert!(kimi_hooks_configured(&doc));
    assert!(!kimi_hooks_outdated(&doc));
}

#[test]
#[serial]
fn kimi_hooks_outdated_true_when_event_missing_from_previous_install() {
    // 旧版本装机缺 SubagentStart / SubagentStop(事件集扩展前)。command 是
    // canonical 的,但事件不全 → 必须判 outdated,否则 auto-connect 会当成
    // "用户断开"跳过,设置面板一直显示未接入。
    let mut doc = toml_edit::DocumentMut::new();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    assert!(!kimi_hooks_outdated(&doc));

    // 删掉 SubagentStart / SubagentStop 两条(模拟旧装机)
    let arr = doc
        .get_mut("hooks")
        .and_then(|h| h.as_array_of_tables_mut())
        .unwrap();
    let to_remove: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.get("event").and_then(|i| i.as_str()) == Some("SubagentStart")
                || t.get("event").and_then(|i| i.as_str()) == Some("SubagentStop")
        })
        .map(|(i, _)| i)
        .collect();
    for i in to_remove.into_iter().rev() {
        arr.remove(i);
    }
    assert!(!kimi_hooks_configured(&doc));
    assert!(kimi_hooks_outdated(&doc));

    // 完全没有我们的 entry(从未安装 / 用户断开)→ 不算 outdated
    let empty = toml_edit::DocumentMut::new();
    assert!(!kimi_hooks_outdated(&empty));
    let user_only = r#"
[[hooks]]
event = "SessionStart"
command = "/usr/bin/true"
"#
    .parse::<toml_edit::DocumentMut>()
    .unwrap();
    assert!(!kimi_hooks_outdated(&user_only));
}

#[test]
#[serial]
fn kimi_merge_preserves_user_config_and_user_hooks() {
    let mut doc = r#"
default_model = "deepseek/v4"
[providers.deepseek]
type = "openai"
[[hooks]]
event = "SessionStart"
command = "/usr/bin/true"
"#
    .parse::<toml_edit::DocumentMut>()
    .unwrap();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    assert_eq!(
        doc.get("default_model").and_then(|v| v.as_str()),
        Some("deepseek/v4")
    );
    assert!(doc.get("providers").is_some());
    let arr = doc
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
        .unwrap();
    assert!(arr
        .iter()
        .any(|t| { t.get("command").and_then(|v| v.as_str()) == Some("/usr/bin/true") }));
    assert!(kimi_hooks_configured(&doc));
}

#[test]
#[serial]
fn kimi_merge_is_idempotent() {
    let mut doc = toml_edit::DocumentMut::new();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    let arr = doc
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
        .unwrap();
    assert_eq!(arr.len(), KIMI_HOOK_EVENTS.len());
}

#[test]
#[serial]
fn kimi_merge_replaces_outdated_path() {
    let mut doc = toml_edit::DocumentMut::new();
    let old_cmd = "/old/home/.local/bin/poke-hook --source=kimi";
    merge_poke_hooks_into_kimi(&mut doc, old_cmd);
    assert!(kimi_hooks_configured(&doc));
    assert!(kimi_hooks_outdated(&doc));
    let new_cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &new_cmd);
    assert!(!kimi_hooks_outdated(&doc));
    let arr = doc
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
        .unwrap();
    assert_eq!(arr.len(), KIMI_HOOK_EVENTS.len());
    assert!(arr
        .iter()
        .all(|t| { t.get("command").and_then(|v| v.as_str()) != Some(old_cmd) }));
}

#[test]
#[serial]
fn kimi_remove_strips_ours_keeps_user_hooks() {
    let mut doc = r#"
[[hooks]]
event = "SessionStart"
command = "/usr/bin/true"
[[hooks]]
event = "Stop"
command = "/usr/bin/true"
"#
    .parse::<toml_edit::DocumentMut>()
    .unwrap();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    remove_poke_hooks_from_kimi(&mut doc);
    let arr = doc
        .get("hooks")
        .and_then(|h| h.as_array_of_tables())
        .unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr
        .iter()
        .all(|t| { t.get("command").and_then(|v| v.as_str()) == Some("/usr/bin/true") }));
    assert!(!kimi_hooks_configured(&doc));
}

#[test]
#[serial]
fn kimi_remove_removes_empty_hooks_key() {
    let mut doc = toml_edit::DocumentMut::new();
    let cmd = expected_hook_command("kimi");
    merge_poke_hooks_into_kimi(&mut doc, &cmd);
    remove_poke_hooks_from_kimi(&mut doc);
    assert!(doc.get("hooks").is_none());
}

#[test]
fn kimi_command_is_ours_does_not_match_other_sources() {
    assert!(!kimi_command_is_ours("/bin/poke-hook --source=cc"));
    assert!(!kimi_command_is_ours("/bin/poke-hook --source=codex"));
    assert!(!kimi_command_is_ours("/bin/poke-hook --source=pi"));
    assert!(kimi_command_is_ours(
        "/x/.local/bin/poke-hook --source=kimi"
    ));
}

#[path = "build_payload_tests.rs"]
mod build_payload_tests;
#[path = "detect_source_tests.rs"]
mod detect_source_tests;
#[path = "drift_tests.rs"]
mod drift_tests;
#[path = "grok_tests.rs"]
mod grok_tests;
#[path = "omp_tests.rs"]
mod omp_tests;

#[path = "snapshot_tests.rs"]
mod snapshot_tests;
