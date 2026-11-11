/** popup 提示热键前缀修饰键,与后端 `Settings.popup_shortcut_modifiers` 对齐。 */
export const DEFAULT_POPUP_HOTKEY_MODIFIERS = "CmdOrCtrl";

/** 静谧模式热键,与后端 `Settings.quiet_mode_shortcut` 对齐(完整组合)。 */
export const DEFAULT_QUIET_MODE_SHORTCUT = "CmdOrCtrl+Semicolon";

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "Shift", "OS"]);

const MODIFIER_CODES = new Set([
  "MetaLeft",
  "MetaRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "OSLeft",
  "OSRight",
]);

const MODIFIER_GLYPHS: Record<string, string> = {
  CmdOrCtrl: "\u2318",
  Control: "\u2303",
  Alt: "\u2325",
  Shift: "\u21e7",
};

export type HotkeyCaptureResult =
  | { type: "commit"; modifiers: string }
  | { type: "cancel" }
  | { type: "ignore" };

export type FullShortcutCaptureResult =
  | { type: "commit"; shortcut: string }
  | { type: "cancel" }
  | { type: "ignore" };

/** 判断修饰键集合是否至少含 CmdOrCtrl / Control 之一,与后端 `parse_full_shortcut` 一致。 */
export function modifiersMeetMinimum(parts: string[]): boolean {
  return parts.includes("CmdOrCtrl") || parts.includes("Control");
}

/** 从 KeyboardEvent 提取修饰键 token 列表(固定顺序,便于稳定序列化)。 */
export function buildModifierParts(e: KeyboardEvent): string[] {
  const parts: string[] = [];
  if (e.metaKey) parts.push("CmdOrCtrl");
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  return parts;
}

export function isModifierKeyEvent(e: KeyboardEvent): boolean {
  return MODIFIER_KEYS.has(e.key) || MODIFIER_CODES.has(e.code);
}

/**
 * 录制会话状态机:
 * - 按住 ≥2 个修饰键时,第二个修饰键按下即确认(如 ⌘+⇧)
 * - 按住修饰键 + 普通键时立即确认(如 ⌘+1,只存 ⌘)
 * - 只按一个修饰键时,松开后确认(如只按 ⌘ 再松开)
 */
export class HotkeyCaptureSession {
  private peak: string | null = null;

  reset() {
    this.peak = null;
  }

  onKeyDown(e: KeyboardEvent): HotkeyCaptureResult {
    if (e.repeat) return { type: "ignore" };
    if (e.key === "Escape") return { type: "cancel" };

    const parts = buildModifierParts(e);
    if (parts.length === 0) return { type: "ignore" };

    const current = parts.join("+");
    this.peak = current;

    if (!isModifierKeyEvent(e)) {
      // 普通键 + 修饰键:必须含 CmdOrCtrl/Control,否则忽略(和后端最小修饰键校验一致)
      if (!modifiersMeetMinimum(parts)) return { type: "ignore" };
      return { type: "commit", modifiers: current };
    }

    if (parts.length >= 2 && modifiersMeetMinimum(parts)) {
      return { type: "commit", modifiers: current };
    }

    return { type: "ignore" };
  }

  onKeyUp(e: KeyboardEvent): HotkeyCaptureResult {
    if (e.key === "Escape") return { type: "cancel" };

    const parts = buildModifierParts(e);
    if (parts.length === 0 && this.peak) {
      const committed = this.peak;
      this.peak = null;
      const peakParts = committed.split("+");
      // 松手 commit 也要满足最小修饰键;否则丢弃这次录入
      if (!modifiersMeetMinimum(peakParts)) return { type: "ignore" };
      return { type: "commit", modifiers: committed };
    }

    if (parts.length > 0) {
      this.peak = parts.join("+");
    }

    return { type: "ignore" };
  }
}

/** @deprecated 用 HotkeyCaptureSession;保留给简单单测场景。 */
export function modifiersFromKeyboardEvent(e: KeyboardEvent): string | null {
  const session = new HotkeyCaptureSession();
  const down = session.onKeyDown(e);
  if (down.type === "commit") return down.modifiers;
  return null;
}

/**
 * 从 `KeyboardEvent.code` 提取"末尾 key token"。后端 `parse_full_shortcut` 会把
 * 末尾 token 直接丢给 `Code::from_str`(keyboard-types),这里直接返回 code 保持一致。
 * 只接受非修饰键;返回 null 表示不适合作为 key(纯修饰键 / 特殊功能键在调用方过滤)。
 */
export function keyCodeFromEvent(e: KeyboardEvent): string | null {
  if (isModifierKeyEvent(e)) return null;
  if (!e.code) return null;
  return e.code;
}

/** 把 `KeyboardEvent.code` 转成人眼可读的一小段(角标 / 设置面板)。 */
export function formatShortcutKeyLabel(code: string): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  const map: Record<string, string> = {
    Semicolon: ";",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backslash: "\\",
    Quote: "'",
    Backquote: "`",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Enter: "\u23CE",
    Space: "␣",
    Tab: "\u21E5",
  };
  return map[code] ?? code;
}

/**
 * 完整 shortcut 录入(modifiers + key)。与 `HotkeyCaptureSession` 的区别:
 * - 必须按下一个非修饰键(key code)才 commit
 * - 修饰键组合必须含 CmdOrCtrl / Control(与后端一致)
 * 按 Escape 取消。
 */
export class FullShortcutCaptureSession {
  onKeyDown(e: KeyboardEvent): FullShortcutCaptureResult {
    if (e.repeat) return { type: "ignore" };
    if (e.key === "Escape") return { type: "cancel" };
    if (isModifierKeyEvent(e)) return { type: "ignore" };

    const parts = buildModifierParts(e);
    if (!modifiersMeetMinimum(parts)) return { type: "ignore" };

    const key = keyCodeFromEvent(e);
    if (!key) return { type: "ignore" };

    return { type: "commit", shortcut: [...parts, key].join("+") };
  }

  onKeyUp(_e: KeyboardEvent): FullShortcutCaptureResult {
    return { type: "ignore" };
  }
}

/** 把 `CmdOrCtrl+Semicolon` 转成 `⌘;` 供 UI 展示。 */
export function formatFullShortcut(raw: string): string {
  const parts = raw
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  if (parts.length < 2) return formatPopupShortcutModPrefix(raw);
  const key = parts[parts.length - 1];
  const mods = parts.slice(0, -1).join("+");
  return `${formatPopupShortcutModPrefix(mods)}${formatShortcutKeyLabel(key)}`;
}

/** 把 `CmdOrCtrl+Shift` 转成 `⌘⇧`,供 popup 角标用。 */
export function formatPopupShortcutModPrefix(raw: string): string {
  const parts = raw
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  const glyphs = parts
    .map((part) => MODIFIER_GLYPHS[part])
    .filter((glyph): glyph is string => Boolean(glyph));
  return glyphs.length > 0 ? glyphs.join("") : "\u2318";
}
