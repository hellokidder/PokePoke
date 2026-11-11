import type { DocsCopy } from "./docsCopy";

export const docsCopyZh: DocsCopy = {
  header: {
    manual: "POKEPOKE(1) — 用户手册",
    tagline: "agent 戳你一下,你一键跳回去。这份手册讲清楚怎么用。",
    backHome: "首页",
    download: "下载",
  },
  sections: [
    {
      id: "quickstart",
      index: "1",
      title: "快速开始",
      blocks: [
        {
          kind: "para",
          text: "系统要求:macOS 13 (Ventura) 及以上,Apple Silicon (arm64)。Intel Mac 未官方打包,可自行构建。",
        },
        {
          kind: "list",
          items: [
            "从 Releases 下载最新 `PokePoke_<version>_aarch64.dmg`,拖进 `/Applications`",
            "首次启动若提示「无法验证开发者」:`系统设置 → 隐私与安全性`,允许打开",
            "启动后菜单栏出现八爪鱼图标即就绪;首次启动会自动为已检测到的 agent 写入 hook,无需手动接入",
          ],
        },
        {
          kind: "note",
          text: "首次点击 popup 跳回终端时,macOS 会弹一次「PokePoke 想要控制 …」授权,点允许即可——每个终端宿主系统只问一次。",
        },
      ],
    },
    {
      id: "agents",
      index: "2",
      title: "接入 agent",
      blocks: [
        {
          kind: "para",
          text: "所有接入都是菜单栏图标 → 对应 agent 的 **Connect** 一键完成;Disconnect / Repair 在同一菜单。写入的全部是用户级配置,不动项目目录。",
        },
        {
          kind: "table",
          head: ["Agent", "写入位置", "事件"],
          rows: [
            [
              "Claude Code",
              "`~/.claude/settings.json`",
              "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
            ],
            [
              "Cursor",
              "`~/.cursor/hooks.json`(全局)",
              "sessionStart / beforeSubmitPrompt / stop / sessionEnd",
            ],
            [
              "Codex CLI",
              "`~/.codex/hooks.json` + `config.toml`",
              "SessionStart / UserPromptSubmit / Stop",
            ],
            [
              "opencode",
              "`~/.config/opencode/plugins/pokepoke.js`",
              "session.idle / permission.ask / event",
            ],
            [
              "pi",
              "`~/.pi/agent/extensions/pokepoke/`",
              "session_start / before_agent_start / agent_settled",
            ],
            [
              "Kimi Code",
              "`~/.kimi-code/config.toml`",
              "SessionStart / UserPromptSubmit / Stop / StopFailure / PermissionRequest",
            ],
            ["Grok Build", "`~/.grok`(CC 兼容 hook)", "与 Claude Code 同名事件"],
          ],
        },
        {
          kind: "note",
          text: "Codex CLI ≥0.131 需要在 TUI 里 `/hooks` 手动 trust 一次 `poke-hook`,否则 Codex 静默不 fire hook。Cursor 若同时开启 Third-party hooks 会双触发(后端有 100ms 去重兜底,建议二选一)。pi 仅在 TUI 模式上报、无 pending 状态——是设计如此,不是缺口。",
        },
        {
          kind: "para",
          text: "升级后若 hook 协议过期,panel 顶部会出现「Hook 配置过期」告警 + 一键修复按钮。",
        },
      ],
    },
    {
      id: "daily",
      index: "3",
      title: "日常使用",
      blocks: [
        {
          kind: "list",
          items: [
            "**Panel**:菜单栏图标展开,每个活跃 session 一行,四种状态 — Running(跑图中)/ Pending(等你输入)/ Idle(干完了)/ Failed(报错)",
            "**Popup**:状态变化时从屏幕角落弹出 + 提示音;不点不走,绝不会自动消失打断你",
            "**跳回**:点 popup 或按 `⌘N` 跳回对应终端 / IDE 窗口(修饰键可在设置里改)",
            "**全局开关**:panel 左下角 toggle,关掉后状态变化只静默记录,不弹不放音",
            "**设置**:提示音(任意系统音/静音)、语言、菜单栏图标(彩色/固定白)、popup 屏幕四角位置",
          ],
        },
      ],
    },
    {
      id: "herdr-remote",
      index: "4",
      title: "herdr 远程模式",
      blocks: [
        {
          kind: "para",
          text: "适用场景:你正在用的机器装 PokePoke,agent 跑在**远程机**的 herdr 里--远程机**零安装**,状态变化在你面前的屏幕上弹 PokePoke popup,而不是 herdr toast。",
        },
        { kind: "para", text: "正确用法只有一步:" },
        {
          kind: "code",
          lines: [
            "# 像往常一样用 herdr 连上远程机",
            "herdr --remote <target>",
            "# 然后什么都不用做 —— PokePoke 会自动发现并接管通知",
          ],
        },
        {
          kind: "para",
          text: "PokePoke 复用 `herdr --remote` 已建立的 SSH 连接(control socket),在**已有连接**上加一条 unix socket 转发去读 herdr 的公开 API(`agent.list` / `events.subscribe`)。不新建 SSH 连接、不重复认证、不需要任何配置。",
        },
        {
          kind: "table",
          head: ["herdr 状态", "PokePoke 状态", "行为"],
          rows: [
            ["working", "Running", "不弹,静默进列表"],
            ["blocked", "Pending", "弹「需要你的注意」+ request 音"],
            ["done / idle", "Idle", "弹「干完了」+ done 音"],
            ["agent 消失(连续 3 轮)", "—", "清理 session"],
          ],
        },
        {
          kind: "note",
          text: "刚连上时就已经 done/idle 的 pane 不会补弹「干完了」——回溯状态不是状态迁移,弹了就是噪音。blocked 首观测会弹(刚连上就有 agent 等输入,正是远程通知的用途)。",
        },
        {
          kind: "para",
          text: "已知边界:如果你是 `ssh host` 之后再手动起 herdr(herdr 整个跑在远程、本机没有 `herdr --remote` 连接),PokePoke 没有可复用的通路,会自动跳过该机器——这种拓扑不在覆盖范围内。",
        },
      ],
    },
    {
      id: "terminals",
      index: "5",
      title: "终端宿主与跳转",
      blocks: [
        {
          kind: "list",
          items: [
            "**iTerm2 / Terminal.app** — 按 tty 精准匹配 tab,跨 space / window 自动 follow",
            "**Ghostty 1.3+** — SessionStart 时存 stable terminal id,同 cwd 多 tab 也能跳对;低于 1.3 退化为打开 cwd 文件夹",
            "**WezTerm** — 经 `wezterm cli activate-pane` 精准回跳 pane",
            "**Otty** — 经 `otty pane` CLI 精准回跳",
            "**Orca** — 经 `orca terminal switch` 聚焦对应 pane;支持 herdr 内嵌 Orca 的宿主发现",
            "**herdr / cmux** — 经 herdr wire 协议聚焦 pane",
            "**Cursor 内置终端** — 走 `cursor <workspace>` CLI 复用既有窗口",
          ],
        },
      ],
    },
    {
      id: "privacy",
      index: "6",
      title: "数据与隐私",
      blocks: [
        {
          kind: "list",
          items: [
            "全部数据本地存于 `~/.pokepoke/`:`sessions.json` / `settings.json` / `pokepoke.log`(设 `POKEPOKE_DEBUG=1` 启 DEBUG 日志)",
            "hook 写入前自动 snapshot 到 `~/.pokepoke/snapshots/`(每个来源保留最近 5 份)",
            "hook ↔ app 走 Unix Domain Socket `~/.pokepoke/sock`,不开网络端口,**也不与任何外部服务通信**",
          ],
        },
      ],
    },
    {
      id: "uninstall",
      index: "7",
      title: "卸载",
      blocks: [
        {
          kind: "code",
          lines: [
            "rm -rf /Applications/PokePoke.app",
            "rm -rf ~/.pokepoke            # 数据(可选)",
            "rm -f  ~/.local/bin/poke-hook # hook binary",
          ],
        },
        {
          kind: "para",
          text: "各 agent 的 hook 配置建议先用菜单栏 Disconnect 清理;或手动编辑对应配置文件,删除含 `poke-hook` 的条目(pi 为 `rm -rf ~/.pi/agent/extensions/pokepoke`)。",
        },
      ],
    },
  ],
};
