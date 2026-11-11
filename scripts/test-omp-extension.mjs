#!/usr/bin/env node

// Test the omp extension TS logic (bin/pokehook/omp-extension.ts) by:
//   - Reading the extension source (Rust 侧 include_str! 同一文件)
//   - Running it in a node `vm` sandbox with a mocked child_process.spawn that
//     captures forward() payloads, and a fake `pi` object collecting on() handlers
//   - Driving each omp event and asserting the forwarded hook_event_name sequence
//
// omp 与 pi 的三处实质差异是这里的重点(见 omp-extension.ts 头注):
//   1. 没有 agent_settled —— settle = agent_end + willContinue !== true
//      (且靠 `"willContinue" in event` 分辨宿主:pi 的 agent_end 无该属性)
//   2. subagent 子 session 会复用同进程的扩展工厂,且 id 前 8 字符与父相同
//      → 必须按 ctx.localProtocolOptions.getSessionId() 过滤
//   3. tool_approval_requested → PermissionRequest(Pending)

import { readFileSync } from "fs";
import vm from "vm";
import { strict as assert } from "assert";

// ---- Step 1: Read the omp extension TS ----
const extPath = new URL(
  "../src-tauri/src/bin/pokehook/omp-extension.ts",
  import.meta.url,
);
let extSrc = readFileSync(extPath, "utf-8");

extSrc = extSrc.replaceAll("__POKE_HOOK_BIN__", "/tmp/test-poke-hook");

// ---- Step 2: Prepare sandbox ----
const capturedCalls = [];

function buildSandbox() {
  const modified = extSrc
    .replace(
      'import { spawn } from "node:child_process";',
      "const spawn = globalThis.__spawn;",
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
  sandbox.globalThis = sandbox;
  return { sandbox, modified };
}

let handlers = {};

async function rebuild() {
  capturedCalls.length = 0;
  handlers = {};
  const { sandbox, modified } = buildSandbox();
  const context = vm.createContext(sandbox);
  new vm.Script(`${modified}\ntypeof module.exports;`).runInContext(context);

  const factory = sandbox.module.exports;
  if (typeof factory !== "function") {
    throw new Error(
      "Extension did not set module.exports to a function, got " + typeof factory,
    );
  }

  const pi = {
    on: (event, handler) => {
      handlers[event] = handler;
    },
  };
  factory(pi);
}

/// 生产形态:omp 对同进程内的 subagent 复用**同一个已 import 的模块实例**,直接
/// 再跑一次工厂(omp 18.2.11 `bindPreparedExtensions`)。返回的 makeInstance() 每次
/// 在**同一个模块实例**上挂一份新的 handler 表,用来验证实例间互不串改。
function makeSharedModule() {
  capturedCalls.length = 0;
  const { sandbox, modified } = buildSandbox();
  const context = vm.createContext(sandbox);
  new vm.Script(`${modified}\ntypeof module.exports;`).runInContext(context);
  const factory = sandbox.module.exports;

  return function makeInstance() {
    const local = {};
    factory({
      on: (event, handler) => {
        local[event] = handler;
      },
    });
    return local;
  };
}

function calls() {
  return capturedCalls.map((c) => {
    try {
      return JSON.parse(c);
    } catch {
      return { raw: c };
    }
  });
}

const MAIN_SID = "01a0ce59-9c11-76b8-949a-e882129cb317";
const SUB_SID = "01a0ce59-a25e-71c8-89bb-a282c3e1019b"; // 与父同前 8 字符

/// omp 的 ctx:localProtocolOptions.getSessionId() 在父 session 上等于自己,
/// 在 subagent 子 session 上等于父(实测 omp 18.2.11)。
function ctx({
  mode = "tui",
  cwd = "/proj",
  sid = MAIN_SID,
  rootSid = sid,
} = {}) {
  return {
    mode,
    cwd,
    sessionManager: { getSessionId: () => sid },
    localProtocolOptions: { getSessionId: () => rootSid },
  };
}

/// omp 的 agent_end 事件对象**恒定带** willContinue 属性(值可能 undefined)。
function agentEnd({ willContinue, stopReason, errorMessage } = {}) {
  const messages = stopReason
    ? [
        {
          role: "assistant",
          stopReason,
          ...(errorMessage ? { errorMessage } : {}),
        },
      ]
    : [];
  return { type: "agent_end", messages, willContinue };
}

function assertLast(expected) {
  const c = calls().at(-1);
  assert.ok(c, "expected a forwarded call");
  for (const [k, v] of Object.entries(expected)) {
    assert.deepEqual(c[k], v, `payload.${k} mismatch: ${JSON.stringify(c)}`);
  }
  return c;
}

// ---- Step 3: Tests ----
const tests = [];

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  assert.equal(calls().length, 1);
  assertLast({
    hook_event_name: "SessionStart",
    session_id: MAIN_SID,
    cwd: "/proj",
  });
  // duplicate session_start for same id -> deduped
  await handlers["session_start"]({}, ctx());
  assert.equal(calls().length, 1, "duplicate session_start should be dropped");
  console.log("  PASS: tui session_start forwards SessionStart exactly once");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx({ mode: "print" }));
  assert.equal(calls().length, 0, "print/rpc/json runs have no pane to track");
  console.log("  PASS: print mode session_start forwards nothing");
});

tests.push(async () => {
  await rebuild();
  // subagent 子 session:localProtocolOptions 指向父 session
  await handlers["session_start"]({}, ctx({ sid: SUB_SID, rootSid: MAIN_SID }));
  assert.equal(
    calls().length,
    0,
    "subagent sub-session 与父共享 task_id,不得上报",
  );
  // 后续事件同样静默(整实例禁用)
  await handlers["before_agent_start"]({}, ctx({ sid: SUB_SID, rootSid: MAIN_SID }));
  await handlers["agent_end"](
    agentEnd({ willContinue: undefined }),
    ctx({ sid: SUB_SID, rootSid: MAIN_SID }),
  );
  assert.equal(calls().length, 0, "subagent 实例全程静默");
  console.log("  PASS: subagent sub-session forwards nothing");
});

tests.push(async () => {
  // 回归:同进程 subagent 复用同一模块实例再跑工厂(parent + sub 两个实例)。
  // 2026-09-24 实测串改:sub 实例的 session_start 曾把模块级 tracking 置 false,
  // 父 session 的 Stop / SessionEnd 全被吞 → session 卡 Running 直到 reaper。
  const makeInstance = makeSharedModule();
  const parent = makeInstance();
  await parent["session_start"]({}, ctx());
  await parent["before_agent_start"]({}, ctx());

  const sub = makeInstance();
  await sub["session_start"]({}, ctx({ sid: SUB_SID, rootSid: MAIN_SID }));
  await sub["before_agent_start"]({}, ctx({ sid: SUB_SID, rootSid: MAIN_SID }));
  await sub["agent_end"](
    agentEnd({ willContinue: undefined }),
    ctx({ sid: SUB_SID, rootSid: MAIN_SID }),
  );
  const afterSub = calls();
  assert.equal(
    afterSub.length,
    2,
    `sub 实例不得上报任何事件: ${JSON.stringify(afterSub)}`,
  );

  // 父 session 继续跑完这一轮 + 退出:Stop / SessionEnd 必须照常上报
  await parent["agent_end"](agentEnd({ willContinue: true }), ctx());
  await parent["agent_end"](agentEnd({ willContinue: undefined }), ctx());
  assertLast({ hook_event_name: "Stop", session_id: MAIN_SID });
  await parent["session_shutdown"]({}, ctx());
  assertLast({ hook_event_name: "SessionEnd", session_id: MAIN_SID });
  assert.equal(
    calls().length,
    4,
    `父实例应发出 SessionStart/UserPromptSubmit/Stop/SessionEnd: ${JSON.stringify(calls())}`,
  );
  console.log("  PASS: parent session survives a subagent factory in the same module");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"](
    {},
    {
      mode: "tui",
      cwd: "/proj",
      sessionManager: { getSessionId: () => undefined },
      localProtocolOptions: { getSessionId: () => undefined },
    },
  );
  assert.equal(calls().length, 0, "--no-session has no stable identity");
  console.log("  PASS: ephemeral session (no id) forwards nothing");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  await handlers["before_agent_start"]({}, ctx());
  assert.equal(calls().length, 2);
  assertLast({ hook_event_name: "UserPromptSubmit", session_id: MAIN_SID });
  console.log("  PASS: before_agent_start forwards UserPromptSubmit");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  // willContinue=true:已安排自动续跑,不是终态
  await handlers["agent_end"](agentEnd({ willContinue: true }), ctx());
  assert.equal(calls().length, 1, "willContinue=true 不得上报 Stop");
  // willContinue=undefined(终态)
  await handlers["agent_end"](agentEnd({ willContinue: undefined }), ctx());
  assertLast({ hook_event_name: "Stop", session_id: MAIN_SID });
  console.log("  PASS: agent_end settles only when willContinue !== true");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  // pi 宿主的 agent_end 没有 willContinue 属性 -> 能力探测为假,静默
  await handlers["agent_end"]({ type: "agent_end", messages: [] }, ctx());
  assert.equal(
    calls().length,
    1,
    "无 willContinue 属性 = 非 omp 宿主,不得上报",
  );
  console.log("  PASS: agent_end without willContinue property forwards nothing");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  await handlers["agent_end"](
    agentEnd({ willContinue: undefined, stopReason: "aborted" }),
    ctx(),
  );
  assertLast({
    hook_event_name: "Stop",
    session_id: MAIN_SID,
    aborted: true,
  });
  console.log("  PASS: user-aborted turn forwards Stop with aborted:true");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  await handlers["agent_end"](
    agentEnd({
      willContinue: undefined,
      stopReason: "error",
      errorMessage: "HTTP 429 rate limited",
    }),
    ctx(),
  );
  assertLast({
    hook_event_name: "StopFailure",
    session_id: MAIN_SID,
    reason: "provider_error",
    detail: "HTTP 429 rate limited",
  });
  console.log("  PASS: provider error forwards StopFailure with detail");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  await handlers["tool_approval_requested"](
    { type: "tool_approval_requested", toolName: "bash" },
    ctx(),
  );
  assertLast({
    hook_event_name: "PermissionRequest",
    session_id: MAIN_SID,
    tool_name: "bash",
  });
  console.log("  PASS: tool_approval_requested forwards PermissionRequest (Pending)");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  await handlers["session_shutdown"]({}, ctx());
  assert.equal(calls().length, 2);
  assertLast({ hook_event_name: "SessionEnd", session_id: MAIN_SID });
  await handlers["agent_end"](agentEnd({ willContinue: undefined }), ctx());
  assert.equal(calls().length, 2, "post-shutdown agent_end must not forward");
  console.log("  PASS: session_shutdown forwards SessionEnd and clears session");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  // 静默换 session(/new、/resume、/fork):下一个事件 ctx 已是新 session id
  const newSid = "01a0ce60-1111-2222-3333-444455556666";
  await handlers["before_agent_start"]({}, ctx({ sid: newSid, rootSid: newSid }));
  const cs = calls();
  assert.equal(cs.length, 4, JSON.stringify(cs));
  assert.equal(cs[1].hook_event_name, "SessionEnd");
  assert.equal(cs[1].session_id, MAIN_SID);
  assert.equal(cs[2].hook_event_name, "SessionStart");
  assert.equal(cs[2].session_id, newSid);
  assert.equal(cs[3].hook_event_name, "UserPromptSubmit");
  assert.equal(cs[3].session_id, newSid);
  console.log("  PASS: silent session switch closes old and registers new");
});

tests.push(async () => {
  await rebuild();
  await handlers["session_start"]({}, ctx());
  const newSid = "01a0ce61-7777-8888-9999-aaaabbbbcccc";
  await handlers["session_switch"]({}, ctx({ sid: newSid, rootSid: newSid }));
  const cs = calls();
  assert.equal(cs.length, 3, JSON.stringify(cs));
  assert.equal(cs[1].hook_event_name, "SessionEnd");
  assert.equal(cs[2].hook_event_name, "SessionStart");
  assert.equal(cs[2].session_id, newSid);
  console.log("  PASS: session_switch event re-registers the new session");
});

tests.push(async () => {
  await rebuild();
  await handlers["before_agent_start"]({}, ctx());
  assert.equal(calls().length, 0, "no session id yet -> no forward");
  console.log("  PASS: before_agent_start forwards nothing before session_start");
});

tests.push(async () => {
  await rebuild();
  const { sandbox, modified } = buildSandbox();
  sandbox.globalThis.__spawn = () => {
    throw new Error("boom");
  };
  const context = vm.createContext(sandbox);
  new vm.Script(`${modified}\ntypeof module.exports;`).runInContext(context);
  handlers = {};
  const pi = { on: (e, h) => (handlers[e] = h) };
  sandbox.module.exports(pi);
  await handlers["session_start"]({}, ctx());
  assert.equal(calls().length, 0, "spawn threw -> forward swallowed");
  console.log("  PASS: spawn failure is swallowed, not propagated");
});

// ---- Step 4: Run ----
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
