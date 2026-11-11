import { describe, expect, test } from "vitest";
import {
  DEFAULT_POPUP_HOTKEY_MODIFIERS,
  DEFAULT_QUIET_MODE_SHORTCUT,
  formatFullShortcut,
  formatPopupShortcutModPrefix,
  FullShortcutCaptureSession,
  HotkeyCaptureSession,
} from "./shortcutModifiers";

function keyEvent(
  type: "keydown" | "keyup",
  init: Partial<KeyboardEvent> & { key: string },
): KeyboardEvent {
  return {
    type,
    code: init.code ?? init.key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    ...init,
  } as KeyboardEvent;
}

describe("HotkeyCaptureSession", () => {
  test("commits command on modifier keyup", () => {
    const session = new HotkeyCaptureSession();
    expect(session.onKeyDown(keyEvent("keydown", { key: "Meta", metaKey: true }))).toEqual({
      type: "ignore",
    });
    expect(session.onKeyUp(keyEvent("keyup", { key: "Meta", metaKey: false }))).toEqual({
      type: "commit",
      modifiers: "CmdOrCtrl",
    });
  });

  test("commits command+shift when shift is pressed while command is held", () => {
    const session = new HotkeyCaptureSession();
    session.onKeyDown(keyEvent("keydown", { key: "Meta", metaKey: true }));
    expect(
      session.onKeyDown(keyEvent("keydown", { key: "Shift", metaKey: true, shiftKey: true })),
    ).toEqual({
      type: "commit",
      modifiers: "CmdOrCtrl+Shift",
    });
  });

  test("commits control when control plus a digit is pressed", () => {
    const session = new HotkeyCaptureSession();
    expect(
      session.onKeyDown(keyEvent("keydown", { key: "1", ctrlKey: true, code: "Digit1" })),
    ).toEqual({
      type: "commit",
      modifiers: "Control",
    });
  });

  test("distinguishes command and control", () => {
    const session = new HotkeyCaptureSession();
    session.onKeyDown(keyEvent("keydown", { key: "Meta", metaKey: true }));
    expect(session.onKeyUp(keyEvent("keyup", { key: "Meta", metaKey: false }))).toEqual({
      type: "commit",
      modifiers: "CmdOrCtrl",
    });

    const session2 = new HotkeyCaptureSession();
    session2.onKeyDown(keyEvent("keydown", { key: "Control", ctrlKey: true }));
    expect(session2.onKeyUp(keyEvent("keyup", { key: "Control", ctrlKey: false }))).toEqual({
      type: "commit",
      modifiers: "Control",
    });
  });

  test("ignores plain keys without modifiers", () => {
    const session = new HotkeyCaptureSession();
    expect(session.onKeyDown(keyEvent("keydown", { key: "a", code: "KeyA" }))).toEqual({
      type: "ignore",
    });
    expect(session.onKeyUp(keyEvent("keyup", { key: "a", code: "KeyA" }))).toEqual({
      type: "ignore",
    });
  });

  test("cancels on escape", () => {
    const session = new HotkeyCaptureSession();
    expect(session.onKeyDown(keyEvent("keydown", { key: "Escape" }))).toEqual({
      type: "cancel",
    });
  });

  test("ignores auto-repeat keydown", () => {
    const session = new HotkeyCaptureSession();
    expect(
      session.onKeyDown(
        keyEvent("keydown", { key: "1", metaKey: true, repeat: true, code: "Digit1" }),
      ),
    ).toEqual({
      type: "ignore",
    });
  });
});

describe("formatPopupShortcutModPrefix", () => {
  test("maps known tokens", () => {
    expect(formatPopupShortcutModPrefix("CmdOrCtrl")).toBe("\u2318");
    expect(formatPopupShortcutModPrefix("CmdOrCtrl+Shift")).toBe("\u2318\u21e7");
    expect(formatPopupShortcutModPrefix("Control+Alt")).toBe("\u2303\u2325");
  });

  test("falls back to command glyph", () => {
    expect(formatPopupShortcutModPrefix("unknown")).toBe("\u2318");
  });

  test("default modifiers stay command", () => {
    expect(DEFAULT_POPUP_HOTKEY_MODIFIERS).toBe("CmdOrCtrl");
  });
});

describe("HotkeyCaptureSession minimum-modifier guard", () => {
  test("rejects pure Shift on keyup", () => {
    const session = new HotkeyCaptureSession();
    session.onKeyDown(keyEvent("keydown", { key: "Shift", shiftKey: true }));
    expect(session.onKeyUp(keyEvent("keyup", { key: "Shift", shiftKey: false }))).toEqual({
      type: "ignore",
    });
  });

  test("rejects Shift+Alt combo", () => {
    const session = new HotkeyCaptureSession();
    session.onKeyDown(keyEvent("keydown", { key: "Shift", shiftKey: true }));
    expect(
      session.onKeyDown(keyEvent("keydown", { key: "Alt", shiftKey: true, altKey: true })),
    ).toEqual({ type: "ignore" });
  });

  test("rejects Shift + plain key", () => {
    const session = new HotkeyCaptureSession();
    expect(
      session.onKeyDown(keyEvent("keydown", { key: "1", shiftKey: true, code: "Digit1" })),
    ).toEqual({ type: "ignore" });
  });
});

describe("FullShortcutCaptureSession", () => {
  test("commits Cmd + Semicolon", () => {
    const session = new FullShortcutCaptureSession();
    const r = session.onKeyDown(
      keyEvent("keydown", { key: ";", code: "Semicolon", metaKey: true }),
    );
    expect(r).toEqual({ type: "commit", shortcut: "CmdOrCtrl+Semicolon" });
  });

  test("commits Ctrl+Shift+K", () => {
    const session = new FullShortcutCaptureSession();
    const r = session.onKeyDown(
      keyEvent("keydown", {
        key: "k",
        code: "KeyK",
        ctrlKey: true,
        shiftKey: true,
      }),
    );
    expect(r).toEqual({ type: "commit", shortcut: "Control+Shift+KeyK" });
  });

  test("rejects Shift+K (no Cmd/Ctrl)", () => {
    const session = new FullShortcutCaptureSession();
    expect(
      session.onKeyDown(keyEvent("keydown", { key: "k", code: "KeyK", shiftKey: true })),
    ).toEqual({ type: "ignore" });
  });

  test("ignores modifier-only keydown", () => {
    const session = new FullShortcutCaptureSession();
    expect(session.onKeyDown(keyEvent("keydown", { key: "Meta", metaKey: true }))).toEqual({
      type: "ignore",
    });
  });

  test("cancels on Escape", () => {
    const session = new FullShortcutCaptureSession();
    expect(session.onKeyDown(keyEvent("keydown", { key: "Escape" }))).toEqual({
      type: "cancel",
    });
  });
});

describe("formatFullShortcut", () => {
  test("default maps to Cmd + semicolon", () => {
    expect(DEFAULT_QUIET_MODE_SHORTCUT).toBe("CmdOrCtrl+Semicolon");
    expect(formatFullShortcut("CmdOrCtrl+Semicolon")).toBe("\u2318;");
  });

  test("Control+Shift+KeyK renders glyphs + K", () => {
    expect(formatFullShortcut("Control+Shift+KeyK")).toBe("\u2303\u21e7K");
  });

  test("digit code maps to digit", () => {
    expect(formatFullShortcut("CmdOrCtrl+Digit3")).toBe("\u23183");
  });
});
