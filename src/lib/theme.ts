export type UiTheme = "light" | "dark" | "system";

export function isUiTheme(value: string): value is UiTheme {
  return value === "light" || value === "dark" || value === "system";
}

function readHashParams(): URLSearchParams {
  const hash = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  return new URLSearchParams(hash);
}

/** 静谧模式窗 URL hash 预载主题,供首帧同步应用。 */
export function readThemeFromHash(): UiTheme | null {
  const raw = readHashParams().get("theme");
  return raw && isUiTheme(raw) ? raw : null;
}

/** React 挂载前同步 document 状态,避免透明浮窗首帧亮色 / tap highlight 规则未生效。 */
export function bootstrapDocument(): void {
  if (readHashParams().get("kind") === "quiet-mode") {
    document.documentElement.classList.add("quiet-mode-window");
  }
  applyUiTheme(readThemeFromHash() ?? "system");
}

export function applyUiTheme(theme: UiTheme): void {
  const root = document.documentElement;
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const resolved = theme === "system" ? (prefersDark ? "dark" : "light") : theme;

  root.classList.remove("dark", "light");
  root.classList.add(resolved);
}

export function subscribeSystemTheme(onChange: () => void): () => void {
  const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaQuery.addEventListener("change", onChange);
  return () => mediaQuery.removeEventListener("change", onChange);
}
