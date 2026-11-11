import type { Locale } from "../i18n/strings";
import type { SessionStatus } from "../types";

type IntegrationSlug =
  | "claude-code"
  | "cursor"
  | "codex"
  | "opencode"
  | "pi"
  | "omp"
  | "kimi"
  | "grok"
  | "dsh";

type IntegrationCopy = {
  name: string;
  slug: IntegrationSlug;
  tone: string;
  events: string;
};

type TerminalCategoryCopy = {
  label: string;
  items: string[];
};

type LoopCopy = {
  title: string;
  body: string;
};

type HeroDemoSessionCopy = {
  project: string;
  workspace: string;
  source: "claude-code" | "cursor" | "codex" | "opencode";
  sourcePill: string;
  status: SessionStatus;
  message: string;
  time: string;
  colorSeed: string;
};

type HeroDemoCopy = {
  sessions: HeroDemoSessionCopy[];
  footerCount: string;
  notificationToggle: string;
  /** northwind 在 demo 循环 phase0(平静态)显示的运行中消息 */
  runningMessage: string;
  popup: {
    title: string;
    body: string;
    source: "claude-code" | "cursor" | "codex" | "opencode";
    sourcePill: string;
    status: SessionStatus;
    time: string;
    jump: string;
    badge: string;
    colorSeed: string;
  };
};

/** 页面级"赖皮 popup":滚过 hero 后驻留角落,点了跳回顶部(网页版 ⌘N) */
type StickyPokeCopy = {
  title: string;
  body: string;
  sourcePill: string;
  status: SessionStatus;
  time: string;
  jump: string;
  badge: string;
  colorSeed: string;
  /** popup 下方的小字注释,点明"不点不走"的隐喻 */
  note: string;
  closeLabel: string;
};

type LandingCopy = {
  language: {
    current: string;
    next: string;
    label: string;
  };
  nav: {
    features: string;
    integrations: string;
    compare: string;
    privacy: string;
    docs: string;
    download: string;
  };
  hero: {
    eyebrow: string;
    title: string;
    body: string;
    primary: string;
    shortcut: string;
    demo: HeroDemoCopy;
  };
  stickyPoke: StickyPokeCopy;
  problem: {
    eyebrow: string;
    title: string;
    body: string;
  };
  features: {
    eyebrow: string;
    title: string;
    body: string;
    loop: LoopCopy[];
  };
  integrations: {
    eyebrow: string;
    title: string;
    body: string;
    badge: string;
    moreLabel: string;
    items: IntegrationCopy[];
  };
  terminalJump: {
    eyebrow: string;
    title: string;
    body: string;
    shortcutLabel: string;
    worksWithLabel: string;
    categories: TerminalCategoryCopy[];
    /** @deprecated flat list kept for tests — derived from categories */
    terminals: string[];
  };
  compare: {
    eyebrow: string;
    title: string;
    body: string;
    points: string[];
    footer: string;
  };
  privacy: {
    eyebrow: string;
    title: string;
    bodyStart: string;
    bodyEnd: string;
    points: string[];
  };
  opensource: {
    eyebrow: string;
    title: string;
    body: string;
  };
  footer: {
    tagline: string;
    download: string;
    docs: string;
    github: string;
    x: string;
    privacy: string;
    contact: string;
  };
};

const en: LandingCopy = {
  language: {
    current: "EN",
    next: "中文",
    label: "Switch language",
  },
  nav: {
    features: "Features",
    integrations: "Integrations",
    compare: "Compare",
    privacy: "Privacy",
    docs: "Docs",
    download: "Download",
  },
  hero: {
    eyebrow: "macOS menu-bar attention tool",
    title: "Never miss the moment your coding agent needs you.",
    body: "PokePoke watches Claude Code, Cursor, Codex (CLI or ChatGPT), OpenCode, pi, omp, Kimi Code, Grok Build, and DeepSeek Harness from your menu bar, then nudges you when a session is waiting, done, or stuck.",
    primary: "Download for macOS",
    shortcut: "Then press ⌘N to jump right back to any session.",
    demo: {
      sessions: [
        {
          project: "atlas",
          workspace: "~/Projects/atlas",
          source: "claude-code",
          sourcePill: "CLAUDE-CODE",
          status: "running",
          message: "Refactoring the notification panel",
          time: "2m ago",
          colorSeed: "octo-violet",
        },
        {
          project: "northwind",
          workspace: "~/Projects/northwind",
          source: "cursor",
          sourcePill: "CURSOR",
          status: "pending",
          message: "Waiting for approval",
          time: "just now",
          colorSeed: "octo-mint",
        },
        {
          project: "orbit",
          workspace: "~/Projects/orbit",
          source: "codex",
          sourcePill: "CHATGPT",
          status: "idle",
          message: "Session completed",
          time: "just now",
          colorSeed: "octo-sky",
        },
        {
          project: "release-kit",
          workspace: "~/Projects/release-kit",
          source: "opencode",
          sourcePill: "OPENCODE",
          status: "last_failed",
          message: "Last turn failed",
          time: "6m ago",
          colorSeed: "octo-lime",
        },
      ],
      footerCount: "4 sessions",
      notificationToggle: "Notifications",
      runningMessage: "Editing src/auth.ts",
      popup: {
        title: "northwind",
        body: "Waiting for approval",
        source: "cursor",
        sourcePill: "CURSOR",
        status: "pending",
        time: "just now",
        jump: "Click or press",
        badge: "⌘N",
        colorSeed: "octo-mint",
      },
    },
  },
  stickyPoke: {
    title: "northwind",
    body: "Waiting for approval",
    sourcePill: "CURSOR",
    status: "pending",
    time: "just now",
    jump: "Click to jump back",
    badge: "⌘N",
    colorSeed: "octo-mint",
    note: "This popup doesn't dismiss itself — like the real thing.",
    closeLabel: "Dismiss",
  },
  problem: {
    eyebrow: "Stop babysitting terminals",
    title: "Agents run ahead. They still need a way to tap your shoulder.",
    body: "Start an agent, switch to another task, and come back later to find it was waiting the whole time. PokePoke turns those stalled moments into visible, audible attention signals.",
  },
  features: {
    eyebrow: "Attention loop",
    title: "From hook event to jump back.",
    body: "Connect an agent, track its state, get nudged hard enough, and return to the exact window.",
    loop: [
      {
        title: "Hook event",
        body: "PokePoke listens when your agent starts, waits, stops, or fails.",
      },
      {
        title: "Session state",
        body: "Running, pending, idle, and failed sessions stay visible in the menu bar.",
      },
      { title: "Nudge", body: "Corner popups and alert sounds bring the moment back to you." },
      {
        title: "Jump back",
        body: "Click the alert or press ⌘N to return to the right terminal pane, herdr/cmux workspace, IDE, or ChatGPT thread.",
      },
    ],
  },
  integrations: {
    eyebrow: "Integrations",
    title: "Connect the agents you already use.",
    body: "PokePoke writes user-level hook config and keeps the attention stream in one menu-bar panel.",
    badge: "hook ready",
    moreLabel: "More agents on the roadmap",
    items: [
      {
        name: "Claude Code",
        slug: "claude-code",
        tone: "Full attention signals",
        events: "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
      },
      {
        name: "Cursor",
        slug: "cursor",
        tone: "cursor-agent in your terminal",
        events: "sessionStart / beforeSubmitPrompt / stop / sessionEnd",
      },
      {
        name: "Codex",
        slug: "codex",
        tone: "CLI and ChatGPT desktop sessions",
        events: "SessionStart / UserPromptSubmit / Stop",
      },
      {
        name: "OpenCode",
        slug: "opencode",
        tone: "Plugin-based session tracking",
        events: "session.idle / permission.ask / event",
      },
      {
        name: "pi",
        slug: "pi",
        tone: "Interactive TUI sessions via extension",
        events: "session_start / before_agent_start / agent_settled / session_shutdown",
      },
      {
        name: "omp",
        slug: "omp",
        tone: "Pi fork: tool-approval aware sessions",
        events: "session_start / before_agent_start / agent_end / tool_approval_requested",
      },
      {
        name: "kimi",
        slug: "kimi",
        tone: "Kimi Code CLI via TOML hooks",
        events: "SessionStart / UserPromptSubmit / Stop / PermissionRequest",
      },
      {
        name: "Grok Build",
        slug: "grok",
        tone: "CC-compatible hooks via ~/.grok",
        events: "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
      },
      {
        name: "DeepSeek Harness",
        slug: "dsh",
        tone: "Desktop & web sessions via cordis plugin",
        events: "turn/start / turn/end / approval asked / user question",
      },
    ],
  },
  privacy: {
    eyebrow: "Local-first",
    title: "Your data never leaves your Mac.",
    bodyStart: "Everything PokePoke tracks lives on your machine under",
    bodyEnd:
      ". Hooks talk to the app over a local Unix socket — no account, no cloud sync, and zero user data ever uploaded.",
    points: [
      "All session data stays on-device. Nothing is sent to our servers.",
      "No prompts, no logs, no code, no terminal output leaves your Mac.",
      "No account, no sign-in, no outbound calls. Nothing leaves your Mac.",
    ],
  },
  terminalJump: {
    eyebrow: "Terminal Jump",
    title: "⌘N to jump. Not ⌘Tab to hunt.",
    body: "PokePoke remembers exactly where each session lives — terminal tab, herdr or cmux pane, IDE window, or ChatGPT thread. Press ⌘N from anywhere and land on the precise surface.",
    shortcutLabel:
      "Press ⌘N from anywhere to return to any active session — terminal, herdr/cmux pane, or ChatGPT.",
    worksWithLabel: "Jump surfaces",
    categories: [
      {
        label: "Terminals",
        items: [
          "iTerm2",
          "Terminal.app",
          "Ghostty 1.3+",
          "WezTerm",
          "Otty",
          "Orca",
          "herdr",
          "cmux",
        ],
      },
    ],
    terminals: [
      "iTerm2",
      "Terminal.app",
      "Ghostty 1.3+",
      "WezTerm",
      "Otty",
      "Orca",
      "herdr",
      "cmux",
    ],
  },
  compare: {
    eyebrow: "Why PokePoke",
    title: "Not elegant. Just won't let you miss it.",
    body: "Vibe Island and Open Island deliver agent alerts through the Mac notch — polished, native, and gone the moment it dismisses. PokePoke takes a different bet: when an agent needs you, the popup stays on screen until you act. Less elegant, harder to ignore. For developers running 3–5 agents in parallel, that difference matters.",
    points: [
      "Popup stays until you click it — switch spaces, grab coffee, come back, it's still there.",
      "Island alerts auto-dismiss. If you miss the sound or weren't looking, the moment is gone.",
      "System alert sound + corner popup = active interruption. Designed for when you've walked away from the terminal.",
    ],
    footer:
      "Vibe Island and Open Island are excellent tools. They focus on control — approving permissions, answering agent questions inline. PokePoke focuses on attention — making sure you know something needs you in the first place.",
  },
  opensource: {
    eyebrow: "Open source",
    title: "MIT licensed, source on GitHub.",
    body: "The full source ships in the public repo — every integration, every jump path. There is no paid tier and no feature gate: every notification, every terminal jump, every day.",
  },
  footer: {
    tagline: "© PokePoke. Built for agent-heavy workflows.",
    download: "Download",
    docs: "Docs",
    github: "GitHub",
    x: "Follow on X",
    privacy: "Privacy Policy",
    contact: "hellokiddermail@gmail.com",
  },
};

const zh: LandingCopy = {
  language: {
    current: "中文",
    next: "EN",
    label: "切换语言",
  },
  nav: {
    features: "功能",
    integrations: "集成",
    compare: "对比",
    privacy: "隐私",
    docs: "文档",
    download: "下载",
  },
  hero: {
    eyebrow: "macOS 菜单栏 agent 强提醒工具",
    title: "别再错过 coding agent 需要你的时刻。",
    body: "PokePoke 在菜单栏里监听 Claude Code、Cursor、Codex（CLI 或 ChatGPT 桌面端）、OpenCode、pi、omp、Kimi Code、Grok Build 和 DeepSeek Harness 的会话；当 agent 等你确认、已经完成或卡住时，用提示窗和声音把你叫回来。",
    primary: "下载 macOS 版",
    shortcut: "安装后按 ⌘N 一键跳回任意会话。",
    demo: {
      sessions: [
        {
          project: "atlas",
          workspace: "~/Projects/atlas",
          source: "claude-code",
          sourcePill: "CLAUDE-CODE",
          status: "running",
          message: "正在重构提示面板",
          time: "2 分钟前",
          colorSeed: "octo-violet",
        },
        {
          project: "northwind",
          workspace: "~/Projects/northwind",
          source: "cursor",
          sourcePill: "CURSOR",
          status: "pending",
          message: "等待你批准",
          time: "刚刚",
          colorSeed: "octo-mint",
        },
        {
          project: "orbit",
          workspace: "~/Projects/orbit",
          source: "codex",
          sourcePill: "CHATGPT",
          status: "idle",
          message: "Session completed",
          time: "刚刚",
          colorSeed: "octo-sky",
        },
        {
          project: "release-kit",
          workspace: "~/Projects/release-kit",
          source: "opencode",
          sourcePill: "OPENCODE",
          status: "last_failed",
          message: "上一轮出错",
          time: "6 分钟前",
          colorSeed: "octo-lime",
        },
      ],
      footerCount: "4 个会话",
      notificationToggle: "全局提示",
      runningMessage: "正在修改 src/auth.ts",
      popup: {
        title: "northwind",
        body: "等待你批准",
        source: "cursor",
        sourcePill: "CURSOR",
        status: "pending",
        time: "刚刚",
        jump: "点击或按",
        badge: "⌘N",
        colorSeed: "octo-mint",
      },
    },
  },
  stickyPoke: {
    title: "northwind",
    body: "等待你批准",
    sourcePill: "CURSOR",
    status: "pending",
    time: "刚刚",
    jump: "点击跳回顶部",
    badge: "⌘N",
    colorSeed: "octo-mint",
    note: "这个弹窗不点不走——就像真的一样。",
    closeLabel: "关闭",
  },
  problem: {
    eyebrow: "不用再盯着终端",
    title: "Agent 可以自己往前跑，但它仍然需要一种方式拍你一下。",
    body: "你启动 agent 后切去做别的事，二十分钟后才发现它一直在等确认。PokePoke 把这些停住的时刻变成看得见、听得到的提醒。",
  },
  features: {
    eyebrow: "注意力闭环",
    title: "从 hook 事件到一键跳回。",
    body: "连接 agent、追踪会话状态、足够强地提醒、回到准确的窗口。",
    loop: [
      { title: "Hook 事件", body: "当 agent 开始、等待、停止或失败时，PokePoke 会监听到。" },
      { title: "会话状态", body: "运行中、等待中、空闲、上一轮失败的会话都会留在菜单栏里。" },
      { title: "强提醒", body: "角落提示窗和提示音会把注意力拉回来。" },
      {
        title: "跳回现场",
        body: "点击提示窗或按 ⌘N，回到对应的终端 pane、herdr/cmux 工作区、IDE 或 ChatGPT 线程。",
      },
    ],
  },
  integrations: {
    eyebrow: "集成",
    title: "连接你已经在用的 coding agent。",
    body: "PokePoke 写入用户级 hook 配置，并把注意力信号集中到一个菜单栏面板里。",
    badge: "hook 就绪",
    moreLabel: "更多 agent 持续接入中",
    items: [
      {
        name: "Claude Code",
        slug: "claude-code",
        tone: "完整的注意力信号",
        events: "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
      },
      {
        name: "Cursor",
        slug: "cursor",
        tone: "命令行里的 cursor-agent 活动",
        events: "sessionStart / beforeSubmitPrompt / stop / sessionEnd",
      },
      {
        name: "Codex",
        slug: "codex",
        tone: "CLI 与 ChatGPT 桌面端会话",
        events: "SessionStart / UserPromptSubmit / Stop",
      },
      {
        name: "OpenCode",
        slug: "opencode",
        tone: "基于 plugin 的会话追踪",
        events: "session.idle / permission.ask / event",
      },
      {
        name: "pi",
        slug: "pi",
        tone: "通过扩展追踪交互式 TUI 会话",
        events: "session_start / before_agent_start / agent_settled / session_shutdown",
      },
      {
        name: "omp",
        slug: "omp",
        tone: "pi 的分支：能识别工具审批",
        events: "session_start / before_agent_start / agent_end / tool_approval_requested",
      },
      {
        name: "kimi",
        slug: "kimi",
        tone: "通过 TOML hooks 接入 Kimi Code CLI",
        events: "SessionStart / UserPromptSubmit / Stop / PermissionRequest",
      },
      {
        name: "Grok Build",
        slug: "grok",
        tone: "通过 CC 兼容 hooks 接入 Grok Build",
        events: "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
      },
      {
        name: "DeepSeek Harness",
        slug: "dsh",
        tone: "通过 cordis 插件接入桌面端与 Web 会话",
        events: "turn/start / turn/end / approval 待审批 / 用户提问",
      },
    ],
  },
  privacy: {
    eyebrow: "本地优先",
    title: "你的数据永远不离开这台 Mac。",
    bodyStart: "PokePoke 追踪的一切都只存在你本机的",
    bodyEnd:
      "目录下。Hook 通过本地 Unix socket 与应用通信——无需账号、不做云同步，绝不上传任何用户数据。",
    points: [
      "所有会话数据都留在本地设备，绝不发送到我们的服务器。",
      "不上传 prompt、日志、代码或终端输出——一个字节都不传。",
      "无需注册、无需登录、没有任何对外请求。数据不离开你的 Mac。",
    ],
  },
  terminalJump: {
    eyebrow: "终端跳转",
    title: "按 ⌘N 跳回，不用 ⌘Tab 去翻。",
    body: "PokePoke 记住每个会话的精确位置——终端 tab、herdr 或 cmux pane、IDE 窗口或 ChatGPT 线程。按 ⌘N 就能从任意窗口跳回对应界面。",
    shortcutLabel: "在任何界面按 ⌘N，一键回到对应会话的终端、herdr/cmux pane 或 ChatGPT。",
    worksWithLabel: "跳转目标",
    categories: [
      {
        label: "终端",
        items: [
          "iTerm2",
          "Terminal.app",
          "Ghostty 1.3+",
          "WezTerm",
          "Otty",
          "Orca",
          "herdr",
          "cmux",
        ],
      },
    ],
    terminals: [
      "iTerm2",
      "Terminal.app",
      "Ghostty 1.3+",
      "WezTerm",
      "Otty",
      "Orca",
      "herdr",
      "cmux",
    ],
  },
  compare: {
    eyebrow: "为什么选 PokePoke",
    title: "不够优雅，但不会让你错过。",
    body: "Vibe Island 和 Open Island 通过 Mac notch 传递 agent 提醒——原生、精致、展示完自动收回。PokePoke 做了不同的选择：agent 需要你的时候，弹窗就挂在屏幕上不走。不那么优雅，但更难忽略。对同时跑 3–5 个 agent 的开发者来说，这个差异很关键。",
    points: [
      "弹窗不点不消失——切桌面、去倒水、回来还在那。",
      "Island 提醒展示后自动回缩。如果你没听到声音或刚好没在看，就错过了。",
      "系统提示音 + 角落弹窗 = 主动打断。为那些你离开终端去做别的事的场景而设计。",
    ],
    footer:
      "Vibe Island 和 Open Island 是很棒的工具。它们侧重控制——内联审批权限、回答 agent 的问题。PokePoke 侧重注意力——先确保你知道有事情需要你。",
  },
  opensource: {
    eyebrow: "开源",
    title: "MIT 授权，源码在 GitHub。",
    body: "全部源码都在公开仓里——每一个接入端、每一条回跳路径。没有付费档、没有功能门禁：提醒不限量，回跳不限量，永久免费。",
  },
  footer: {
    tagline: "© PokePoke. 为 agent-heavy 工作流而生。",
    download: "下载",
    docs: "文档",
    github: "GitHub",
    x: "在 X 上关注",
    privacy: "隐私政策",
    contact: "hellokiddermail@gmail.com",
  },
};

export function getLandingCopy(locale: Locale): LandingCopy {
  return locale === "en" ? en : zh;
}
