import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import PopupWindow from "./popup/PopupWindow";
import SessionPanel from "./panel/SessionPanel";
import SettingsWindow from "./settings/SettingsWindow";
import ConfettiOverlay from "./confetti/ConfettiOverlay";
import QuietModePicker from "./quiet-mode/QuietModePicker";
import LandingPage from "./landing/LandingPage";
import DocsPage from "./docs/DocsPage";
import { LocaleProvider } from "./i18n/context";
import type { Locale } from "./i18n/strings";
import {
  applyUiTheme,
  isUiTheme,
  readThemeFromHash,
  subscribeSystemTheme,
  type UiTheme,
} from "./lib/theme";

const isTauriRuntime = () => "__TAURI_INTERNALS__" in window;

// 首次(无已存选择)按浏览器/系统语言:以 zh 开头→中文,其余→英文。
const detectBrowserLocale = (): Locale =>
  navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";

function readAppKind(): string | null {
  if (typeof window === "undefined") return null;
  const hash = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  return new URLSearchParams(hash).get("kind");
}

function App() {
  const label = isTauriRuntime() ? getCurrentWebviewWindow().label : "landing";
  // 浏览器预览(#kind=quiet-mode)也走这里;Tauri 内行为不变
  const appKind = readAppKind();
  const [locale, setLocale] = useState<Locale>(() => {
    const stored = localStorage.getItem("pokepoke-landing-locale");
    if (stored === "en" || stored === "zh") return stored;
    return detectBrowserLocale();
  });
  const [uiTheme, setUiTheme] = useState<UiTheme>(() => readThemeFromHash() ?? "system");

  useEffect(() => {
    if (!isTauriRuntime()) return;
    if (!label.startsWith("popup-")) return;
    document.documentElement.classList.add("popup-window");
    return () => document.documentElement.classList.remove("popup-window");
  }, [label]);

  useEffect(() => {
    applyUiTheme(uiTheme);
    if (uiTheme !== "system") return;
    return subscribeSystemTheme(() => applyUiTheme("system"));
  }, [uiTheme]);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    invoke<{ locale: string; ui_theme: string }>("get_settings").then((s) => {
      if (s.locale === "en" || s.locale === "zh") setLocale(s.locale);
      if (isUiTheme(s.ui_theme)) setUiTheme(s.ui_theme);
    });
    const unlisten = listen("settings-updated", () => {
      invoke<{ locale: string; ui_theme: string }>("get_settings").then((s) => {
        if (s.locale === "en" || s.locale === "zh") setLocale(s.locale);
        if (isUiTheme(s.ui_theme)) setUiTheme(s.ui_theme);
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const setLandingLocale = (nextLocale: Locale) => {
    setLocale(nextLocale);
    if (!isTauriRuntime()) {
      localStorage.setItem("pokepoke-landing-locale", nextLocale);
    }
  };

  // /docs 仅 web 路由;Tauri 桌面端永不命中(isTauriRuntime 门控)
  const isDocsRoute = !isTauriRuntime() && window.location.pathname.startsWith("/docs");

  let content: React.ReactNode = null;

  if (label.startsWith("popup-")) {
    content = <PopupWindow />;
  } else if (label === "panel") {
    content = <SessionPanel />;
  } else if (label === "settings") {
    content = <SettingsWindow />;
  } else if (label === "confetti-overlay") {
    content = <ConfettiOverlay />;
  } else if (label === "quiet-mode" || appKind === "quiet-mode") {
    content = <QuietModePicker />;
  } else if (isDocsRoute) {
    content = <DocsPage locale={locale} onLocaleChange={setLandingLocale} />;
  } else if (label === "main") {
    content = <LandingPage locale={locale} onLocaleChange={setLandingLocale} />;
  } else {
    content = <LandingPage locale={locale} onLocaleChange={setLandingLocale} />;
  }

  return <LocaleProvider value={locale}>{content}</LocaleProvider>;
}

export default App;
