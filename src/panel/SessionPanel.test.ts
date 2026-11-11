import { describe, expect, test } from "vitest";
import type { Session } from "../types";
import {
  isActive,
  projectName,
  sortSessionsForPanel,
  sourceLabel,
  workspacePath,
} from "./sessionPanelHelpers";

function session(overrides: Partial<Session> = {}): Session {
  return {
    id: "session-1",
    task_id: "cc-123",
    external_session_id: "cc-session-123",
    title: "Claude Code: my-proj",
    message: "Working...",
    source: "claude-code",
    priority: "normal",
    status: "running",
    created_at: "2026-04-17T00:00:00Z",
    updated_at: "2026-04-17T00:00:00Z",
    terminal_tty: "/dev/ttys001",
    workspace_path: "/Users/tester/my-proj",
    failure_reason: null,
    terminal_id: null,
    terminal_id_captured_cwd: null,
    transcript_path: null,
    ...overrides,
  };
}

describe("SessionPanel helpers", () => {
  test("projectName 从 title 中提取项目名", () => {
    expect(projectName(session())).toBe("my-proj");
  });

  test("projectName 在无前缀时回退原 title", () => {
    expect(projectName(session({ title: "plain" }))).toBe("plain");
  });

  test("sourceLabel 映射已知来源", () => {
    expect(sourceLabel(session({ source: "claude-code" }))).toBe("Claude Code");
    expect(sourceLabel(session({ source: "cursor" }))).toBe("Cursor");
    // session() 默认 terminal_tty 有值 -> codex CLI 显示 Codex
    expect(sourceLabel(session({ source: "codex" }))).toBe("Codex");
    expect(sourceLabel(session({ source: "opencode" }))).toBe("opencode");
    expect(sourceLabel(session({ source: "pi" }))).toBe("Pi");
  });

  test("sourceLabel codex 桌面端(无 tty)显示 ChatGPT", () => {
    expect(sourceLabel(session({ source: "codex", terminal_tty: null }))).toBe("ChatGPT");
  });

  test("sourceLabel 对空值返回空字符串", () => {
    expect(sourceLabel(session({ source: null }))).toBe("");
  });

  test("workspacePath 将用户目录缩写为 ~", () => {
    expect(workspacePath(session())).toBe("~/my-proj");
  });

  test("isActive 对 running 和 pending 返回 true", () => {
    expect(isActive(session({ status: "running" }))).toBe(true);
    expect(isActive(session({ status: "pending" }))).toBe(true);
  });

  test("isActive 对 idle 和 last_failed 返回 false", () => {
    expect(isActive(session({ status: "idle" }))).toBe(false);
    expect(isActive(session({ status: "last_failed" }))).toBe(false);
  });

  test("sortSessionsForPanel 将 running/pending 置顶", () => {
    const idle = session({
      id: "idle",
      status: "idle",
      created_at: "2026-04-17T10:00:00Z",
    });
    const running = session({
      id: "running",
      status: "running",
      created_at: "2026-04-17T08:00:00Z",
    });
    const pending = session({
      id: "pending",
      status: "pending",
      created_at: "2026-04-17T09:00:00Z",
    });
    const failed = session({
      id: "failed",
      status: "last_failed",
      created_at: "2026-04-17T07:00:00Z",
    });

    expect(sortSessionsForPanel([idle, running, failed, pending]).map((s) => s.id)).toEqual([
      "running",
      "pending",
      "failed",
      "idle",
    ]);
  });

  test("sortSessionsForPanel 在同组内保持 created_at 升序", () => {
    const older = session({
      id: "older",
      status: "idle",
      created_at: "2026-04-17T08:00:00Z",
    });
    const newer = session({
      id: "newer",
      status: "idle",
      created_at: "2026-04-17T10:00:00Z",
    });

    expect(sortSessionsForPanel([newer, older]).map((s) => s.id)).toEqual(["older", "newer"]);
  });
});
