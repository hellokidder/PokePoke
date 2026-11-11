import { describe, expect, test } from "vitest";
import { getExpression, hashColor } from "./SourceIcon";

describe("SourceIcon helpers", () => {
  test("hashColor 对同一 seed 结果稳定", () => {
    expect(hashColor("cc-123")).toBe(hashColor("cc-123"));
  });

  test("hashColor 输出 hsl 格式", () => {
    expect(hashColor("cursor-abc")).toMatch(/^hsl\(\d+(\.\d+)?, 70%, 62%\)$/);
  });

  test("pending 表情包含白色眼睛像素", () => {
    const expression = getExpression("pending");
    expect(expression).toContainEqual([5, 7, "#FFFFFF"]);
    expect(expression).toContainEqual([12, 8, "#FFFFFF"]);
  });

  test("idle 和 last_failed 使用不同的关键表情像素", () => {
    const idle = getExpression("idle");
    const failed = getExpression("last_failed");

    expect(idle).toContainEqual([5, 7, "#1a1a2e"]);
    expect(idle).toContainEqual([10, 11, "#1a1a2e"]);
    expect(failed).toContainEqual([6, 8, "#1a1a2e"]);
    expect(failed).toContainEqual([6, 12, "#1a1a2e"]);
  });
});
