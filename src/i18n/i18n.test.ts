import { describe, expect, test } from "vitest";
import { translate } from "./context";
import { strings } from "./strings";

describe("i18n helpers", () => {
  test("zh 和 en 的 key 集合一致", () => {
    const zhKeys = Object.keys(strings.zh).sort();
    const enKeys = Object.keys(strings.en).sort();

    expect(enKeys).toEqual(zhKeys);
  });

  test("translate 正确替换模板变量", () => {
    expect(translate("zh", "panel.sessions", { n: 3 })).toBe("3 个会话");
    expect(translate("en", "panel.sessions", { n: 2 })).toBe("2 sessions");
  });

  test("translate 对不存在的 key 返回 key 本身", () => {
    expect(translate("zh", "missing.key")).toBe("missing.key");
  });
});
