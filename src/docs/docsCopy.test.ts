import { describe, expect, it } from "vitest";
import { getDocsCopy } from "./docsCopy";

describe("docsCopy", () => {
  it("EN/ZH 结构对齐(节 id 与编号一致)", () => {
    const en = getDocsCopy("en");
    const zh = getDocsCopy("zh");
    expect(en.sections.map((s) => s.id)).toEqual(zh.sections.map((s) => s.id));
    expect(en.sections.map((s) => s.index)).toEqual(zh.sections.map((s) => s.index));
  });

  it("herdr 远程模式一节保留关键事实(与 herdr_remote.rs 头注释一致)", () => {
    for (const locale of ["en", "zh"] as const) {
      const section = getDocsCopy(locale).sections.find((s) => s.id === "herdr-remote");
      expect(section).toBeDefined();
      const text = JSON.stringify(section);
      // 零配置自动发现:复用 herdr --remote 已有 SSH 连接
      expect(text).toContain("herdr --remote");
      // 状态映射:working→不弹 / blocked→弹 pending / done·idle→弹
      expect(text).toContain("working");
      expect(text).toContain("blocked");
    }
  });

  it("7 个手册节齐全", () => {
    expect(getDocsCopy("en").sections).toHaveLength(7);
    expect(getDocsCopy("zh").sections).toHaveLength(7);
  });
});
