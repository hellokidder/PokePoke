export type PopupPosition = "top-right" | "top-left" | "bottom-right" | "bottom-left";
export type PopupKind = "session" | "update";

export const VALID_POSITIONS: PopupPosition[] = [
  "top-right",
  "top-left",
  "bottom-right",
  "bottom-left",
];

/// 后端通过 hash fragment(`#position=...&kind=...&n=...`)注入,不能用 query ——
/// Tauri `WebviewUrl::App` 会对路径里的 `?` URL 编码导致 webview 404。
export function readHashParam(name: string): string | null {
  const hash = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  return new URLSearchParams(hash).get(name);
}

export function readPosition(): PopupPosition {
  const raw = readHashParam("position");
  return (VALID_POSITIONS.find((p) => p === raw) as PopupPosition | undefined) ?? "top-right";
}

export function readKind(): PopupKind {
  return readHashParam("kind") === "update" ? "update" : "session";
}

/// session popup 的 label 格式:`popup-s-{token}-{session_id}`(session_id 可含 `-`)。
export function parseSessionIdFromLabel(label: string): string | null {
  const m = label.match(/^popup-s-\d+-(.+)$/);
  return m ? m[1] : null;
}

export function entryOffsetX(position: PopupPosition): number {
  return position === "top-left" || position === "bottom-left" ? -30 : 30;
}

export function glowGradientFor(
  kind: "update" | "idle" | "pending" | "last_failed" | "default",
): string {
  const colors: Record<string, string> = {
    update: "var(--glow-update)",
    idle: "var(--glow-idle)",
    pending: "var(--glow-pending)",
    last_failed: "var(--glow-failed)",
    default: "var(--glow-default)",
  };
  return colors[kind] || colors.default;
}

export const POPUP_BODY_CLASS =
  "relative z-[1] flex h-full flex-col rounded-[14px] panel-base px-4 py-3.5 text-[var(--text-primary)]";

export const POPUP_GUTTER_CLASS = "pt-5 pr-5";
export const POPUP_CARD_INSET_CLASS = "right-5 top-5";
export const POPUP_CLOSE_ANCHOR_CLASS = "right-1 top-1";
export const POPUP_CARD_CLASS = `absolute bottom-0 left-0 ${POPUP_CARD_INSET_CLASS} overflow-hidden rounded-[14px]`;

export const POPUP_FONT_CLASS =
  "font-[-apple-system,BlinkMacSystemFont,'SF Pro Text','PingFang SC',sans-serif] text-[13px] animate-fade-in";

export const POPUP_SHELL_CLASS = `relative box-border h-full w-full overflow-visible ${POPUP_GUTTER_CLASS} bg-transparent ${POPUP_FONT_CLASS}`;
export const POPUP_SHELL_MINI_CLASS = `relative h-full w-full overflow-hidden rounded-[14px] bg-transparent ${POPUP_FONT_CLASS}`;
export const POPUP_CLOSE_INLINE_CLASS = "right-1.5 top-1/2 -translate-y-1/2";
