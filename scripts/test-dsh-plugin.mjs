#!/usr/bin/env node

// Test the DeepSeek Harness host plugin (src-tauri/src/bin/pokehook/dsh-plugin.js):
//   - run the real plugin module in a child process with an isolated HOME
//   - point __POKE_HOOK_BIN__ at a fake poke-hook that records stdin payloads
//   - emit DSH session events and assert the normalized hook payloads
//
// Covers: turn/start, turn/end (completed / aborted / error), approval/asked,
// user-questions/request (Pending + back to Running), subagent filtering, and the
// quiescence gate (Stop waits for the whole session tree: child sessions, active
// goals, queued inbox) plus its fallback path when DSH services are unavailable.

import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { strict as assert } from "node:assert";

const root = mkdtempSync(join(tmpdir(), "dsh-plugin-test-"));
const home = join(root, "home");
mkdirSync(home, { recursive: true });
const logPath = join(root, "forwarded.jsonl");

const fakeHook = join(root, "fake-poke-hook.cjs");
writeFileSync(
  fakeHook,
  `#!/usr/bin/env node
const fs = require("node:fs");
let body = "";
process.stdin.on("data", (chunk) => { body += chunk; });
process.stdin.on("end", () => { fs.appendFileSync(process.env.POKEPOKE_TEST_LOG, body + "\\n"); });
`,
);
chmodSync(fakeHook, 0o755);

const pluginSource = readFileSync(
  new URL("../src-tauri/src/bin/pokehook/dsh-plugin.js", import.meta.url),
  "utf8",
).replaceAll("__POKE_HOOK_BIN__", fakeHook);
const pluginPath = join(root, "dsh-plugin.mjs");
writeFileSync(pluginPath, pluginSource);

const ctxStub = `
const handlers = new Map();
const agents = new Map();
const goals = new Map();
const log = process.env.POKEPOKE_TEST_LOG;

function makeAgent(id, status = "idle") {
  const agent = { id, status, inbox: { nextTurn: [], nextStep: [] } };
  agents.set(id, agent);
  return agent;
}

const services = {
  connection: { fetch: { register() { return () => {}; } } },
  agents: { get: (id) => agents.get(id) },
  goals: { get: (agent) => goals.get(agent.id) },
  subagents: { waitForChildren: async () => false },
};

const hookOptions = new Map();
const ctx = {
  on(name, fn, options) { handlers.set(name, fn); hookOptions.set(name, options ?? {}); },
  inject(names, cb) {
    const owner = {};
    for (const name of names) {
      if (services[name] === undefined) return;
      owner[name] = services[name];
    }
    cb(owner);
  },
  effect(fn) { fn(); },
};

apply(ctx);

const sess = (id, cwd = "/tmp/proj") => ({ header: { id, cwd } });
const emit = (type, data, target) => handlers.get("session/event")?.(target, { type, data });
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const mark = (label) => fs.appendFileSync(log, JSON.stringify({ marker: label }) + "\\n");
`;

// Main path: DSH services present, Stop must wait for the session tree to go quiet.
const driverPath = join(root, "driver.mjs");
writeFileSync(
  driverPath,
  `import fs from "node:fs";
const { apply } = await import(process.env.DSH_PLUGIN_PATH);
${ctxStub}
// cordis 按 agent scope 过滤 waterfall listener:插件被热重载后普通 listener 收不到
// ask_user_question,所以这个订阅必须带 global:true。
mark("questions-global=" + (hookOptions.get("user-questions/request")?.global === true));

const main = sess("session-test-1");
makeAgent("session-test-1");
emit("turn/start", {}, main);
emit("turn/end", { reason: { kind: "completed" } }, main);

const abortedSession = sess("session-test-2");
makeAgent("session-test-2");
emit("turn/end", { reason: { kind: "aborted", reason: { kind: "user" } } }, abortedSession);

const errorSession = sess("session-test-3");
makeAgent("session-test-3");
emit(
  "turn/end",
  { reason: { kind: "error", error: { kind: "rate_limit", message: "429 too many requests" } } },
  errorSession,
);
emit("approval/asked", { toolName: "bash" }, main);
emit("turn/start", {}, { header: { id: "session-sub-1", cwd: "/tmp", origin: "subagent" } });

// Goal 自动续轮:turn/end 后 50ms 又 turn/start。Stop 必须被取消。
const goalSession = sess("session-goal-1", "/tmp/goal");
makeAgent("session-goal-1");
emit("turn/end", { reason: { kind: "completed" } }, goalSession);
await wait(50);
emit("turn/start", {}, goalSession);

const questions = handlers.get("user-questions/request");
questions?.(
  { agent: { session: main }, questions: [{ question: "Plan ok?" }] },
  () => Promise.resolve("answered"),
);

// 后台子会话还在跑:父会话 turn/end 不算完成。
const childSession = sess("session-test-4");
makeAgent("session-test-4");
makeAgent("session-child-1", "running");
emit("subagent/catalog", { childId: "session-child-1" }, childSession);
emit("turn/end", { reason: { kind: "completed" } }, childSession);

// goal 还在 active+armed:DSH 自己还要续轮。
const pausedGoal = sess("session-test-5");
makeAgent("session-test-5");
goals.set("session-test-5", { phase: "active", activation: "armed" });
emit("turn/end", { reason: { kind: "completed" } }, pausedGoal);

// 排队待发消息:用户插话 / 子会话回话还没被 claim。
const queuedSession = sess("session-test-6");
const queuedAgent = makeAgent("session-test-6");
queuedAgent.inbox.nextTurn.push({ id: "queued-message" });
emit("turn/end", { reason: { kind: "completed" } }, queuedSession);

// 问答工具走 session/event(未过滤):tool/call 即 Pending,tool/result 恢复 Running。
// 这条路径不依赖 agent 级 waterfall,插件热重载后仍然有效。
const questionSession = sess("session-test-7");
makeAgent("session-test-7");
emit(
  "tool/call",
  {
    callId: "call-q1",
    name: "ask_user_question",
    arguments: JSON.stringify({ questions: [{ question: "Pick one?" }] }),
  },
  questionSession,
);
emit("tool/call", { callId: "call-b1", name: "bash", arguments: "{}" }, questionSession);
await wait(50);
emit("tool/result", { toolCallId: "call-q1" }, questionSession);

// 观察点:此时子会话 / goal / 排队三种会话都还不该有 Stop。
await wait(1800);
mark("gate-open");
agents.get("session-child-1").status = "idle";
goals.delete("session-test-5");
queuedAgent.inbox.nextTurn.length = 0;

await wait(2400);
process.exit(0);
`,
);

// Fallback path: no DSH services -> legacy 1.2s debounce still reports the Stop.
const fallbackPath = join(root, "driver-fallback.mjs");
writeFileSync(
  fallbackPath,
  `const { apply } = await import(process.env.DSH_PLUGIN_PATH);
const handlers = new Map();
const hookOptions = new Map();
const ctx = {
  on(name, fn, options) { handlers.set(name, fn); hookOptions.set(name, options ?? {}); },
  inject(names, cb) {
    if (names.length === 1 && names[0] === "connection") {
      cb({ connection: { fetch: { register() { return () => {}; } } } });
    }
  },
  effect(fn) { fn(); },
};
apply(ctx);
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const session = { header: { id: "session-fallback-1", cwd: "/tmp/fallback" } };
handlers.get("session/event")?.(session, { type: "turn/end", data: { reason: { kind: "completed" } } });
await wait(1800);
process.exit(0);
`,
);

try {
  const run = (path) =>
    spawnSync(process.execPath, [path], {
      env: {
        ...process.env,
        HOME: home,
        DSH_PLUGIN_PATH: pluginPath,
        POKEPOKE_TEST_LOG: logPath,
      },
      timeout: 30000,
      encoding: "utf8",
    });

  const primary = run(driverPath);
  assert.equal(primary.status, 0, `driver failed: ${primary.stderr}`);
  const fallback = run(fallbackPath);
  assert.equal(fallback.status, 0, `fallback driver failed: ${fallback.stderr}`);

  const lines = existsSync(logPath)
    ? readFileSync(logPath, "utf8").trim().split("\n").filter(Boolean)
    : [];
  const payloads = lines.map((line) => JSON.parse(line));
  const forwarded = payloads.filter((payload) => payload.marker === undefined);

  const count = (name) =>
    forwarded.filter((payload) => payload.hook_event_name === name).length;
  const of = (sessionId) => forwarded.filter((payload) => payload.session_id === sessionId);
  // Each event spawns its own poke-hook child, so completion order is not stable.
  assert.equal(
    count("UserPromptSubmit"),
    4,
    "two turn/start for the main session + one goal continuation + one answered question tool",
  );
  assert.equal(count("Stop"), 6, "main + aborted + three gated + fallback sessions -> Idle");
  assert.equal(count("StopFailure"), 1);
  assert.equal(count("PermissionRequest"), 1);
  assert.equal(count("Notification"), 2, "waterfall question + question tool");

  for (const payload of of("session-test-1")) {
    assert.equal(payload.cwd, "/tmp/proj");
  }
  assert.equal(
    of("session-goal-1").length,
    1,
    "goal continuation must cancel the pending Stop, leaving only its UserPromptSubmit",
  );
  assert.equal(of("session-goal-1")[0].hook_event_name, "UserPromptSubmit");

  const stops = forwarded.filter((payload) => payload.hook_event_name === "Stop");
  assert.equal(
    stops.filter((payload) => payload.aborted === true).length,
    1,
    "aborted turn carries aborted:true",
  );
  assert.equal(
    stops.filter((payload) => payload.aborted === true)[0].session_id,
    "session-test-2",
  );
  assert.equal(
    forwarded.find((payload) => payload.hook_event_name === "StopFailure").reason,
    "rate_limit",
  );
  assert.equal(
    forwarded.find((payload) => payload.hook_event_name === "StopFailure").session_id,
    "session-test-3",
  );
  assert.equal(
    forwarded.find((payload) => payload.hook_event_name === "PermissionRequest").tool_name,
    "bash",
  );
  assert.equal(
    of("session-test-1").find((payload) => payload.hook_event_name === "Notification").message,
    "Plan ok?",
  );
  // 问答工具的 Pending / Running 由 tool/call + tool/result 驱动。
  const questionTool = of("session-test-7");
  assert.equal(questionTool.length, 2, "question tool reports Pending then Running");
  assert.equal(questionTool[0].hook_event_name, "Notification");
  assert.equal(questionTool[0].message, "Pick one?");
  assert.equal(questionTool[1].hook_event_name, "UserPromptSubmit");
  assert.equal(count("Notification"), 2, "waterfall question + tool question");
  assert.ok(
    !payloads.some((payload) => payload.session_id === "session-sub-1"),
    "subagent sessions must be filtered out",
  );

  // 问句订阅必须是 global listener,否则热重载后收不到 ask_user_question。
  assert.ok(
    payloads.some((payload) => payload.marker === "questions-global=true"),
    "user-questions/request must be subscribed with { global: true }",
  );

  // 静默门:子会话 / goal / 排队消息任一没解除之前都不能发 Stop。
  const gateIndex = payloads.findIndex((payload) => payload.marker === "gate-open");
  assert.ok(gateIndex >= 0, "gate-open marker must be recorded");
  for (const sessionId of ["session-test-4", "session-test-5", "session-test-6"]) {
    const index = payloads.findIndex(
      (payload) => payload.session_id === sessionId && payload.hook_event_name === "Stop",
    );
    assert.ok(
      index > gateIndex,
      `${sessionId} must not report Stop while the session tree is still busy`,
    );
    assert.equal(of(sessionId).length, 1, `${sessionId} reports exactly one Stop`);
  }

  // 回落路径:没有 DSH 服务时保持旧的固定去抖。
  const fallbackStops = of("session-fallback-1").filter(
    (payload) => payload.hook_event_name === "Stop",
  );
  assert.equal(fallbackStops.length, 1, "fallback path still reports Idle");
  assert.equal(fallbackStops[0].cwd, "/tmp/fallback");

  console.log(`DSH plugin test passed (${forwarded.length} forwarded events)`);
} finally {
  rmSync(root, { recursive: true, force: true });
}
