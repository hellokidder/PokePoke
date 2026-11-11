# Changelog

> [中文](./CHANGELOG.md) | English

This file records **user-visible changes** in Poke Poke. It is published with each Release (and shown in the in-app update prompt), following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Logging principles

**What to record**: only behavior changes users can perceive (everything else lives in git log):

- New / removed terminal, agent, or feature support
- Popup trigger / dismiss / suppression behavior changes
- Settings additions, removals, default-value adjustments
- User-visible bug fixes

**How to write**: describe, from the user's perspective, what you can now do / what was fixed; one line per entry, concise.

**Do not write**:

- Implementation mechanics and internals: hook events, environment variables, config file paths, CLI commands, priority matrices, schema field names
- Dev/ops-facing content: diagnostics exports, internal commands, tests and refactors
- Brief / PR / commit references (those live in git history and `docs/briefs/`)

## Categories

- `Added`: new behavior
- `Changed`: changes to existing behavior
- `Deprecated`: scheduled for removal
- `Removed`: removed
- `Fixed`: fixes (with behavioral correction)
- `Security`: security-related

---

## [1.1.7] - 2026-11-11

### Added

- Keyboard control in the quiet mode panel: the duration is focused on open, arrow keys
  switch the duration, Enter starts immediately, Esc closes

### Changed

- The tray context menu no longer lists per-integration status and connect toggles;
  integrations are managed in Settings → Integrations (which covers all 9 of them)

### Fixed

- Fixed the update prompt's "Click to upgrade now, or skip this version for later" text
  being clipped at the bottom
- Fixed the "Send feedback" link pointing at an internal repository, which failed to open

---

## [1.1.6] - 2026-11-11

### Added

- **DeepSeek Harness (DSH) support**: session status alerts, approval & question prompts,
  and one click straight back to the exact conversation
- **Open source**: the source is public under the MIT license — use it, modify it, redistribute it

### Changed

- **Every feature is free**: the daily reminder cap and the 3-day trial are gone; no license
  is needed for unlimited reminders
- Smarter DSH "done" detection: no more premature completion alerts while sub-sessions or
  follow-up turns are still running

### Removed

- The Settings "License" tab, the quota banner in the panel, and the "Upgrade to Pro"
  entry points are all gone; they disappear automatically on upgrade

---

## [1.1.5] - 2026-10-31

### Added

- Support for the **oh-my-pi (omp)** agent: status alerts, completion popups, approval-request
  alerts, and click-to-jump-back to its terminal

### Changed

- More accurate Ghostty jump-back: when several Ghostty tabs or splits share the same directory,
  clicking an alert now returns to the exact tab or split that raised it (Ghostty 1.4 and later)
- Alerts appear sooner: after an agent finishes a turn, alerting no longer waits for the app to
  locate / focus the target window, cutting about 200ms of perceived delay (trigger and
  suppression behavior is unchanged)
- Shorter alert-sound warm-up: the lead-in before the first sound after a device idles is now about
  100ms, down from about 300ms

### Removed

- The "Ghostty focus accuracy" setting is gone. It rewrote terminal titles to improve jump-back
  precision, which is no longer needed; the toggle simply disappears after upgrading, with nothing
  to do manually

### Fixed

- Fixed Claude Code permission prompts not alerting right away: the alert now fires as soon as the
  prompt appears (previously only after about 6s, and not at all if you confirmed sooner)
- Fixed Cursor CLI session alerts flashing once and vanishing, with no record in the session panel
  (the session was treated as ended about 10s in while the IDE was closed)
- Fixed click-to-jump-back not working for sessions running inside cmux, especially when the app
  was launched from the Dock
- Fixed a wasted probe on every alert when Orca isn't running (about 165ms added per alert)
- Fixed pi playing the "task completed" sound after you manually cancel a turn (Ctrl+C)
- Fixed alert sounds occasionally starting mid-way (the leading part clipped on the first sound
  after a device idles)
- Fixed a permission request occasionally ringing twice while the terminal is focused (agent
  re-sending pending notifications)
- Fixed the free-reminder counter in the session panel not following actual usage (it now updates
  live instead of only after a restart)

## [1.1.4] - 2026-08-19

### Added

- Support for the **Warp** terminal: click a session to jump back to its window
- Support for **tmux** sessions: status alerts and click-to-jump-back
- Support for **herdr remote sessions**: connect to a remote machine's herdr over an
  SSH-forwarded socket and show the full popup locally (zero install on the remote)

### Fixed

- Fix an occasional swallowed completion sound: when an agent completes shortly after a
  "waiting for you" reminder, the completion sound now plays as expected
  (previously only the waiting sound could be heard)
- Fixed a spurious completion sound / repeated alert when a Cursor task is cancelled with Ctrl+C
- Fixed the herdr triple "done" popup race in a single conversation
- Fixed herdr sessions hosted in Warp / cmux not jumping back (or landing on the wrong tab) on click
- Fixed herdr remote sessions occasionally being undiscoverable and never notifying
- Fixed herdr sessions replaying a "all done" popup after a restart


## [1.1.3] - 2026-08-03

### Added

- Support for **Otty** and **Orca** terminals: click a session to jump back to its window; no popup when you're already focused there
- Support for **Kimi Code** and **Grok Build**
- Custom alert sounds: import your own audio files

### Changed

- Alert sounds upgraded: bundled sound effects included, and you can now set a different sound for each status (running / idle / failed)

### Fixed

- Fixed Kimi session popup misfires and status mix-ups when running multiple sessions at once
- Fixed custom alert sounds occasionally going missing

## [1.1.2] - 2026-07-27

### Added

- Support for the **pi** agent: session status alerts, completion popups, and click-to-jump-back
- Support for the **WezTerm** terminal: click a session to jump back to its window; no popup when you're already focused there

## [1.1.1] - 2026-07-22

### Added

- **Quiet mode**: new timed alert-mute feature; popups still appear during the countdown but no alert sound is played; global shortcut defaults to `⌘;` and can be re-bound in Settings → General (independent from the popup hotkey prefix).
- **Explicit disconnect lifecycle for integrations**: CC / Codex / Cursor / OpenCode can be disconnected from both the tray menu and Settings; disconnected sources have `/notify` rejected and are never silently reinstalled on first-run.
- **Minimum-modifier guard for shortcuts**: both popup prefix and quiet-mode shortcut reject pure Shift/Alt combos (must include Cmd or Control) to avoid accidental triggers during normal typing.

### Changed

- **First-run hook install now version-checks**: skips overwriting the deployed hook when the bundled version is older or equal (prevents dev-build downgrading a release hook).
- **Focus suppression extended** to cmux / herdr / Cursor; herdr pane suppression additionally requires the outer host app to be frontmost.

### Fixed

- **No phantom first-run install when no agent is present**: skips install + auto-connect when zero agents are detected.

## [1.1.0] - 2026-07-16

### Added

- **CC jump-back inside herdr / cmux panes**: when CC runs inside a herdr / cmux multiplexer pane, reminders now jump to the correct pane. Sessions running directly in Ghostty/iTerm2/Terminal are unaffected.
- **Codex desktop precise jump-back**: clicking a reminder now jumps back to the exact conversation via `codex://threads/<id>`.

### Changed

- **Codex auto-approval turns suppress the popup**: turns in auto-approval no longer fire the approval prompt; falls back to allowing when the signal can't be read.
- **Ghostty reconcile exponential backoff**: consecutive failures during periodic reconciliation now back off exponentially, avoiding busy-spin retries.
- **Hook auto-refresh after upgrade-restart**: converges stale debug hooks from older versions being overwritten.

### Fixed

- **Popup race protection**: when a popup for the same session is already on screen, a refresh-only update no longer rebuilds the window or replays the sound.
- **`focus_check` AppleScript syntax**: fixes parse failures on some system versions.

## [1.0.2] - 2026-07-08

### Changed
- **Clicking a reminder could jump to the wrong window when the same project has multiple tabs**: on cold start, if the first capture grabbed the wrong window id, every subsequent reminder click would jump to the wrong window until that window was closed. The tab title is now used as strong evidence to auto-correct the mis-binding.
- **Closed agent sessions lingered in the panel and the reminder wouldn't close**: after an agent exited, if its session had no terminal info, any same-named process on the machine was mistaken for "still alive", leaving dead sessions in the panel and the reminder popup stuck open. Sessions without terminal info are now treated as exited; they reappear when the next event arrives.
- **Reminders were wrongly dismissed after switching terminals**: a session previously used in Ghostty and later moved to iTerm2/Terminal could be misjudged as "user is watching" when a leftover Ghostty window happened to be frontmost, suppressing or closing its reminder. Focus is now judged by the actual current terminal type, no longer confused by stale ids.
- **Agent failure notifications were silently dropped**: in rare cases, a third-party wrapper forwarding "turn ended" then "API error" within 100ms caused the failure state to be deduped away as a duplicate, hiding the failure reminder. Dedup now distinguishes "normal end" from "error failure", so failure reminders are no longer swallowed.
- **Launching a second instance could corrupt the main instance**: previously, opening PokePoke again would have the new instance clear sessions and rewrite integration configs before exiting, disrupting the primary instance's state. The second instance now exits immediately without touching any shared state.
- **Upgrading users were misdetected as first-run and got disconnected integrations reconnected**: a user who never changed a setting had no settings file, so an upgrade was treated as first-run, auto-reinstalling all integrations (potentially reconnecting a tool you had manually disconnected). Once you've used PokePoke even once (the session log is persisted), it's no longer misdetected.
- **Sessions that just appeared on startup could be wrongly cleared**: the race window between startup cleanup and event intake was narrowed, so a real session reported right at startup is no longer deleted as historical residue.
- **Uninstalling an integration could delete your own hook entries**: previously removal worked per "group", so if you manually placed your own hook entry into PokePoke's group it was deleted along with ours. Now only PokePoke's own entries are removed; yours are kept.
- **Integration drift detection is stricter**: previously any command carrying the source arg was considered healthy, so stale-path binaries were misreported as fine. The command string must now match exactly; any path or arg deviation triggers a one-tap repair prompt.

### Added
- **opencode integration added**: Connect / Disconnect / Check opencode from the tray menu. opencode's session lifecycle events (start working, awaiting permission approval, turn ended, errored) are aggregated into the panel and trigger reminders just like Claude Code / Codex / Cursor. opencode is the only integration besides Claude Code that can trigger a failure reminder on error. First-run auto-connects; config drift prompts a repair.

## [1.0.1] - 2026-07-05

### Fixed
- **focus-watcher now respects `focus_suppression_enabled`**: when focus suppression is off, the watcher no longer auto-closes the popup in the background (previously the popup "flashed" and disappeared).
- **Stop with unfinished `background_tasks` no longer fires the "completed" popup**: while a sub-agent / background shell is still running, Stop is only a turn boundary; the session stays Running.
- **CC/Cursor/Codex install no longer panics on corrupted config**: non-object / non-table `settings.json` / `hooks.json` / `config.toml` degrades to an empty object instead of crashing.
- **Creem / `codex login status` HTTP calls now have timeouts**: no longer hang indefinitely on unreachable APIs (the latter ran synchronously on first-run's main thread — a hang meant the window never appeared).
- **Settings save failure rolls back**: `saveField` rolls back UI state when the backend write fails, instead of showing the new value without persisting it.

### Security
- **`settings.json` / `sessions.json` tightened to 0600**: previously 0644 due to umask, leaking Feishu credentials / workspace paths to other processes on the same machine. Matches the existing `license.json` 0600 pattern.

### Changed
- **Settings window no longer switches activation policy**: stays Accessory throughout, fixing the side effect where a popup popping in would raise the settings window on top. Trade-off: settings doesn't appear in Dock/Cmd+Tab (entry is via the tray). Window size 560×600 → 620×720.

## [1.0.0] - 2026-07-01

First stable release. Commerce/payments now run on Creem production.

### Added

- **First-run locale auto-detection**: on a fresh install, `default_locale()` uses `sys-locale` to detect the system language (`zh*` → Chinese, everything else → English, Chinese as fallback when unavailable); the frontend's first-frame value now prefers localStorage and otherwise detects from `navigator.language`, eliminating the Chinese first-frame flash on English systems. Only affects fresh installs / new visitors; existing `settings.json` / localStorage behavior is unchanged.

### Changed

- **License payments switched to Creem production**: `POKEPOKE_CREEM_API_KEY` now uses a production `creem_xxx` key (no `_test_` prefix → hits the `api.creem.io` production endpoint); product_id mapping uses production products.
- **Purchase entry split into monthly/lifetime links**: the Settings "Upgrade to Pro" single button becomes two buttons ("Monthly / Lifetime"); `open_purchase_url` takes a `kind` (monthly/lifetime) and derives `https://www.creem.io/payment/{product_id}` from `POKEPOKE_CREEM_*_PRODUCT_ID`; the landing page's two checkout links point to production products.

### Removed

- **Removed the `POKEPOKE_CREEM_PURCHASE_URL` environment variable**: purchase links are now derived from product_id, so this separate setting is no longer needed.

### Fixed

- **Codex install no longer overwrites existing user hooks**: `--install-codex` (including first-run auto-connect) previously rewrote `~/.codex/hooks.json` wholesale with a fresh config, silently wiping any hooks the user had authored in Codex; it now reads the existing config and merges per event (removes stale PokePoke entries, then appends — idempotent), preserving the user's custom hooks in other events / the same event, consistent with the Claude Code / Cursor merge semantics.
- **Ghostty jump-back no longer mutates the other window's selected tab**: with focus on window B, jumping back to a conversation in window A would also change B's selected tab. Root cause: the L1/L2 scripts in `focus_ghostty` stored `targetTab`/`targetTerminal` while iterating `repeat with aWindow in windows` — these are lazily-resolved references indexed by the current z-order; the subsequent `activate window targetWindow` brought the target to the front and flipped the windows order, so the later `select tab targetTab` reused the stale reference and re-resolved it against the new z-order, landing on the same-index tab of the now-covered window (reproducible: target in A.tab1 with focus on B changed B's selected tab to tab1; if B had fewer tabs it errored "invalid index"). Fix: only store `targetWindow` during iteration; after `activate window` flips z-order, re-resolve the tab/terminal from `tabs of front window` by id/cwd before `select tab`+`focus` — never reuse a reference across the flip.

## [0.0.9] - 2026-06-30

### Added

- **3-day Pro trial on first launch**: the first launch grants a 3-day Pro trial automatically; once it expires the app falls back to the free-tier daily notification quota.
- Show a neutral pending reminder when a Claude Code transcript contains an unrecovered interruption marker. A lightweight transcript watcher scans only `running` Claude Code sessions, detects `[Request interrupted by user]` at the tail, and feeds a synthetic `pending` event through the existing `/notify` path to trigger popup/sound.
- **Periodic Ghostty terminal_id reconciliation** (brief `20260629-ghostty-jump-reconcile`): a background watcher (8s tick) enumerates every Ghostty terminal's `(id, working directory, title)` and binds claude-code/codex sessions to a stable terminal UUID by **title containing the `external_session_id` prefix** (primary key) / cwd (fallback). This covers the cold-start gap that a single capture misses; a title-key hit can **correct** a mis-binding caused by cwd first-match (the cwd fallback still only writes into empty slots, never overwrites). It enumerates only when relevant sessions exist and holds no store lock while enumerating.

### Changed

- **Licensing backend migrated from Lemon Squeezy to Creem**: `open_purchase_url` opens the Creem checkout; license activate/validate/deactivate go through the Creem API (`creem_test_*` key → test-api.creem.io, otherwise api.creem.io). ProductKind is narrowed to `MonthlySubscription` + `Lifetime`. Build-time envs are now `POKEPOKE_CREEM_*` (purchase URL / API key / two product ids); leaving them all empty still builds and launches — only the license feature is inert.
- **License storage location clarified as a local file**: `~/.pokepoke/license.json` (0600 permissions, **not macOS Keychain**); legacy files are migrated and chmod'd on upgrade.
- **WebView CSP tightened from null to an allowlist**: removes the arbitrary external-link / inline injection surface.
- **Log file auto-rotates past 5MB**: prevents unbounded log growth for the long-lived process.

### Fixed

- **Multi-device deactivation now keys on instance state**: fixes the "deactivate needs two clicks" bug — it judges by the instance's deactivated state rather than the whole order's status.
- **validate downgrades on expired/disabled state**: an expired/disabled license now correctly falls back to Free instead of being stuck on Pro.
- **Ghostty jump-targeting hardened** (brief `20260629-ghostty-jump-reconcile`): fixes three symptoms — jumping to Ghostty without switching to the right tab / focus not landing (needing a manual click) / wrong tab when multiple tabs share a cwd. The `focus_ghostty` L1 (by id) and L2 (by cwd) scripts now locate `window→tab→terminal` in three levels, then `activate window` + `select tab` + `focus` + `delay`; L1 additionally reads back `focused terminal` after focusing to verify, retrying (≤3×) until focus actually lands. Root cause: this machine's Ghostty `working directory` often stalls at the home dir (OSC7 not continuously reported), so the old single-capture cwd check always failed, the terminal_id never persisted, and every jump degraded to L2 cwd first-match (no verification, and collision-prone).

## [0.0.8] - 2026-06-25

### Added

- **Lemon Squeezy Pro licensing**: a License section in Settings supports activate / validate / deactivate; `open_purchase_url` opens the LS checkout page. The HTTP server validates daily; free-tier users get a daily notification quota, after which new popups are suppressed (`license_allows_new_disturbance` — a new popup suppression rule). New Tauri commands `get_entitlement` / `activate_license` / `validate_license` / `deactivate_license` / `open_purchase_url`. License state persists to `~/.poke-poke/license.json` (not macOS Keychain).
- **Pro product-kind distinction**: new `ProductKind::MonthlySubscription` (monthly subscription) alongside legacy monthly `Monthly` and lifetime `Lifetime`; new build-time env `POKEPOKE_LEMON_MONTHLY_SUBSCRIPTION_VARIANT_ID` (default `1826357`). The frontend labels each kind differently; persisted `"monthly"` still deserializes backward-compatibly to `Monthly`.
- **Claude Code transcript interruption reminder**: when a transcript contains an unrecovered `[Request interrupted by user]` marker, a neutral `pending` reminder nudges the user to return. The watcher scans only `running` Claude Code sessions and feeds a synthetic `pending` event through the existing `/notify` path to trigger popup/sound.

### Changed

- **Purchase entry collapsed to a single "Upgrade to Pro" button**: `open_purchase_url` drops the `kind` parameter; the two build-time envs (`POKEPOKE_LEMON_MONTHLY_URL` + `POKEPOKE_LEMON_LIFETIME_URL`) merge into a single `POKEPOKE_LEMON_PURCHASE_URL`; monthly vs lifetime selection now happens on the LS checkout page.
- **Repository is now proprietary closed-source**: MIT LICENSE and CONTRIBUTING.md removed; READMEs rewritten to a proprietary notice (no redistribution); the open-source contribution flow (public-mirror PR replay) is retired.

### Fixed

- **Pending is always eligible for popup, no longer suppressed by identical message**: the old `is_stage_popup_eligible` suppressed Pending→Pending with identical messages, swallowing hook retries — so when an agent sat at Pending waiting for approval/input, users missed the first popup and the agent went silent. Pending is now always eligible; frequency is bounded by `PopupCooldown` (3s) + `AckedStages` (which now skips Pending — acknowledging does not mean authorized).
- **Ghostty capture / cwd verification relaxed**: when shell integration doesn't report OSC7 the working directory is unavailable, and the old logic misread an empty cwd as "wrong tab captured", so the terminal id never persisted and jumps fell back to L2. Retries now allow an empty cwd (capture reason distinguishes `session_start` vs `retry-*`); focus verification distinguishes "tab doesn't exist" (invalidate) from "tab exists but cwd unreadable" (trust the persisted stable id and jump by id).
- **Lemon Squeezy variant_id parsing fixed + product_kind no longer wiped**: variant_id is read from the response's top-level `meta` object (previously misread from `license_key.variant_id`); `validate_license` preserves the last known good kind when the provider returns no kind, so a transient API gap no longer degrades the UI label to Free.

---

## [0.0.7] - 2026-06-01

### Changed

- **Ghostty `terminal_id` capture extended from SessionStart-only to multi-event retry** (brief `20260527-ghostty-capture-retry.md`, overrides b20520's "no recapture" Non-goal). Real-world logs showed SessionStart capture only hit ~72% (multi-tab parallel work / cc daemon resume / cwd-mismatch reject). The retry now fires on UserPromptSubmit / Notification / Stop for any ghostty session whose `terminal_id` is still None — `set_terminal_id_if_unset` keeps it idempotent so already-captured ids are never overwritten. Expected L1 hit rate ≥95%. **Pure win for the forward-jump users b20520 was for**: same-cwd multi-session click jumps now reliably target the right tab. **Hard prerequisite for the (not-yet-spec'd) reverse-channel brief** — that feature requires ≥95% L1 hit before it can avoid injecting "y" into the wrong session. Hook side: pokehook `terminal_app` field is now emitted on all cc/codex events (Cursor events still skip — Cursor rarely lives in Ghostty). Server side: `spawn_ghostty_capture` was extracted as a shared helper used by both the SessionStart path and the retry path, with a `reason` field in logs (`session_start` | `retry-user_prompt_submit` | `retry-notification` | `retry-stop`) for hit-rate bucketing during smoke.
- **`bot_outbound.include_message_body=true` upgraded from synthesized placeholder to real transcript summary + fixed LastFailed's empty `**message**:` row** (shipped alongside 0.0.7's Feishu push MVP). When Feishu push landed in 0.0.7, the outbound body was the pokehook-synthesized placeholder ("Session completed" / "" / "Working...\n{cwd}") — even with `include_message_body=true` users only saw these stub strings, and the LastFailed status had empty `message`, so the card rendered an empty `**message**:` row (looked half-broken to the user). This change wires up the `transcript_path` field that's already in cc/codex hook payloads (pointing at `~/.claude/projects/.../{session}.jsonl`); when outbound is enabled with `include_message_body=true`, it extracts a status-specific summary: LastFailed prefers the `isApiErrorMessage` text (the real API error body), Idle takes the last 3 `tool_use` entries + the final assistant text, Pending appends 2 `tool_use` entries as context after the Notification text, Running is skipped (transient). Each entry is clipped to 200 chars and the whole summary to 500 chars; Bash takes the cmd's first line, Edit/Read takes the file basename (full path never leaked). **When `include_message_body=false`, the jsonl is not even opened** (privacy hardgate that strengthens the existing fingerprinting boundary). All 5 pokehook handlers (SessionStart / UserPromptSubmit / Notification / Stop / StopFailure) now forward `transcript_path` (snake/camel both accepted); Session and NotifyRequest schemas gained `transcript_path: Option<String>` with `#[serde(default)]` for old `sessions.json` compatibility, and upsert uses Some-overwrites / None-keeps semantics so a later event missing the field doesn't wipe a known path. v1 covers cc only (jsonl schema is hardcoded); codex/cursor transparently fall back to the placeholder path until a follow-up brief covers them. New `bot_outbound/transcript.rs` module with 14 fixture tests covering happy/api_error/empty/malformed/clip/tail-truncation; plus 5 new bot_outbound tests and 1 end-to-end sessions test.

### Added

- **Mobile push (MVP): Feishu self-built application bot outbound** (brief `20260527-mobile-push-feishu-dingtalk`, **rescoped 2026-05-28**). A new "Mobile push" section in Settings: the user registers a custom app on Feishu Open Platform, fills in `app_id` / `app_secret` / their own `open_id` / endpoint (CN Feishu vs oversea Lark); whenever a desktop popup actually fires (`popup_created==true`) PokePoke asynchronously calls the Feishu IM API to **DM** the user a notification card. **Slightly more setup than a group webhook bot** but a much cleaner ongoing experience — no empty group cluttering the IM list, the bot DMs you directly. Two-layer gate: layer 1 is the existing popup suppression chain (`already_focused` / `recently_visited` / `acked_stages` / `popup_cooldown` / `notifications_enabled` / refresh-only all skip outbound too); layer 2 is `BotOutboundTrigger` ∈ `TerminalUnfocused` (default) / `Always` / `PendingOnly`. **`Always` does not bypass layer 1**. Token cache: `tenant_access_token` (2h TTL) cached per `app_id`, refreshed 5 min before expiry, shared across providers with the same `app_id`. Default `include_message_body=false`; default payload includes status / source / workspace basename / "PokePoke" / time / failure reason (LastFailed only); **task_id / session_id / full workspace path are never sent**. Default throttle 60s per session (Test button bypasses). Cards **strictly forbid** any action/button elements (`elements[*].tag` ∈ `{div, hr}` plus recursive scan), eliminating any "remotely controllable" suggestion. Logs flow through three pure redaction helpers so App Secret / open_id / token never reach disk. Failures rate-limited to 1-per-10 by `OutboundFailureLogLimiter`. Providers use UUID v4 stable IDs. First enable shows a one-time confirm dialog about "messages are relayed through Feishu's servers." New settings field `bot_outbound` (backward compatible — existing settings.json upgrades to `enabled=false`); new Tauri command `test_bot_outbound` (by provider id); new backend module `bot_outbound::{redact, throttle, token_cache}` with 38 unit tests covering token cache lifecycle / payload schema / field whitelist / redaction / throttle / log limiter / endpoint URLs. **DingTalk is not supported** (individual-developer onboarding is too heavy — Hermes / openclaw and similar internal tools' "zero-config" feel comes from corporate infrastructure, not from the App-bot path being easier). **Inbound** (replies / card buttons / public callbacks) is reserved for a separate brief. Setup guide: `docs/guides/mobile-push-setup.md`.

---

## [0.0.6] - 2026-05-26

### Fixed

- **Pending → Pending suppression swallowed Notifications with new content** (caught 2026-05-25): after a user dismissed one Pending popup, if cc fired another Notification with **different content** (e.g. "approval required for plan"), the 0.0.3+ "consecutive pending suppression" rule swallowed it — user got no alert, the session quietly updated in sessions.json. The old rule blanket-suppressed Pending → Pending (to defend against cc's same-message keep-alive polling), now changed to **suppress only when message text matches the previous one; treat a different message as a new request and pop**. `UpsertResult` gained `prev_message: Option<String>` to surface the old message; `should_show_popup` gained `new_message + prev_message` parameters to do the diff. Covers the user-reported "cc asked for plan approval but no popup" case while still blocking cc's same-message keep-alive polling. Tests: `should_show_popup_for_pending_to_pending_different_message` (new positive) + `should_show_popup_for_pending_to_pending_same_message` (negative for the original behavior) + a `prev_message=None` edge case.
- **Ghostty jump consistently lands on wrong tab** + **stale `terminal_id` self-heal** (caught 2026-05-25): with multiple cc sessions per workspace, popup ⌘N jumps **consistently** landed on a wrong Ghostty tab (not random), and even sessions from different workspaces jumped to the same wrong place. Two root causes chained: (1) `cwds_compatible` (the capture-time validator introduced by eec181f) **allowed reverse-direction prefix** (`session_workspace.starts_with(captured + "/")`), so when the Ghostty front terminal was the home shell (`/Users/edy`), **every session got that home tab's id stamped** — `/Users/edy` is an ancestor of nearly every workspace. The sessions.json showed `cc-c252379b (/Users/edy)` and `cc-de925cce (/Users/edy/.../star-maker)` sharing the same `terminal_id` exactly via this path. (2) `focus_ghostty` L1 by id trusted the stored id without re-verifying, so any historical pollution + (1)'s capture race meant **every subsequent click stably went to the wrong tab**. Two-part fix: (a) `cwds_compatible` dropped the reverse-direction branch — only `captured == session` or `captured starts with session/` qualify now; (b) `focus_ghostty` L1 now queries the current cwd of the stored id, runs it through `cwds_compatible`, and treats mismatch as L1-miss while signalling `invalidate_id` to the caller; the new `SessionStore::clear_terminal_id_if_matches(session_id, expected)` (race-safe — only clears when the current value matches `expected`) is invoked from `focus_session_terminal` / `open_session_source` on invalidate, so **already-polluted sessions self-heal on the first wrong click and fall through to L2 cwd-match next time** — no manual sessions.json edit required. Added unit tests: one for the new reverse-direction rejection, four for the clear helper (missing session / currently-None / mismatch no-op / matched clears). (caught in v0.0.5 real-world testing): when the user was focused on the originating Ghostty terminal, popups still fired instead of being suppressed by the `already_focused` gate. Root cause: the server-side Ghostty terminal_id capture gate was too tight — it only fired when the hook explicitly reported `terminal_app="ghostty"`. But Claude Code's resume / compact / background-reconnect paths spawn the hook from a daemon whose env may have **none of the Ghostty signals (TERM_PROGRAM / TERM / GHOSTTY_*)** — so even the 24a3d41 multi-env fallback couldn't detect Ghostty, `terminal_app` was reported as `None`, capture was skipped, `terminal_id` stayed `None` forever, and the Ghostty branch of `is_focused` (which needs `terminal_id` to match) always returned false. **Two-part fix**: (a) gate loosened to "try capture when `terminal_app` is missing OR explicitly `ghostty`"; only skip when `terminal_app` explicitly identifies a non-Ghostty terminal (iterm.app / apple_terminal / vscode / cursor / etc.) — the capture function itself already guards with `is_app_running("Ghostty")`. (b) **cwd validation** — loosening the gate introduces a risk of opportunistic capture grabbing the id of an unrelated Ghostty tab (cc resume firing in the background while the user has wandered off / iTerm2 user with an unrelated Ghostty tab in front). `capture_ghostty_focused_terminal_id` now also returns the captured tab's `working directory`; the server compares it against `session.workspace_path` using a prefix relation (allowing `cd` into a subdir after cc starts, or the inverse), and rejects the write if they don't match. This protects click-jump L1 from landing on the wrong tab and stops `is_focused` from false-suppressing popups for unrelated tabs. 8 unit tests cover exact match / trailing slash / subdir in both directions / unrelated paths / sibling-prefix false-match / empty / root edge cases.

---

## [0.0.5] - 2026-05-21

### Added

- **Ghostty 1.3+ terminal focus** (brief `20260520-ghostty-terminal-focus`): clicking a popup or pressing ⌘N now jumps to the correct Ghostty tab. When a SessionStart event arrives and the hook reports `terminal_app="ghostty"`, the server asynchronously runs `osascript -e 'tell app "Ghostty" to id of focused terminal of front window'` to capture Ghostty's stable terminal id and stores it on the session. Subsequent focus uses a three-layer fallback: **L1** focus by id (primary path — accurate even when multiple tabs share the same cwd) → **L2** focus by working directory first-match → **L3** activate Ghostty only. Ghostty not running / < 1.3 (no sdef) / user-switched-tab-in-200ms all fall through gracefully. First jump triggers a one-time macOS TCC prompt to "allow PokePoke to control Ghostty" — same UX as iTerm2 / Terminal.app first-jump.
- **Ghostty completeness: popup screen pick + auto-dismiss on focus** (tasks #54/#55, follow-up to the focus brief). `monitors::terminal_monitor` gained a Ghostty branch (AppleScript walks `windows × tabs × terminals` by terminal_id to find the owning window's bounds), so under `popup_screen_strategy=Terminal` a Ghostty session pops on the right screen instead of falling back to cursor monitor. `is_terminal_session_focused` got the matching Ghostty branch (matches `id of focused terminal of selected tab of front window` against the captured terminal_id), and the focus-watcher now spawns whenever **either** tty or terminal_id is present — Ghostty popups now auto-close within ~1.5s when the user refocuses the originating Ghostty terminal, instead of lingering until the next `Running` transition. Closes the (A) hypothesis from task #56 ("heard the sound but no popup" caused by popups landing on the wrong screen).
- **Ghostty detection: multi-env fallback** (caught from real logs): pokehook's `terminal_app()` previously only read `TERM_PROGRAM`, but Claude Code drops it in some resume / compact / background-reconnect paths (the hook is spawned from a cc daemon whose env was captured at daemon launch, not the user's current terminal). That made the server miss `terminal_app="ghostty"`, skip the id capture, and force click-to-jump down the L2 cwd-first-match path — which lands on the wrong Ghostty tab when multiple tabs share the same cwd. Added a Ghostty-specific fallback: if `TERM_PROGRAM` is empty, accept any of `TERM=xterm-ghostty` / `GHOSTTY_RESOURCES_DIR` / `GHOSTTY_SHELL_FEATURES` as a Ghostty signal (these come from terminfo / the Ghostty bundle and survive daemon-spawn env scrubbing). Priority: `TERM_PROGRAM` wins when present; fallback only runs when it's missing. 6 new unit tests cover each fallback path plus case-insensitivity.
- **Updater popup UX overhaul** (brief `20260521-updater-popup-ux`): upgrade popup now shows release notes (collapsed "What's new" toggle expanding into a ~120px scrollable text block) and a download/install progress bar. The Upgrade button is now split into two backend commands: `download_update_with_progress` emits `update-progress` events (frontend rAF-throttled to avoid render thrash) for the download phase, then `install_cached_update` runs install + restart with an indeterminate "Installing and restarting…" indicator. Either step failing surfaces a "Retry" button. The settings panel "Update and restart" button still uses the legacy one-shot `install_update` (no progress needed). `scripts/gen-latest-json.sh` now extracts the current version's CHANGELOG section (zh + en) into `latest.json.notes`, and the popup reads it via the new `get_pending_update_info` command — replacing the hardcoded `"See CHANGELOG.md for details."`. New i18n keys: `popup.update_downloading_pct`, `popup.update_downloading_bytes`, `popup.update_installing_restart`, `popup.update_retry`, `popup.update_notes_show`, `popup.update_notes_hide`, `popup.update_downloading`.

---

## [0.0.4] - 2026-05-20

### Added

- **Codex `PermissionRequest` event subscription** (brief `20260520-hook-coverage-expansion`): when a Codex agent needs user approval to run a command, PokePoke now raises a popup (Pending) — equivalent to Claude's `Notification.permission_prompt`. `--install-codex` writes 4 events (the original 3 + PermissionRequest); `--check-codex` verifies all 4. The popup message shows `Awaiting approval: <tool_name>` when the payload includes `tool_name`, falling back to a generic `Awaiting approval` otherwise. **Note: Codex 0.131+ hook trust gating still applies — after Connect, run `codex` (TUI) → `/hooks` and trust the entry once.**
- **`StopFailure`: fill in two missing reason mappings** (brief `20260520-hook-coverage-expansion`): the official Claude Code `StopFailure` matcher has 9 reasons; the frontend `KNOWN_REASONS` whitelist was missing `oauth_org_not_allowed` and `model_not_found`. Added both plus zh/en i18n strings so sessions hitting these reasons no longer fall back to the generic `unknown` ("API error") and users see the specific cause.

### Fixed

- **0.0.3 orphan NSPanel popup** (brief `20260520-nspanel-orphan-destroy-v2`): after a popup closed, a ghost NSPanel could remain on screen — hovering it showed the spinning cursor and global `⌘N` shortcuts got misaligned. Root cause was `close_popup_by_label` using `win.close()` (soft close), which in some timings got swallowed into an orderOut and never deallocated the NSPanel. Switched to `win.destroy()` (hard destroy); still routed through `panel.to_window()` so the ObjC class is restored to NSWindow first, avoiding the v1-fix landmine where skipping `to_window` triggered a selector mismatch → NSException → SIGABRT (stash@{0}, discarded).
- **⌘N shortcut dismissed every popup, not just the targeted one** (brief `20260520-nspanel-orphan-destroy-v2`): with 2+ popups on screen, pressing ⌘1 jumped to popup1 but also closed popup2/3/.... Root cause: `PopupWindow.tsx` used Tauri's app-wide `listen("popup-shortcut-trigger", ...)`, which ignores the backend's `app.emit_to(label, ...)` target filtering — every popup window received the event and ran `handleClick` (focus_session_terminal + close_popup_window). Switched to `getCurrentWebviewWindow().listen(...)` (window-scoped), so each popup only receives events targeted at its own label.

### Changed

- **Codex feature flag key migration** (brief `20260520-codex-feature-key-migration`): `--install-codex` now writes the canonical key `[features].hooks = true` (the old `codex_hooks` was a deprecated alias upstream); `--check-codex` accepts either key (`hooks || codex_hooks`), so users who were previously installed by an older PokePoke aren't falsely reported as "not enabled" after upgrade; `--uninstall-codex` leaves both keys untouched (continuing the existing "feature flag may reflect the user's independent preference" stance).

---

## [0.0.3] - 2026-05-19

### Added

- Settings fields `update_auto_check: bool` (default true) and `update_skip_version: Option<String>` — background updater check at 5s after startup and every 24h afterwards; when a newer release is available and the user hasn't skipped that version, an "update-available" popup is shown. The popup has three actions: [Upgrade now] / [Skip this version] / a top-right `×` (defer — popup goes away, settings untouched, next 24h check re-shows it). The popup **always overrides `popup_auto_close_secs`** — the update notice must not be eaten by a 5s/30s auto-close. Defer routes through a `dismiss_update_popup` command (does not touch `update_skip_version`), distinct from "Skip this version" semantics.
- Global shortcuts `⌘1` ~ `⌘9` + `⌘Enter` — while session popups are on screen, the corresponding shortcut behaves exactly like clicking that popup (focuses the session terminal and dismisses the popup). `⌘Enter` is an alias for `⌘1`. Each session popup shows a `⌘N` badge in the bottom-right indicating its current number.
- When `popup_list` is empty, all global shortcuts are **unregistered**, releasing the busy `⌘Enter`/`⌘N` combos back to other apps; they're re-registered only when popups appear (`min(N,9)+1` shortcuts).

### Fixed

- **Update popup label containing `.` was rejected by Tauri** (a latent bug introduced by the unique-label refactor; would have killed every real update popup on first occurrence): Tauri webview labels only allow alphanumeric + `-`/`_`/`:`/`/`, but semver always contains `.`, and our `popup-u-{token}-{version}` ran straight into the wall. Fix: a new `version_to_label_token` helper encodes `.` and `+` (semver build metadata) to `_`; `update_popup_label` and `find_update_popup_label` use the encoding symmetrically. The `update_popup_label_is_tauri_safe_for_semver` regression test pins the contract across `9.9.9` / `1.0.0-rc.1` / `1.0.0+build.42`.
- **Several updater-task UX papercuts fixed in one pass** (review items H1/H2/H3a/H4):
  - **Toggle changes took up to 24h to take effect** → switched to 60s tick + `next_check_at` state. Flipping `update_auto_check` false→true now becomes effective within 60s instead of waiting for the next day's tick.
  - **Any check failure pushed the next retry out by 24h** → on `Err`, retry in 15min instead. Laptops that boot without network and miss the 5s initial check no longer lose an entire day.
  - **`install_update` did its own `updater.check()` instead of reusing the background result** → background task now caches the `Update` handle in `PendingUpdate: Arc<Mutex<Option<Update>>>`, and `install_update` reads it directly. This drops the extra round-trip (button hang) and removes a corner case where a retracted GitHub release made the install button surface "no update available".
  - **Install failures left the user staring at the popup with no feedback** → frontend now catches the error and renders a red "Upgrade failed, try again later" line above the buttons; hover for the underlying error string.
- **popup bottom-left time switched from `session.created_at` to `session.updated_at`** — old sessions firing a new event no longer show "4h ago" (which made it look like an old popup had risen from the dead); matches the session-time semantic in the panel.
- **`show_popup` now does same_screen + visual_index + push atomically under a single `popup_list` lock** (review item M2): previously `calculate_position` took a lock → `session_popup_count` took another → push later took a third; two concurrent `show_popup` calls could compute mismatched `visual_index` values, so the URL hash baked the wrong `⌘N` into the popup for one frame (`emit_popup_shortcuts_updated` would correct the badge within ~50ms via store-emit, but the first frame was wrong). Replaced with a single `reserve_slot_and_position` that reads + pushes atomically, so the first frame is correct.
- **pokehook strips the `<source>:` prefix from the popup `title`** (`"Claude Code: poke-poke"` → `"poke-poke"`): the popup already has a source tag (CLAUDE-CODE / CURSOR / CODEX) in the top-right, so the prefix was redundant; the now-dead `Source::label()` helper was removed.
- **popup labels switched to "unique-per-instance" format to eliminate the entire cross-instance race class** (root-cause fix — the "I heard the sound but no popup" bug that kept resurfacing in 0.0.2 / 0.0.3 ends here). The old `popup-{session_id}` design reused the same label across different popups of the same session, violating Tauri's own `WebviewWindowBuilder` guidance ("use unique labels per instance"). Manifestations:
  - 0.0.2: auto-close timer woke up and checked the label, killing the successor popup of the same session (patched with a token check)
  - 0.0.3: focus-watcher had the exact sibling bug (also patched with a token check)
  - close_popup's main-thread destroy dispatch could in theory hit a newly-built same-label panel that landed in Tauri's registry between our list mutation and the destroy execution (race not observed thanks to main-thread FIFO ordering, but reusing the label was an anti-pattern walking next to a foot-gun)
  
  Refactor: new format `popup-s-{token}-{session_id}` / `popup-u-{token}-{version}`. A `POPUP_INSTANCE_COUNTER` gives every popup instance a unique token, so **no two popups ever share a label**. Companion changes:
  - `close_popup(session_id)` now goes through `find_session_popup_label` to look up the current label; `mark_update_skipped(version)` goes through `find_update_popup_label`
  - The auto-close timer and focus-watcher both capture their `label` at creation time and call `close_popup_by_label(my_label)` on fire — no way to reach a successor instance by construction
  - The frontend's `popup-` prefix strip is replaced by a regex `popup-s-\d+-(.+)` to extract `session_id`
  - The label namespace no longer has any reuse → Tauri's internal close-then-recreate label race (the same family as [#9353](https://github.com/tauri-apps/tauri/issues/9353)) is no longer reachable for us

### Changed

- popup URL hash gains two parameters: `kind=session|update` (the frontend routes to different templates per kind) and `n=<0-based visual index>` (used by session popup to render the `⌘N` badge). Numbering is global across screens.
- Settings panel About section adds an `update_auto_check` toggle; window height bumped 990 → 1040 to fit the new row.

---

## [0.0.2] - 2026-05-18

### Added

- Settings field `popup_auto_close_secs: Option<u32>` — auto-close the popup N seconds after it appears. `None` (default) = never, preserving 0.0.1 behavior; the settings panel adds an "Auto-Close" dropdown with Never / 5s / 15s / 30s / 1min / 3min. The backend does not whitelist values: any `>0` integer is honored so users can hand-edit `settings.json`.

### Fixed

- macOS `close_popup` destroy chain (`panel.to_window()` calls `object_setClass` + `win.close()`) was historically invoked from multiple background threads (tokio worker / focus-watcher / Tauri command handler / auto-close timer). Mutating AppKit obj-c class from off the main thread is UB — it manifested as `popup_list` cleared while the on-screen panel persisted, with the React content silently refreshing to the Running state, looking like the popup never closed. `close_popup` now dispatches its destroy chain via `run_on_main_thread`, making all callers automatically safe.
- Auto-close timer matches the popup by `(label, token)` rather than label alone. Labels are `popup-{session_id}` and get reused when the same session pops again; a stale timer from a previous popup could otherwise close the wrong (newer) instance — visible to the user as "the popup didn't even flash".

### Changed

- Log timestamps switched from UTC + `Z` to local time + `+HH:MM` offset (e.g. `2026-05-18T16:13:00.123+08:00`) so they line up with terminal time without mental conversion.

---

## [0.0.1] - 2026-05-12

First official release. The entries below describe the **initial behavior contract**. Intermediate states from the early development period (transport TCP↔UDS round trips, the `endpoints.json` multi-instance discovery table, `~/.pokepoke/locks/` file locks, the `_pokepoke` managed marker, `install-meta.json` metadata, debug-mode toggle, global shortcut, one-click Codex onboarding, autostart…) are no longer tracked here — see git log for the trajectory.

### Onboarding / install

- One-click Claude Code onboarding: writes the `SessionStart` / `UserPromptSubmit` / `Notification` / `Stop` / `StopFailure` events into `~/.claude/settings.json`; Disconnect mirrors uninstall.
- One-click Cursor onboarding (user-level global): writes `sessionStart` / `beforeSubmitPrompt` / `stop` / `sessionEnd` to `~/.cursor/hooks.json`.
- Codex CLI: **not** managed (`~/.codex/hooks.json` / `~/.codex/config.toml` are user-maintained); events flow via the strong `turn_id` payload signal + `--source=codex` arg, and panel / popup still display correctly.
- Hook command shape: `~/.local/bin/poke-hook --source=<cc|codex|cursor>`, source pinned at install time.
- Startup-time hook-binary deploy: bundled `Contents/MacOS/pokehook` is force-copied to `~/.local/bin/poke-hook` (idempotent every launch, ~10 ms), giving zero-friction protocol upgrades — **no** `pnpm install:hook` needed.
- Startup-time hook-config drift check: when the command string is stale (missing the `--source=<x>` suffix), emits `hook-drift-detected` → panel header shows a warning + a per-source "one-click fix" button.
- Pre-write snapshots: every rewrite of `~/.claude/settings.json` / `~/.cursor/hooks.json` is first copied to `~/.pokepoke/snapshots/<source>-<ts>.json`; each source pool keeps the last 5 (write-only — restore is a manual `cp`).

### Changed - 2026-04-27

- Codex CLI is back as a PokePoke-managed integration: the tray's Connect / Disconnect / Check writes `~/.codex/hooks.json` + `~/.codex/config.toml` and enables `[features] codex_hooks = true`.
- Startup-time hook drift detection and panel one-click fix now cover Codex too; the fix path goes through `--install-codex`.
- Pre-write snapshots cover both Codex files: `codex-hooks-<ts>.json` and `codex-config-<ts>.toml` are pooled separately.

### Source detection

`hook.rs::detect_source` uses a three-tier priority:
1. Payload-exclusive fields (strong signal): `workspace_roots` / `workspaceRoots` → Cursor; `turn_id` → Codex
2. The `--source=<x>` arg pinned at install time
3. Fallback → ClaudeCode

Strong signal beating arg covers the Cursor *Third-party hooks* double-fire case: Cursor reads the `--source=cc` command from `~/.claude/settings.json`, but the payload is actually Cursor's (carries `workspace_roots`); the strong-signal override prevents misclassification.

### Transport

- Hook ↔ server runs over a fixed UDS at `~/.pokepoke/sock`, file 0600 / parent dir 0700.
- Single-instance semantics: at startup, if the sock exists → probe-connect → if reachable, a peer is alive and this process exits; if not, unlink the stale socket and re-bind.
- No TCP ports opened. No external services contacted.
- The hook side hand-rolls HTTP/1.1 over UnixStream with 2 s read/write timeouts; when the server is offline, events are silently dropped and a `PrintOnDrop` writes a valid `{}` to satisfy the hook host's contract.

### Session state machine + liveness

- States: `running` / `pending` / `idle` / `last_failed` (`success` / `failure` are kept as serde aliases for backward compatibility with old data).
- `event_type` distinguishes "state update" from "session truly ended" (`session_end` removes the row directly without entering the four-state model).
- Liveness: a 5 s periodic `ps` probe across all sessions; reap only after 2 consecutive misses (10 s grace, guards against transient `ps` flakes). Layered by source: claude-code / codex use `ps -t <tty> -o comm=`; cursor uses `pgrep -x Cursor` (app-level granularity).
- On startup the panel is unconditionally cleared (waits for agents to re-report) so stale rows can't linger.

### Event dedupe

`http_server::DedupeWindow` keys on `(task_id, signature)` within a 100 ms in-memory window; on hit it returns `200 + {"deduped": true}`, leaves the store untouched, fires no popup, and plays no sound. Covers the third-party-hooks double-fire case (Cursor reading both `~/.claude/settings.json` and `~/.cursor/hooks.json`).

### Settings

- `alert_sound`: any system sound / `mute` (default `system:Glass`)
- `locale`: `zh` / `en` (default `zh`)
- `notifications_enabled`: master notification switch (default `true`)
- `popup_position`: any of the 4 screen corners (default `top-right`)
- `tray_icon_style`: `colored` / `white` (default `colored`)

Removed legacy `settings.json` fields (`auto_start` / `panel_shortcut` / `dev_mode`) are silently ignored by serde — upgrades cost nothing.

### Popup

- macOS uses NSPanel (`PokePopupPanel`, nonactivating + can_become_key=false); does not steal focus.
- Position follows the `popup_position` setting; stack direction and CSS slide-in animation direction follow the chosen corner.
- Loss of focus / clicking "jump" / panel hidden / `event_type=session_end` all dismiss the popup.
- Click-to-jump dispatch via the pure function `decide_focus_target`: with a tty → AppleScript pinpoints the iTerm2 / Terminal tab; Cursor GUI without tty → `cursor <workspace_path>`; otherwise no tty → `open <workspace_path>` opens Finder.

### Tray / Panel

- Multi-screen positioning: `pick_monitor_at_point` walks `available_monitors()` in physical coordinates to find the screen the click landed on, then places the panel in that screen's logical coordinate system (no longer always snapping to the primary).
- ⏻ quit button at the panel's top-right → `quit_app` Tauri command → `app.exit(0)`.

### POKEPOKE_DEBUG

Setting `POKEPOKE_DEBUG=1` (or `true`/`yes`/`on`) at startup turns on DEBUG-level logging into `~/.pokepoke/pokepoke.log`. There is no runtime channel switch.

### Bundle Identifier

Locked to `com.pokepoke.PokePoke` (will **not** change after this — changing it breaks the upgrade path).

### Manual residue cleanup (optional)

The code does not actively clean these files / directories on machines from older versions; `rm` them yourself if needed:

```bash
rm -rf ~/.pokepoke/sockets ~/.pokepoke/endpoints.json ~/.pokepoke/endpoints.lock
rm -rf ~/.pokepoke/locks
rm  -f ~/.pokepoke/install-meta.json
```

---

## History

No official release yet.
