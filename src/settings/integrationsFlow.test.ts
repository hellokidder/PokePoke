// IntegrationsTab 编排状态机转移契约表驱动测试(W11,brief
// 20260813-integrations-flow-reducer)。纯 vitest,无 DOM 库。

import { describe, expect, it } from "vitest";
import {
  canAct,
  flowInitialState,
  flowReducer,
  type IntegrationsFlowAction,
  type IntegrationsFlowState,
  i18nMessage,
  rawMessage,
} from "./integrationsFlow";
import type { IntegrationSource, ManagedIntegrationStatus } from "./integrationsLogic";

function row(source: IntegrationSource, overrides: Partial<ManagedIntegrationStatus> = {}) {
  return {
    source,
    agent_present: true,
    connected: true,
    hooks_configured: true,
    hooks_outdated: false,
    installed: true,
    issue: null,
    repair_available: false,
    ...overrides,
  } as ManagedIntegrationStatus;
}

/** 按 action 序列逐步 reducer 并返回最终状态。 */
function run(actions: IntegrationsFlowAction[]): IntegrationsFlowState {
  return actions.reduce(flowReducer, flowInitialState());
}

/** 除某行外其余行全部 busy 的状态(互斥断言用)。 */
function busyElsewhere(state: IntegrationsFlowState, source: IntegrationSource) {
  const busy = { ...state.rowBusy };
  for (const s of Object.keys(busy) as IntegrationSource[]) {
    if (s !== source) busy[s] = "connect";
  }
  return { ...state, rowBusy: busy };
}

describe("refresh 生命周期", () => {
  it("初始 loading=true,首轮 rows_loaded + initial_loaded 后 loading=false 且 rows 落地", () => {
    const init = flowInitialState();
    expect(init.loading).toBe(true);
    const state = flowReducer(
      flowReducer(init, {
        type: "rows_loaded",
        rows: [row("cc")],
      }),
      { type: "initial_loaded" },
    );
    expect(state.loading).toBe(false);
    expect(state.rows).toHaveLength(1);
  });

  it("refresh_started → refresh_done:refreshingAll 置位后清除 + check_all_done 提示 + rows 替换", () => {
    const state = run([
      { type: "refresh_started" },
      { type: "refresh_done", rows: [row("cc"), row("codex")] },
    ]);
    expect(state.refreshingAll).toBe(false);
    expect(state.message).toEqual(i18nMessage("settings.integrations.check_all_done"));
    expect(state.rows).toHaveLength(2);
  });

  it("refresh_started → refresh_failed:refreshingAll 清除 + action_failed 提示", () => {
    const state = run([{ type: "refresh_started" }, { type: "refresh_failed" }]);
    expect(state.refreshingAll).toBe(false);
    expect(state.message).toEqual(i18nMessage("settings.integrations.action_failed"));
  });
});

describe("action 互斥", () => {
  it("canAct:默认全行可操作;refreshingAll 时全部禁用", () => {
    const idle = flowInitialState();
    expect(canAct(idle, "cc")).toBe(true);
    expect(canAct(idle, "grok")).toBe(true);
    const refreshing = flowReducer(idle, { type: "refresh_started" });
    expect(canAct(refreshing, "cc")).toBe(false);
  });

  it("canAct:该行 busy 时禁用,其它行不受影响(现状 per-row 互斥)", () => {
    const state = run([{ type: "action_started", source: "cc", kind: "connect" }]);
    expect(canAct(state, "cc")).toBe(false);
    expect(canAct(state, "codex")).toBe(true);
  });

  it("action_started 置位 → action_done 清位", () => {
    const state = run([
      { type: "action_started", source: "cc", kind: "repair" },
      { type: "action_done", source: "cc", message: null },
    ]);
    expect(state.rowBusy.cc).toBeNull();
    expect(canAct(state, "cc")).toBe(true);
  });

  it("action_started 清 message,action_done 携带结果提示", () => {
    const withMsg = flowReducer(flowInitialState(), {
      type: "action_done",
      source: "cc",
      message: rawMessage("backend says no"),
    });
    const state = run([
      { type: "action_started", source: "cc", kind: "connect" },
      { type: "action_done", source: "cc", message: rawMessage("backend says no") },
    ]);
    expect(state.message).toEqual(rawMessage("backend says no"));
    expect(withMsg.message).toEqual(rawMessage("backend says no"));
  });
});

describe("connect/repair/disconnect 分支", () => {
  it("action_failed:busy 清除 + action_failed 提示", () => {
    const state = run([
      { type: "action_started", source: "codex", kind: "connect" },
      { type: "action_failed", source: "codex" },
    ]);
    expect(state.rowBusy.codex).toBeNull();
    expect(state.message).toEqual(i18nMessage("settings.integrations.action_failed"));
  });

  it("success 无提示分支:action_done(message=null) 只清 busy 不清既有 rows", () => {
    const state = run([
      { type: "rows_loaded", rows: [row("cc")] },
      { type: "action_started", source: "cc", kind: "repair" },
      { type: "action_done", source: "cc", message: null },
    ]);
    expect(state.rowBusy.cc).toBeNull();
    expect(state.message).toBeNull();
    expect(state.rows).toHaveLength(1);
  });

  it("disconnect 成功带 sessions_removed:i18n 提示携带 n 参数", () => {
    const state = run([
      { type: "action_started", source: "cursor", kind: "disconnect" },
      {
        type: "action_done",
        source: "cursor",
        message: i18nMessage("settings.integrations.disconnect_cleared_sessions", { n: 3 }),
      },
    ]);
    expect(state.message).toEqual(
      i18nMessage("settings.integrations.disconnect_cleared_sessions", { n: 3 }),
    );
  });
});

describe("disconnect confirm 流", () => {
  it("confirm_open → confirm_cancel:弹窗开 → 关,rows/busy 不受影响", () => {
    const state = run([
      { type: "rows_loaded", rows: [row("cc")] },
      { type: "confirm_open", source: "cc" },
      { type: "confirm_cancel" },
    ]);
    expect(state.confirmSource).toBeNull();
    expect(state.rows).toHaveLength(1);
  });

  it("confirm_open → confirm_cancel + action_started(disconnect):确认后执行断开", () => {
    const state = run([
      { type: "confirm_open", source: "cc" },
      { type: "confirm_cancel" },
      { type: "action_started", source: "cc", kind: "disconnect" },
    ]);
    expect(state.confirmSource).toBeNull();
    expect(state.rowBusy.cc).toBe("disconnect");
  });
});

describe("drift 事件", () => {
  it("drift_detected:rows 替换且不打断 busy/message/confirm", () => {
    const busy = busyElsewhere(flowInitialState(), "cc");
    const withMsg = flowReducer(busy, {
      type: "action_done",
      source: "cc",
      message: rawMessage("x"),
    });
    const state = flowReducer(withMsg, {
      type: "drift_detected",
      rows: [row("grok", { connected: false })],
    });
    expect(state.rows[0].source).toBe("grok");
    // 其它行 busy 保持(现状 loadAll 不碰 busy)
    expect(state.rowBusy.codex).toBe("connect");
    expect(state.message).toEqual(rawMessage("x"));
  });

  it("drift_repaired(带 rows):同 drift_detected 语义", () => {
    const state = run([
      { type: "action_started", source: "cc", kind: "repair" },
      { type: "drift_repaired", source: "cc", rows: [row("cc", { hooks_outdated: false })] },
    ]);
    expect(state.rows[0].hooks_outdated).toBe(false);
    expect(state.rowBusy.cc).toBe("repair");
  });

  it("drift_repaired(不带 rows):no-op,不打断任何状态", () => {
    const state = run([
      { type: "action_started", source: "cc", kind: "repair" },
      { type: "drift_repaired", source: "cc" },
    ]);
    expect(state.rowBusy.cc).toBe("repair");
  });
});

describe("message 生命周期", () => {
  it("展示 → message_clear 清空", () => {
    const withMsg = flowReducer(flowInitialState(), {
      type: "action_done",
      source: "cc",
      message: rawMessage("y"),
    });
    const cleared = flowReducer(withMsg, { type: "message_clear" });
    expect(cleared.message).toBeNull();
  });

  it("action_started / refresh_started 自动清 message(现状动作入口清提示)", () => {
    const withMsg = flowReducer(flowInitialState(), {
      type: "action_done",
      source: "cc",
      message: rawMessage("y"),
    });
    const started = flowReducer(withMsg, { type: "action_started", source: "cc", kind: "connect" });
    expect(started.message).toBeNull();
    const refreshed = flowReducer(started, { type: "refresh_started" });
    expect(refreshed.message).toBeNull();
  });
});
