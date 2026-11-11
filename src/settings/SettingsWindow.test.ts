import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { WHITE_TRAY_ICON_IMAGE_CLASS } from "./SettingsWindow";

describe("SettingsWindow styles", () => {
  test("settings sidebar tabs avoid broad layout transitions", () => {
    const source = readFileSync(new URL("./SettingsWindow.tsx", import.meta.url), "utf8");

    expect(source).not.toContain("text-sm transition-all");
    expect(source).toContain("border border-transparent");
    expect(source).toContain("transition-[background-color,border-color,box-shadow,color]");
  });

  test("white tray icon stays in a normal settings control with subtle image contrast", () => {
    expect(WHITE_TRAY_ICON_IMAGE_CLASS).toContain("drop-shadow-[0_1px_1px_rgba(15,15,35,0.45)]");
    expect(WHITE_TRAY_ICON_IMAGE_CLASS).not.toContain("bg-[#171724]");
  });
});
