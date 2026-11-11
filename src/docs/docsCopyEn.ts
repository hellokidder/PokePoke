import type { DocsCopy } from "./docsCopy";

export const docsCopyEn: DocsCopy = {
  header: {
    manual: "POKEPOKE(1) — User Manual",
    tagline: "Your agent pokes you; you jump back in one click. This manual covers how.",
    backHome: "Home",
    download: "Download",
  },
  sections: [
    {
      id: "quickstart",
      index: "1",
      title: "Quickstart",
      blocks: [
        {
          kind: "para",
          text: "Requirements: macOS 13 (Ventura) or later, Apple Silicon (arm64). Intel Macs are not officially packaged — build from source if you need one.",
        },
        {
          kind: "list",
          items: [
            "Download the latest `PokePoke_<version>_aarch64.dmg` from Releases and drag it into `/Applications`",
            "If macOS says the developer can't be verified on first launch: `System Settings → Privacy & Security`, allow it",
            "The octopus icon in your menu bar means you're ready — first launch auto-installs hooks for every detected agent, no manual setup",
          ],
        },
        {
          kind: "note",
          text: "The first time you click a popup to jump back, macOS asks once to let PokePoke control that terminal — allow it. macOS asks only once per terminal host.",
        },
      ],
    },
    {
      id: "agents",
      index: "2",
      title: "Connect your agents",
      blocks: [
        {
          kind: "para",
          text: "Every integration is one click: menu bar icon → the agent's **Connect**. Disconnect / Repair live in the same menu. Everything is written to user-level config — your project directories are never touched.",
        },
        {
          kind: "table",
          head: ["Agent", "Writes to", "Events"],
          rows: [
            [
              "Claude Code",
              "`~/.claude/settings.json`",
              "SessionStart / UserPromptSubmit / Notification / Stop / StopFailure",
            ],
            [
              "Cursor",
              "`~/.cursor/hooks.json` (global)",
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
            ["Grok Build", "`~/.grok` (CC-compatible hooks)", "Same event names as Claude Code"],
          ],
        },
        {
          kind: "note",
          text: "Codex CLI ≥0.131 needs a one-time `/hooks` trust of `poke-hook` in its TUI, or Codex silently never fires. Cursor's Third-party hooks option double-fires events (a 100ms in-memory dedupe covers you, but pick one). pi reports in TUI mode only and has no pending state — by design, not a gap.",
        },
        {
          kind: "para",
          text: 'If a future update changes the hook protocol, the panel shows a "hook config outdated" banner with a one-click repair.',
        },
      ],
    },
    {
      id: "daily",
      index: "3",
      title: "Daily use",
      blocks: [
        {
          kind: "list",
          items: [
            "**Panel**: click the menu bar icon — one row per live session, four states: Running / Pending (waiting on you) / Idle (finished) / Failed",
            "**Popups**: appear in a screen corner with a sound on state change; they stay until you click — they never auto-dismiss and vanish on you",
            "**Jump back**: click the popup or press `⌘N` to focus the right terminal / IDE window (modifier keys configurable in Settings)",
            "**Global toggle**: bottom-left of the panel — mute everything; states keep updating silently",
            "**Settings**: alert sound (any system sound / mute), language, menu bar icon (color / fixed white), popup corner",
          ],
        },
      ],
    },
    {
      id: "herdr-remote",
      index: "4",
      title: "herdr remote mode",
      blocks: [
        {
          kind: "para",
          text: "The scenario: PokePoke runs on the machine in front of you while agents run inside herdr on a **remote machine** - with **zero install** on the remote side. State changes pop up as PokePoke popups on your screen, not as herdr toasts.",
        },
        { kind: "para", text: "The correct usage is exactly one step:" },
        {
          kind: "code",
          lines: [
            "# Connect to the remote machine with herdr, as usual",
            "herdr --remote <target>",
            "# That's it — PokePoke discovers the connection and takes over notifications",
          ],
        },
        {
          kind: "para",
          text: "PokePoke reuses the SSH connection `herdr --remote` already established (its control socket) and adds a unix-socket forward on top of that **existing connection** to read herdr's public API (`agent.list` / `events.subscribe`). No new SSH connection, no re-authentication, no configuration.",
        },
        {
          kind: "table",
          head: ["herdr status", "PokePoke status", "Behavior"],
          rows: [
            ["working", "Running", "No popup; tracked silently"],
            ["blocked", "Pending", 'Popup "needs your input" + request sound'],
            ["done / idle", "Idle", 'Popup "finished" + done sound'],
            ["agent gone (3 polls)", "—", "Session cleaned up"],
          ],
        },
        {
          kind: "note",
          text: "Panes already done/idle when you connect don't retroactively pop \"finished\" — a backfilled state isn't a transition, and popping would be noise. A blocked pane on first observation does pop (something is already waiting for you — exactly what remote notifications are for).",
        },
        {
          kind: "para",
          text: "Known boundary: if you `ssh host` first and then start herdr inside the remote shell (herdr lives entirely on the remote, no local `herdr --remote` connection), PokePoke has no channel to reuse and silently skips that machine — this topology is out of scope.",
        },
      ],
    },
    {
      id: "terminals",
      index: "5",
      title: "Terminal hosts & jumping",
      blocks: [
        {
          kind: "list",
          items: [
            "**iTerm2 / Terminal.app** — precise tty-based tab matching, follows across spaces / windows",
            "**Ghostty 1.3+** — stores a stable terminal id at SessionStart; jumps to the right tab even with several tabs on the same cwd; older versions fall back to opening the cwd folder",
            "**WezTerm** — precise pane jump via `wezterm cli activate-pane`",
            "**Otty** — precise pane jump via the `otty pane` CLI",
            "**Orca** — focuses the right pane via `orca terminal switch`; Orca embedded in herdr is discovered too",
            "**herdr / cmux** — pane focus over the herdr wire protocol",
            "**Cursor integrated terminal** — reuses the existing window via the `cursor <workspace>` CLI",
          ],
        },
      ],
    },
    {
      id: "privacy",
      index: "6",
      title: "Data & privacy",
      blocks: [
        {
          kind: "list",
          items: [
            "Everything lives locally in `~/.pokepoke/`: `sessions.json` / `settings.json` / `pokepoke.log` (set `POKEPOKE_DEBUG=1` for DEBUG logs)",
            "Hook configs are snapshotted before every write to `~/.pokepoke/snapshots/` (last 5 per source)",
            "Hooks talk to the app over a Unix domain socket at `~/.pokepoke/sock` — no network port, and no outbound calls at all",
          ],
        },
      ],
    },
    {
      id: "uninstall",
      index: "7",
      title: "Uninstall",
      blocks: [
        {
          kind: "code",
          lines: [
            "rm -rf /Applications/PokePoke.app",
            "rm -rf ~/.pokepoke            # data (optional)",
            "rm -f  ~/.local/bin/poke-hook # hook binary",
          ],
        },
        {
          kind: "para",
          text: "Clean up agent hook configs via each agent's Disconnect in the menu bar first; or edit the config files by hand and remove entries containing `poke-hook` (for pi: `rm -rf ~/.pi/agent/extensions/pokepoke`).",
        },
      ],
    },
  ],
};
