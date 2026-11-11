import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";
import { devDebug } from "../devLog";

// NSPanel show_and_make_key 后 webview 会把焦点落到第一个可聚焦控件
// (Switch / 退出按钮等),触发 focus-visible 描边。Panel 是扫一眼的浮层,
// 打开时不应有任何控件处于聚焦态 —— 在 focus/visibility 回调里主动 blur。
// 注意：blur 控件偶发会连带触发 window.blur / Rust Focused(false)，
// 需要短暂抑制 hide_panel，否则 tray 点开立刻被收掉。
const PANEL_BLUR_HIDE_GRACE_MS = 400;

function clearPanelAutoFocus(suppressBlurHideUntilRef: React.MutableRefObject<number>) {
  const extendGrace = () => {
    suppressBlurHideUntilRef.current = Date.now() + PANEL_BLUR_HIDE_GRACE_MS;
  };
  extendGrace();
  // 双 rAF:等 webview makeKey 之后浏览器完成自动聚焦,再 blur 掉。
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      extendGrace();
      const el = document.activeElement;
      if (el instanceof HTMLElement && el !== document.body && el !== document.documentElement) {
        el.blur();
      }
    });
  });
}

/** panel 窗 document 类 + 失焦自动关闭 + 打开时清焦点。 */
export function usePanelWindowLifecycle(settingsLoaded: boolean) {
  const settingsReadyRef = useRef(false);
  const suppressBlurHideUntilRef = useRef(0);

  useEffect(() => {
    document.documentElement.classList.add("panel-window");
    return () => document.documentElement.classList.remove("panel-window");
  }, []);

  useEffect(() => {
    devDebug("panel", "SessionPanel mounted, attaching blur listener");
    suppressBlurHideUntilRef.current = Date.now() + PANEL_BLUR_HIDE_GRACE_MS;
    const handleBlur = () => {
      if (Date.now() < suppressBlurHideUntilRef.current) {
        devDebug("panel", "window.blur suppressed (focus clearing grace period)");
        return;
      }
      devDebug("panel", "window.blur fired, calling hide_panel");
      invoke("hide_panel");
    };
    const handleFocus = () => {
      devDebug("panel", "window.focus fired");
      clearPanelAutoFocus(suppressBlurHideUntilRef);
    };
    const handleVisibility = () => {
      devDebug("panel", `visibilitychange -> ${document.visibilityState}`);
      if (document.visibilityState === "visible") {
        clearPanelAutoFocus(suppressBlurHideUntilRef);
      }
    };
    window.addEventListener("blur", handleBlur);
    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisibility);
    clearPanelAutoFocus(suppressBlurHideUntilRef);

    return () => {
      devDebug("panel", "SessionPanel unmounted, cleaning listeners");
      window.removeEventListener("blur", handleBlur);
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  useEffect(() => {
    if (!settingsLoaded || settingsReadyRef.current) return;
    settingsReadyRef.current = true;
    clearPanelAutoFocus(suppressBlurHideUntilRef);
  }, [settingsLoaded]);
}
