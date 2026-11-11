#!/usr/bin/env node

// Test the pi extension TS logic (embedded in pokehook.rs as PI_EXTENSION_TS) by:
//   - Extracting the extension source from the Rust source
//   - Running it in a node `vm` sandbox with a mocked child_process.spawn
//     that captures forward() payloads, and a fake `pi` object that
//     collects on() handlers
//   - Driving each pi event (session_start / before_agent_start /
//     agent_settled / session_shutdown) and asserting the forwarded
//     hook_event_name sequence matches the PokePoke contract.
//
// Pi's extension model differs from opencode's: the extension exports a
// factory `function (pi)` that registers handlers via `pi.on(event, handler)`,
// rather than returning an `{ event }` dispatcher. The sandbox mirrors that.

import { readFileSync } from "fs";
import vm from "vm";
import { strict as assert } from "assert";

// ---- Step 1: Read the pi extension TS ----
// pokehook.rs 自 6cb60fe 起以 include_str!("pokehook/pi-extension.ts") 引入,
// 不再内嵌 r#"..."# 常量。直接读同一源文件,与 include_str! 路径一致。
const extPath = new URL(
  "../src-tauri/src/bin/pokehook/pi-extension.ts",
  import.meta.url,
);
let extSrc = readFileSync(extPath, "utf-8");

// Replace the __POKE_HOOK_BIN__ placeholder with a known test path.
extSrc = extSrc.replaceAll("__POKE_HOOK_BIN__", "/tmp/test-poke-hook");

// ---- Step 2: Prepare sandbox ----
// The extension uses ESM `import { spawn } from "node:child_process"` and
// `export default function`. Rewrite both so they run in a vm sandbox.
const capturedCalls = [];

function buildSandbox() {
  const modified = extSrc
    .replace(
      'import { spawn } from "node:child_process";',
      "const spawn = globalThis.__spawn;"
    )
    .replace("export default function", "module.exports = function");

  const spawn = () => ({
    stdin: {
      write: (d) => capturedCalls.push(d),
      end: () => {},
      on: () => {},
    },
    on: () => {},
  });

  const sandbox = {
    __spawn: spawn,
    module: { exports: {} },
    exports: {},
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
  // sandbox.globalThis must reference itself so `globalThis.__spawn` resolves
  // to the sandbox-level __spawn above.
  sandbox.globalThis = sandbox;
  return { sandbox, modified };
}

// ---- Step 3: Test harness ----
let handlers = {};

async function rebuild() {
  capturedCalls.length = 0;
  handlers = {};
  const { sandbox, modified } = buildSandbox();
  const context = vm.createContext(sandbox);
  const script = new vm.Script(`${modified}\ntypeof module.exports;`);
  script.runInContext(context);

  const factory = sandbox.module.exports;
  if (typeof factory !== "function") {
    throw new Error(
      "Extension did not set module.exports to a function, got " + typeof factory
    );
  }

  // Fake pi: collect on() registrations. Handlers are async; caller awaits.
  const pi = {
    on: (event, handler) => {
      handlers[event] = handler;
    },
  };
  factory(pi);
}

class TestCase {
  constructor(name) {
    this.name = name;
  }
  prepare() {
    capturedCalls.length = 0;
  }
  // Fire a pi event handler. ctx controls mode/cwd/sessionManager.isIdle.
  async fire(event, ctx = {}) {
    const h = handlers[event];
    assert.ok(typeof h === "function", `no handler registered for ${event}`);
    await h({}, ctx);
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
}

function ctx({ mode = "tui", cwd = "/proj", sid = "019f89bd-6cee-7635-a2e9-aa5886a5bff1" } = {}) {
  return {
    mode,
    cwd,
    sessionManager: {
      getSessionId: () => sid,
    },
    isIdle: () => true,
  };
}

// ---- Step 4: Tests ----
const tests = [];

tests.push(async () => {
  await rebuild();
  const t = new TestCase("tui session_start forwards SessionStart exactly once");
  t.prepare();
  await handlers["session_start"]({}, ctx());
  assert.equal(t.calls.length, 1);
  assert.equal(t.calls[0].hook_event_name, "SessionStart");
  assert.equal(t.calls[0].session_id, "019f89bd-6cee-7635-a2e9-aa5886a5bff1");
  assert.equal(t.calls[0].cwd, "/proj");
  // Duplicate session_start for same id -> deduped
  await handlers["session_start"]({}, ctx());
  assert.equal(t.calls.length, 1, "duplicate session_start should be dropped");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("print mode session_start forwards nothing");
  t.prepare();
  await handlers["session_start"]({}, ctx({ mode: "print" }));
  assert.equal(t.calls.length, 0, "print/rpc/json runs have no pane to track");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("rpc mode session_start forwards nothing");
  t.prepare();
  await handlers["session_start"]({}, ctx({ mode: "rpc" }));
  assert.equal(t.calls.length, 0);
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("ephemeral session (no id) forwards nothing");
  t.prepare();
  // Inline ctx: getSessionId() returns undefined (--no-session).
  // (ctx({ sid: undefined }) would trip the destructure default, so build it directly.)
  await handlers["session_start"](
    {},
    { mode: "tui", cwd: "/proj", sessionManager: { getSessionId: () => undefined }, isIdle: () => true }
  );
  assert.equal(t.calls.length, 0, "--no-session has no stable identity");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("before_agent_start forwards UserPromptSubmit");
  t.prepare();
  await handlers["session_start"]({}, ctx());
  await handlers["before_agent_start"]({}, ctx());
  assert.equal(t.calls.length, 2);
  assert.equal(t.calls[1].hook_event_name, "UserPromptSubmit");
  assert.equal(t.calls[1].session_id, "019f89bd-6cee-7635-a2e9-aa5886a5bff1");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("agent_settled forwards Stop (not agent_end)");
  t.prepare();
  await handlers["session_start"]({}, ctx());
  await handlers["agent_settled"]({}, ctx());
  assert.equal(t.calls.length, 2);
  assert.equal(t.calls[1].hook_event_name, "Stop");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("session_shutdown forwards SessionEnd and clears session");
  t.prepare();
  await handlers["session_start"]({}, ctx());
  await handlers["session_shutdown"]({}, ctx());
  assert.equal(t.calls.length, 2);
  assert.equal(t.calls[1].hook_event_name, "SessionEnd");
  // After shutdown, sessionId is cleared -> later agent_settled forwards nothing
  await handlers["agent_settled"]({}, ctx());
  assert.equal(t.calls.length, 2, "post-shutdown agent_settled must not forward");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  await rebuild();
  const t = new TestCase("before_agent_start forwards nothing before session_start");
  t.prepare();
  await handlers["before_agent_start"]({}, ctx());
  assert.equal(t.calls.length, 0, "no session id yet -> no forward");
  console.log(`  PASS: ${t.name}`);
});

tests.push(async () => {
  // Swallow test: if spawn throws, forward()'s try/catch must absorb it so
  // a poke-hook outage never surfaces into pi.
  await rebuild();
  const t = new TestCase("spawn failure is swallowed, not propagated");
  t.prepare();
  const { sandbox, modified } = buildSandbox();
  sandbox.globalThis.__spawn = () => {
    throw new Error("boom");
  };
  const context = vm.createContext(sandbox);
  new vm.Script(`${modified}\ntypeof module.exports;`).runInContext(context);
  handlers = {};
  const pi = { on: (e, h) => (handlers[e] = h) };
  sandbox.module.exports(pi);
  // Must not throw:
  await handlers["session_start"]({}, ctx());
  assert.equal(t.calls.length, 0, "spawn threw -> forward swallowed, nothing captured");
  console.log(`  PASS: ${t.name}`);
});

// ---- Step 5: Run ----
async function runTests() {
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
