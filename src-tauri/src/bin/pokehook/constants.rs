//! 常量:lock 目录、各 agent 的事件注册表、CLI 子命令参数表。
//! 从 `pokehook.rs` 抽出(AGENTS.md §4a),行为零变化。

pub(crate) const LOCK_DIR: &str = "/tmp";

// Events we register for Claude Code.
// PermissionRequest(2026-08-19 起订阅):CC 弹权限确认时**立即**触发
// PermissionRequest;Notification.permission_prompt 只在权限框等 ~6s 无人理后
// 才 fire。若只订 Notification,用户在 6s 内点确认时 PokePoke 完全收不到事件、
// popup 不弹。因此 PermissionRequest 才是 CC "等用户确认" 的主信号,与
// Codex/Kimi 同语义(映射 Pending)。PermissionRequest 是 decision hook,但我们
// 只观察不决策(返回 `{}`),不拦截 CC 的权限流程。6s 后仍可能 fire
// Notification.permission_prompt,彼时 status 已是 Pending,server 端走
// refresh-only,不会双弹窗/双响。
pub(crate) const CC_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "Notification",
    "PermissionRequest",
    "Stop",
    "StopFailure",
];
// Events we register for Codex CLI. 当前订阅四类:
//   - SessionStart / UserPromptSubmit / Stop:基础生命周期
//   - PermissionRequest:agent 跑命令需要审批,与 Claude `Notification.permission_prompt`
//     同语义("等用户允许"),映射到 Pending 状态弹 popup。Codex 0.131+ 还要求
//     用户在 TUI 里 `/hooks` 信任一次,才会真正 fire(README 已提示)。
pub(crate) const CODEX_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "Stop",
    "PermissionRequest",
];
// Events we register for Kimi Code CLI(`~/.kimi-code/config.toml` 的 `[[hooks]]`)。
// Kimi 的 hook 契约与 Claude Code 几乎一致(同名事件 + stdin JSON + exit 2 block),
// 区别仅在配置格式是 TOML array-of-tables 而非 JSON。事件集对齐 CC 的 proven
// 组合,只把 CC 的 `Notification`(cc 的权限信号)换成 Kimi 独立的 `PermissionRequest`
// (Kimi 把"工具待审批"拆成了独立事件,与 cc `Notification.permission_prompt` 同语义):
//   - SessionStart / UserPromptSubmit / Stop / StopFailure:基础生命周期,与 cc 同构
//   - PermissionRequest:工具待审批 -> Pending 弹窗
//   - SubagentStart / SubagentStop:Kimi `Agent` 工具子 agent;main Stop 可能在子 agent
//     仍跑时 fire(2026-07-31 真机误弹)。Start 计数 +1、Stop 计数 -1;main `Stop` 在
//     计数 >0 时 suppress(对齐 CC `background_tasks` 语义,不走 PreToolUse)。
// 不注册 `Notification`:Kimi 的 notification_type(如 task.completed)语义未经真机
// 确认,若每个 turn 末尾与 Stop 同时 fire 会双弹窗。Stop 已覆盖"完成"、
// PermissionRequest 已覆盖"待审批",Notification 留待真机验证 firing 语义后再加
// (项目记忆:解析外部数据必须真实样本验证)。
// SessionEnd 不注册:靠 session_lifecycle 的 TTY 探活收割兜底(与 cc 一致)。
pub(crate) const KIMI_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "StopFailure",
    "PermissionRequest",
];
// Events we register for Cursor。
// subagentStart/subagentStop:Task 工具子代理生命周期(见 docs/hook-events.md)。
// 复用 subagents.rs 跨进程计数,main stop 在子代理计数 >0 时 suppress
// (对齐 Kimi 的 SubagentStart/SubagentStop 语义,2026-08-04 grok review §4-②)。
pub(crate) const CURSOR_HOOK_EVENTS: &[&str] = &[
    "sessionStart",
    "beforeSubmitPrompt",
    "subagentStart",
    "subagentStop",
    "stop",
    "sessionEnd",
];
// opencode 接入的事件由 plugin JS 侧订阅(见 OPENCODE_PLUGIN_JS),plugin 会把每个
// opencode event 归一成 poke-hook 已认识的扁平事件名 + 字段,再 shell-out 调本二进制。
// 因此 Rust 侧不像 CC/Codex/Cursor 那样注册"事件名列表到外部配置",接入单元是
// 一整个 plugin 文件。plugin 归一后上报的内部事件名:SessionStart / UserPromptSubmit /
// Notification(→ Pending)/ Stop / StopFailure。

/// CLI 子命令参数表:hook binary 以任一参数被调用时按 CLI 模式走
/// (main 里 `Some("--install")` 等逐 arm 分发),不是 hook 事件模式。
pub(crate) const HOOK_CLI_ARGS: &[&str] = &[
    "--install",
    "--uninstall",
    "--check",
    "--install-codex",
    "--uninstall-codex",
    "--check-codex",
    "--install-cursor",
    "--uninstall-cursor",
    "--check-cursor",
    "--install-opencode",
    "--uninstall-opencode",
    "--check-opencode",
    "--install-pi",
    "--uninstall-pi",
    "--check-pi",
    "--install-kimi",
    "--uninstall-kimi",
    "--check-kimi",
    "--install-grok",
    "--uninstall-grok",
    "--check-grok",
    "--install-dsh",
    "--uninstall-dsh",
    "--check-dsh",
    "--deploy-self",
];
