import { describe, expect, test } from "vitest";
import type { SessionStatus } from "../types";
import { getLandingCopy } from "./landingCopy";

describe("landing copy", () => {
  test("returns English landing copy", () => {
    const copy = getLandingCopy("en");

    expect(copy.hero.title).toBe("Never miss the moment your coding agent needs you.");
    expect(copy.nav.download).toBe("Download");
    expect(copy.language.current).toBe("EN");
  });

  test("returns Chinese landing copy", () => {
    const copy = getLandingCopy("zh");

    expect(copy.hero.title).toBe("别再错过 coding agent 需要你的时刻。");
    expect(copy.nav.download).toBe("下载");
    expect(copy.language.current).toBe("中文");
  });

  test("uses multiple real product demo sessions", () => {
    const copy = getLandingCopy("en");
    const statuses = copy.hero.demo.sessions.map(
      (session) => session.status satisfies SessionStatus,
    );

    expect(copy.hero.demo.sessions).toHaveLength(4);
    expect(statuses).toEqual(["running", "pending", "idle", "last_failed"]);
    expect(copy.hero.demo.sessions.map((session) => session.project)).toEqual([
      "atlas",
      "northwind",
      "orbit",
      "release-kit",
    ]);
  });

  test("uses safe example paths and no fake progress copy", () => {
    const copy = getLandingCopy("en");

    expect(
      copy.hero.demo.sessions.every((session) => session.workspace.startsWith("~/Projects/")),
    ).toBe(true);
    expect(JSON.stringify(copy.hero.demo)).not.toContain("poke-poke");
    expect(JSON.stringify(copy.hero.demo)).not.toContain("starflow");
    expect(JSON.stringify(copy.hero.demo)).not.toContain("bbc");
    expect(JSON.stringify(copy.hero.demo)).not.toContain("Hungry");
    expect(JSON.stringify(copy.hero.demo)).not.toContain("hs-code-keeper");
    expect(JSON.stringify(copy.hero.demo)).not.toContain("progress");
  });

  test("uses distinct color seeds for a more colorful hero demo", () => {
    const copy = getLandingCopy("en");
    const seeds = copy.hero.demo.sessions.map((session) => session.colorSeed);

    expect(new Set(seeds).size).toBe(copy.hero.demo.sessions.length);
    expect(seeds).toEqual(["octo-violet", "octo-mint", "octo-sky", "octo-lime"]);
    expect(copy.hero.demo.popup.colorSeed).toBe("octo-mint");
  });

  test("terminal jump lists herdr and cmux alongside classic terminals", () => {
    const en = getLandingCopy("en");
    const zh = getLandingCopy("zh");

    for (const copy of [en, zh]) {
      expect(copy.terminalJump.terminals).toEqual(
        expect.arrayContaining(["herdr", "cmux", "WezTerm", "Ghostty 1.3+", "Otty", "Orca"]),
      );
      expect(copy.terminalJump.terminals).not.toEqual(
        expect.arrayContaining(["Cursor", "OpenCode", "ChatGPT"]),
      );
      expect(copy.terminalJump.categories).toHaveLength(1);
      expect(copy.terminalJump.categories[0].items).toEqual(
        expect.arrayContaining(["iTerm2", "herdr", "cmux"]),
      );
      expect(copy.integrations.items.find((item) => item.name === "Codex")).toBeTruthy();
      expect(copy.integrations.items.find((item) => item.name === "OpenCode")).toBeTruthy();
      expect(copy.integrations.items.find((item) => item.name === "pi")).toBeTruthy();
      expect(copy.integrations.items.find((item) => item.name === "kimi")).toBeTruthy();
      expect(copy.integrations.items.find((item) => item.name === "Grok Build")).toBeTruthy();
      expect(copy.integrations.moreLabel).toBeTruthy();
      expect(
        copy.hero.demo.sessions.find((session) => session.sourcePill === "CHATGPT"),
      ).toBeTruthy();
    }
  });

  test("compare section focuses on popup persistence as the core differentiator", () => {
    const en = getLandingCopy("en");
    const zh = getLandingCopy("zh");

    // Both locales have the compare section
    expect(en.compare.title).toBeTruthy();
    expect(zh.compare.title).toBeTruthy();

    // Each locale has exactly 3 talking points
    expect(en.compare.points).toHaveLength(3);
    expect(zh.compare.points).toHaveLength(3);

    // Every point has content
    for (const p of [...en.compare.points, ...zh.compare.points]) {
      expect(p).toBeTruthy();
    }

    // Footer exists in both locales
    expect(en.compare.footer).toBeTruthy();
    expect(zh.compare.footer).toBeTruthy();

    // Nav includes compare link
    expect(en.nav.compare).toBeTruthy();
    expect(zh.nav.compare).toBeTruthy();

    // Copy acknowledges competitors respectfully — no trash talk
    const allCopy = JSON.stringify([en.compare, zh.compare]);
    expect(allCopy).not.toMatch(/worse|inferior|bad choice|wrong/i);

    // Core message is about popup persistence / not missing moments
    expect(allCopy).toMatch(/popup|弹窗|stays|不消失|persist|miss/);
  });

  test("sticky poke copy exists in both locales and matches the persistent-popup metaphor", () => {
    const en = getLandingCopy("en");
    const zh = getLandingCopy("zh");

    for (const copy of [en, zh]) {
      expect(copy.stickyPoke.status).toBe("pending" satisfies SessionStatus);
      expect(copy.stickyPoke.title).toBeTruthy();
      expect(copy.stickyPoke.jump).toBeTruthy();
      expect(copy.stickyPoke.badge).toBe("⌘N");
      expect(copy.stickyPoke.closeLabel).toBeTruthy();
      // 注释必须点明"不点不走",这是 StickyPoke 交互存在的理由
      expect(copy.stickyPoke.note).toMatch(/doesn't dismiss|不点不走/);
    }
  });
});
