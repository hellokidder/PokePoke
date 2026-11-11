#!/usr/bin/env node

// Test the opencode plugin JS logic (embedded in pokehook.rs) by:
//   - Extracting the plugin JS from the Rust source
//   - Mocking child_process.spawn to capture forward() calls
//   - Simulating opencode event sequences to verify:
//     1. session.idle is debounced 10s before forwarding Stop
//     2. message.updated cancels pending idle timers
//     3. session.created still forwards SessionStart immediately
//     4. permission.updated still forwards Notification immediately
//     5. permission.updated cancels pending idle timers (waiting for approval != idle)

import { readFileSync } from "fs";
import vm from "vm";
import { strict as assert } from "assert";

// ---- Step 1: Read the opencode plugin JS ----
// pokehook.rs 自 6cb60fe 起以 include_str!("pokehook/opencode-plugin.js") 引入,
// 不再内嵌 r#"..."# 常量。这里直接读同一源文件,与 include_str! 路径一致,
// 避免测试脚本与抽取后的结构脱节。
const pluginPath = new URL(
  "../src-tauri/src/bin/pokehook/opencode-plugin.js",
  import.meta.url,
);
let pluginJs = readFileSync(pluginPath, "utf-8");

// Replace the __POKE_HOOK_BIN__ placeholder with a known test path
pluginJs = pluginJs.replaceAll("__POKE_HOOK_BIN__", "/tmp/test-poke-hook");

// ---- Step 2: Prepare sandbox to execute the plugin JS ----
// The plugin uses ESM `import { createRequire } from "module"` at top level.
// We run it in a vm.Script that provides a mock `module` object with
// createRequire, and a mocked child_process.spawn that captures forwarded
// payloads.

const capturedCalls = [];
const mockTimers = [];

function buildSandbox() {
  // Rewrite the plugin JS: remove the top-level `import` and replace it
  // with an inline createRequire-powered require.
  // Also replace `import.meta.url` with a fixed string since vm sandbox
  // doesn't support it.
  // Replace `export default` with an assignment so it works in vm sandbox.
  const modified = pluginJs
    .replace(
      'import { createRequire } from "module"',
      'const createRequire = () => module.__require__'
    )
    .replaceAll("import.meta.url", '"file:///test/plugin.js"')
    .replace("export default", "module.exports =");

  const sandbox = {
    // Spy tracking (shared between sandbox and test runner)
    __handler: null,
    __capturedCalls: capturedCalls,
    __mockTimers: mockTimers,

    module: {
      __require__: (mod) => {
        // Mock require that returns child_process.spawn
        if (mod === "child_process") {
          return {
            spawn: () => {
              const stdin = {
                end: () => {},
                on: () => {},
                write: (d) => capturedCalls.push(d),
              };
              return { stdin, on: () => {} };
            },
          };
        }
        return {};
      },
      exports: {},
    },
    exports: {},

    // Mock setTimeout/clearTimeout (must come before the shorthand section below
    // which previously shadowed these with real Node.js globals)
    setTimeout: (fn, delay) => {
      const timer = { id: mockTimers.length, fn, delay, cancelled: false };
      mockTimers.push(timer);
      return timer.id;
    },
    clearTimeout: (id) => {
      const t = mockTimers.find((t) => t.id === id);
      if (t) t.cancelled = true;
    },

    // Minimal URL polyfill (the plugin doesn't use it but import.meta.url
    // is referenced via `import.meta` somewhere)
    URL: class URL {
      constructor(href) { this.href = href; }
      toString() { return this.href; }
    },
    "import.meta": { url: "file:///test/plugin.js" },

    // Global objects expected by JS runtime
    Set,
    Map,
    JSON,
    String,
    Number,
    Boolean,
    Object,
    Array,
    Error,
    console,
    Promise,
  };

  return { sandbox, modified };
}

// ---- Step 3: Test helper ----
class TestCase {
  constructor(name) {
    this.name = name;
  }

  prepare() {
    capturedCalls.length = 0;
    mockTimers.length = 0;
  }

  // Fire an event to the plugin (opencode passes { event: { type, properties } })
  async fire(type, properties) {
    const h = getHandler();
    await h({ event: { type, properties } });
  }

  // Advance mock time by firing all pending timers up to a given duration
  advanceTime(ms) {
    const toFire = mockTimers.filter(
      (t) => t.delay <= ms && !t.cancelled
    );
    // Sort by delay
    toFire.sort((a, b) => a.delay - b.delay);
    const result = [];
    for (const timer of toFire) {
      if (!timer.cancelled) {
        timer.fn();
        result.push(timer);
      }
    }
    return result;
  }

  get calls() {
    return capturedCalls.map((c) => {
      try {
        return JSON.parse(c);
      } catch {
        return { raw: c };
      }
    });
  }

  get timers() {
    return mockTimers;
  }
}

// ---- Step 4: Build and execute plugin ----
let context;
let contextInit;

async function rebuildPlugin() {
  const prepared = buildSandbox();

  // Clear spies
  capturedCalls.length = 0;
  mockTimers.length = 0;

  context = vm.createContext(prepared.sandbox);

  const script = new vm.Script(`
    ${prepared.modified}
    typeof module.exports;
  `);
  script.runInContext(context);

  const exportType = typeof context.module.exports;
  if (exportType !== "function") {
    throw new Error("Plugin code did not set module.exports to a function, got " + exportType);
  }

  const pluginResult = context.module.exports({ directory: "/test/project" });
  const plugin = await Promise.resolve(pluginResult);
  if (typeof plugin !== "object" || plugin === null || typeof plugin.event !== "function") {
    console.error("plugin type:", typeof plugin);
    console.error("plugin keys:", plugin ? Object.keys(plugin) : "null");
    throw new Error("Plugin returned invalid structure");
  }
  context.__handler = plugin.event;

  // Reset smoke test state
  capturedCalls.length = 0;
  mockTimers.length = 0;
}

// ---- Step 5: Run tests ----

const tests = [];

function getHandler() {
  if (typeof context.__handler !== "function") {
    console.error("context.__handler value:", String(context.__handler));
    console.error("context.__handler type:", typeof context.__handler);
    console.error("context.__handler keys:", context.__handler ? Object.keys(context.__handler) : "null/undefined");
    throw new Error("Plugin not initialized yet — await contextInit first. typeof=" + typeof context.__handler);
  }
  return context.__handler;
}

// The main entry point runs tests; ensure plugin is initialized first.
async function runTests() {
  // First call: init from the module-level rebuildPlugin() call
  if (contextInit) await contextInit;
  // Rebuild for fresh state, and wait for the new plugin to be ready
  await rebuildPlugin();

tests.push(async () => {
  const t = new TestCase("session.created forwards SessionStart immediately");
  t.prepare();
  await t.fire("session.created", {
    info: { id: "ses_abc123", directory: "/test/proj" },
  });
  assert.equal(t.calls.length, 1, "should forward exactly once");
  assert.equal(t.calls[0].hook_event_name, "SessionStart");
  assert.equal(t.calls[0].session_id, "ses_abc123");
  assert.equal(t.calls[0].cwd, "/test/proj");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase("session.created dedupes by session id");
  t.prepare();
  await t.fire("session.created", {
    info: { id: "ses_abc", directory: "/proj" },
  });
  await t.fire("session.created", {
    info: { id: "ses_abc", directory: "/proj" },
  });
  assert.equal(t.calls.length, 1, "duplicate session.created should be dropped");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "dedup set soft-caps to avoid unbounded memory growth"
  );
  t.prepare();
  await t.fire("session.created", { info: { id: "ses_A", directory: "/p" } });
  assert.equal(t.calls.length, 1, "first ses_A forwarded");
  // Fill past the 2000 cap to trigger a clear
  for (let i = 0; i < 2001; i++) {
    await t.fire("session.created", { info: { id: `ses_${i}`, directory: "/p" } });
  }
  // After the clear, ses_A is forgotten -> re-firing forwards it again
  await t.fire("session.created", { info: { id: "ses_A", directory: "/p" } });
  const last = t.calls[t.calls.length - 1];
  assert.equal(last.hook_event_name, "SessionStart");
  assert.equal(last.session_id, "ses_A", "ses_A should re-forward after cap clear");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "session.idle does not forward Stop immediately; timer is set"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  assert.equal(
    t.calls.length,
    0,
    "session.idle should NOT forward Stop immediately"
  );
  assert.equal(
    t.timers.length,
    1,
    "should create one debounce timer"
  );
  assert.equal(
    t.timers[0].delay,
    10000,
    "debounce should be 10 seconds"
  );
  assert.ok(
    !t.timers[0].cancelled,
    "timer should not be cancelled"
  );
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "session.idle timer fires Stop after 10s if no message arrives"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  assert.equal(t.calls.length, 0, "no call before timer fires");

  const fired = t.advanceTime(10000);
  assert.equal(fired.length, 1, "one timer should fire");
  assert.equal(t.calls.length, 1, "Stop should be forwarded after timer fires");
  assert.equal(t.calls[0].hook_event_name, "Stop");
  assert.equal(t.calls[0].session_id, "ses_abc");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "message.updated cancels pending idle timer"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  assert.equal(t.calls.length, 0, "no call yet");

  // A new user message arrives (iteration continues)
  await t.fire("message.updated", {
    info: {
      role: "user",
      id: "msg_new",
      sessionID: "ses_abc",
    },
  });
  assert.equal(t.calls.length, 1, "should forward UserPromptSubmit");
  assert.equal(t.calls[0].hook_event_name, "UserPromptSubmit");

  // Advance time past 10s — timer should have been cancelled
  const fired = t.advanceTime(10000);
  assert.equal(fired.length, 0, "no timers should fire (all cancelled)");
  assert.equal(
    t.calls.length,
    1,
    "no additional calls after advancing time"
  );
  assert.ok(t.timers[0].cancelled, "idle timer should be cancelled");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "consecutive session.idle resets the debounce timer (only one fires)"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  await t.fire("session.idle", { sessionID: "ses_abc" }); // resets
  // First timer was cleared, second timer created
  const activeTimers = t.timers.filter((t) => !t.cancelled);
  assert.equal(activeTimers.length, 1, "only one active timer should remain");
  assert.ok(t.timers[0].cancelled, "first timer should be cancelled");
  assert.ok(!t.timers[1].cancelled, "second timer should be active");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "different session IDs have independent idle timers"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  await t.fire("session.idle", { sessionID: "ses_xyz" });

  const activeTimers = t.timers.filter((t) => !t.cancelled);
  assert.equal(activeTimers.length, 2, "two independent timers should exist");

  // Advance to 10s — both fire
  const fired = t.advanceTime(10000);
  assert.equal(fired.length, 2, "both timers should fire");
  assert.equal(t.calls.length, 2, "two Stop events should be forwarded");
  assert.equal(t.calls[0].session_id, "ses_abc");
  assert.equal(t.calls[1].session_id, "ses_xyz");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "message.updated only cancels timer for its own session"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  await t.fire("session.idle", { sessionID: "ses_xyz" });

  // New message for ses_abc only
  await t.fire("message.updated", {
    info: { role: "user", id: "msg_x", sessionID: "ses_abc" },
  });

  await t.fire("message.updated", {
    info: { role: "assistant", id: "msg_y", sessionID: "ses_abc" },
  });

  const fired = t.advanceTime(10000);
  assert.equal(fired.length, 1, "only ses_xyz timer should fire");
  assert.equal(t.calls.length, 2, "UserPromptSubmit + Stop for ses_xyz");
  assert.equal(t.calls[0].hook_event_name, "UserPromptSubmit");
  assert.equal(t.calls[0].session_id, "ses_abc");
  assert.equal(t.calls[1].hook_event_name, "Stop");
  assert.equal(t.calls[1].session_id, "ses_xyz");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "permission.updated forwards Notification immediately (no debounce)"
  );
  t.prepare();
  await t.fire("permission.updated", {
    sessionID: "ses_abc",
    title: "Write tool",
  });
  assert.equal(t.calls.length, 1, "should forward immediately");
  assert.equal(t.calls[0].hook_event_name, "Notification");
  assert.equal(
    t.calls[0].message,
    "Awaiting approval: Write tool"
  );
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "permission.updated cancels pending idle timer (waiting for approval is not idle)"
  );
  t.prepare();
  await t.fire("session.idle", { sessionID: "ses_abc" });
  assert.equal(t.calls.length, 0, "no call yet - idle is debounced");

  // Permission prompt arrives within the 10s debounce window
  await t.fire("permission.updated", {
    sessionID: "ses_abc",
    title: "Write tool",
  });
  assert.equal(t.calls.length, 1, "should forward Notification immediately");
  assert.equal(t.calls[0].hook_event_name, "Notification");

  // Advance past 10s - the idle timer must have been cancelled, no Stop fires
  // to clobber the Pending status (server upsert has no Pending-over-Idle guard).
  const fired = t.advanceTime(10000);
  assert.equal(fired.length, 0, "idle timer should be cancelled by permission.updated");
  assert.equal(t.calls.length, 1, "no Stop should fire - agent is still waiting for approval");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  const t = new TestCase(
    "session.error forwards StopFailure immediately"
  );
  t.prepare();
  await t.fire("session.error", {
    sessionID: "ses_abc",
    error: { name: "ProviderError", type: "rate_limit" },
  });
  assert.equal(t.calls.length, 1);
  assert.equal(t.calls[0].hook_event_name, "StopFailure");
  assert.equal(t.calls[0].reason, "ProviderError");
  console.log(`  PASS: ${t.name}`);
});

// ---- Step 6: Run all tests ----
  let passed = 0;
  let failed = 0;
  for (const test of tests) {
    try {
      await test();
      passed++;
    } catch (e) {
      console.log(`  FAIL: ${e.message}`);
      failed++;
    }
  }

  console.log(`\n${passed}/${passed + failed} tests passed`);
  if (failed > 0) {
    process.exit(1);
  }
}

runTests();
