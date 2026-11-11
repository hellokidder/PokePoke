import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "../components/ui/button";
import { useT } from "../i18n/context";
import { FullShortcutCaptureSession, HotkeyCaptureSession } from "../lib/shortcutModifiers";
import { cn } from "../lib/utils";
import { AboutTab } from "./AboutTab";
import { GeneralTab } from "./GeneralTab";
import { IntegrationsTab } from "./IntegrationsTab";
import { PopupTab } from "./PopupTab";
import {
  type CheckUpdateResult,
  type DiagnosticsState,
  HEADER_BORDER_CLASS,
  SETTINGS_BODY_CLASS,
  SETTINGS_CONTAINER_CLASS,
  SETTINGS_TITLE_CLASS,
  type Settings,
  TABS,
  type TabKey,
  type UpdateProgressPayload,
  type UpdateState,
} from "./shared";
import { useSoundCatalog } from "./useSoundCatalog";

// 兼容 re-export:这三个符号原 export 自本文件,现归入 ./shared(IntegrationsTab/test 零改动)。
export { WHITE_TRAY_ICON_IMAGE_CLASS } from "./shared";

export default function SettingsWindow() {
  const t = useT();
  const [settings, setSettings] = useState<Settings | null>(null);
  const { sounds, bundledSounds, customSounds, reloadCustomSounds } = useSoundCatalog();

  const [saved, setSaved] = useState(false);
  const [activeTab, setActiveTab] = useState<TabKey>("general");
  const [version, setVersion] = useState<string>("");
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: "idle" });
  const [downloaded, setDownloaded] = useState(0);
  const [totalBytes, setTotalBytes] = useState<number | null>(null);
  const [diagnosticsState, setDiagnosticsState] = useState<DiagnosticsState>({ kind: "idle" });
  const [hotkeyRecording, setHotkeyRecording] = useState(false);
  const hotkeyButtonRef = useRef<HTMLButtonElement>(null);
  const [quietHotkeyRecording, setQuietHotkeyRecording] = useState(false);
  const quietHotkeyButtonRef = useRef<HTMLButtonElement>(null);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  useEffect(() => {
    document.documentElement.classList.add("settings-window");
    return () => document.documentElement.classList.remove("settings-window");
  }, []);

  useEffect(() => {
    invoke<Settings>("get_settings").then(setSettings);
    invoke<string>("app_version")
      .catch(() => getVersion())
      .then(setVersion);
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (hotkeyRecording) {
        setHotkeyRecording(false);
        return;
      }
      if (quietHotkeyRecording) {
        setQuietHotkeyRecording(false);
        return;
      }
      invoke("close_settings_window");
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [hotkeyRecording, quietHotkeyRecording]);

  useEffect(() => {
    if (!hotkeyRecording) return;
    hotkeyButtonRef.current?.focus();

    const session = new HotkeyCaptureSession();
    const commitModifiers = (modifiers: string) => {
      const snapshot = settingsRef.current;
      if (!snapshot) return;
      setHotkeyRecording(false);
      const next = { ...snapshot, popup_shortcut_modifiers: modifiers };
      setSettings(next);
      void invoke("save_settings", { settings: next })
        .then(() => {
          setSaved(true);
          setTimeout(() => setSaved(false), 1200);
        })
        .catch((err) => {
          console.error("save_settings failed, rolling back", err);
          setSettings(snapshot);
          invoke<Settings>("get_settings").then(setSettings);
        });
    };

    const onKeyDown = (e: KeyboardEvent) => {
      const result = session.onKeyDown(e);
      if (result.type === "ignore") return;
      e.preventDefault();
      e.stopPropagation();
      if (result.type === "cancel") {
        setHotkeyRecording(false);
        return;
      }
      commitModifiers(result.modifiers);
    };

    const onKeyUp = (e: KeyboardEvent) => {
      const result = session.onKeyUp(e);
      if (result.type === "ignore") return;
      e.preventDefault();
      e.stopPropagation();
      if (result.type === "cancel") {
        setHotkeyRecording(false);
        return;
      }
      commitModifiers(result.modifiers);
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [hotkeyRecording]);

  useEffect(() => {
    if (!quietHotkeyRecording) return;
    quietHotkeyButtonRef.current?.focus();

    const session = new FullShortcutCaptureSession();
    const commitShortcut = (shortcut: string) => {
      const snapshot = settingsRef.current;
      if (!snapshot) return;
      setQuietHotkeyRecording(false);
      const next = { ...snapshot, quiet_mode_shortcut: shortcut };
      setSettings(next);
      void invoke("save_settings", { settings: next })
        .then(() => {
          setSaved(true);
          setTimeout(() => setSaved(false), 1200);
        })
        .catch((err) => {
          console.error("save_settings failed, rolling back", err);
          setSettings(snapshot);
          invoke<Settings>("get_settings").then(setSettings);
        });
    };

    const onKeyDown = (e: KeyboardEvent) => {
      const result = session.onKeyDown(e);
      if (result.type === "ignore") return;
      e.preventDefault();
      e.stopPropagation();
      if (result.type === "cancel") {
        setQuietHotkeyRecording(false);
        return;
      }
      commitShortcut(result.shortcut);
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [quietHotkeyRecording]);

  if (!settings) return null;

  const saveField = async (key: keyof Settings, value: unknown) => {
    const previous = settings;
    const next = { ...settings, [key]: value } as Settings;
    setSettings(next);
    try {
      await invoke("save_settings", { settings: next });
      setSaved(true);
      setTimeout(() => setSaved(false), 1200);
    } catch (err) {
      // 写失败回滚本地状态,避免 UI 显示新值但配置没落盘。对齐
      // SessionPanel.toggleNotifications 的处理;兜底从后端 reload 防 previous 过期。
      console.error("save_settings failed, rolling back", err);
      setSettings(previous);
      invoke<Settings>("get_settings").then(setSettings);
    }
  };

  const handleCheckUpdate = async () => {
    setUpdateState({ kind: "checking" });
    try {
      const result = await invoke<CheckUpdateResult>("check_update_now");
      if (result.available && result.info) {
        setUpdateState({ kind: "available", info: result.info });
      } else {
        setUpdateState({ kind: "up_to_date" });
      }
    } catch {
      setUpdateState({ kind: "failed" });
    }
  };

  const handleInstallUpdate = async () => {
    setDownloaded(0);
    setTotalBytes(null);
    setUpdateState({ kind: "installing" });

    let unlisten: (() => void) | null = null;
    let pendingRaf: number | null = null;
    let lastDownloaded = 0;
    let lastTotal: number | null = null;

    try {
      unlisten = await listen<UpdateProgressPayload>("update-progress", (e) => {
        if (e.payload.phase === "downloading") {
          lastDownloaded = e.payload.downloaded;
          lastTotal = e.payload.total;
          if (pendingRaf === null) {
            pendingRaf = requestAnimationFrame(() => {
              pendingRaf = null;
              setDownloaded(lastDownloaded);
              setTotalBytes(lastTotal);
            });
          }
        }
      });

      await invoke("download_update_with_progress");
      if (pendingRaf !== null) {
        cancelAnimationFrame(pendingRaf);
        pendingRaf = null;
      }
      setDownloaded(lastDownloaded);
      setTotalBytes(lastTotal);

      await invoke("install_cached_update");
    } catch {
      setUpdateState({ kind: "failed" });
    } finally {
      if (pendingRaf !== null) cancelAnimationFrame(pendingRaf);
      if (unlisten) unlisten();
    }
  };

  const handleFeedback = async () => {
    const body = [
      "### 问题描述 / What happened\n\n\n",
      "### 复现步骤 / Steps to reproduce\n\n\n",
      "---",
      `- App 版本: ${version || "unknown"}`,
      `- 系统: ${navigator.userAgent}`,
    ].join("\n");
    // 必须指向公仓:用户只装了公开版本,私仓 issues 对他们返回 404。
    const url =
      "https://github.com/hellokidder/PokePoke/issues/new?" +
      new URLSearchParams({ title: "[反馈] ", body }).toString();
    await openUrl(url);
  };

  const handleExportDiagnostics = async () => {
    setDiagnosticsState({ kind: "exporting" });
    try {
      const path = await invoke<string>("export_diagnostics");
      setDiagnosticsState({ kind: "done", path });
    } catch {
      setDiagnosticsState({ kind: "failed" });
    }
  };

  return (
    <div className={SETTINGS_CONTAINER_CLASS}>
      <div className={SETTINGS_BODY_CLASS}>
        {/* Header - Tauri drag region;父元素 data-tauri-drag-region 已足够,Button 自动 bypass 拖拽 */}
        <div
          className="flex items-center justify-between border-b px-6 pb-4 pt-5"
          style={{ borderColor: "var(--divider)" }}
          data-tauri-drag-region
        >
          <h1 className={SETTINGS_TITLE_CLASS}>{t("settings.title")}</h1>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 text-muted-foreground hover:bg-[var(--surface-hover)] hover:text-foreground focus-visible:ring-ring/50"
            onClick={() => invoke("close_settings_window")}
            aria-label="Close"
          >
            <X size={16} strokeWidth={1.75} aria-hidden="true" />
          </Button>
        </div>

        {/* Body: sidebar | tabpanel -- 纵向滚动责任下放到具体 tabpanel */}
        <div className="flex min-h-0 flex-1 overflow-hidden">
          {/* Sidebar tab 列表 */}
          <div
            role="tablist"
            aria-orientation="vertical"
            aria-label={t("settings.title")}
            className="flex w-[132px] shrink-0 flex-col gap-0.5 border-r border-[var(--border-color)] bg-[var(--bg-secondary)] px-3 py-3"
          >
            {TABS.map((key) => {
              const active = activeTab === key;
              return (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  id={`settings-tab-${key}`}
                  aria-selected={active}
                  aria-controls={`settings-tabpanel-${key}`}
                  tabIndex={active ? 0 : -1}
                  className={cn(
                    "relative rounded-md border border-transparent px-3 py-2 text-left text-sm transition-[background-color,border-color,box-shadow,color]",
                    "focus:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                    active
                      ? "glass-btn font-medium text-[var(--text-primary)] border-[var(--brand-purple)]/30"
                      : "text-[var(--text-secondary)] hover:glass-btn hover:text-[var(--text-primary)]",
                  )}
                  onClick={() => setActiveTab(key)}
                >
                  {active ? (
                    <span
                      aria-hidden
                      className="absolute bottom-1.5 left-0 top-1.5 w-[3px] rounded-full bg-gradient-to-b from-[var(--brand-purple)] to-[var(--brand-blue)]"
                      style={{ boxShadow: "0 0 8px var(--brand-purple)" }}
                    />
                  ) : null}
                  {t(`settings.tab_${key}` as const)}
                </button>
              );
            })}
          </div>

          {/* Tabpanel - 全部 tab overflow-y-auto:窗口可竖直缩放,内容超高时滚动条兜底 */}
          <div
            role="tabpanel"
            id={`settings-tabpanel-${activeTab}`}
            aria-labelledby={`settings-tab-${activeTab}`}
            // biome-ignore lint/a11y/noNoninteractiveTabindex: WAI-ARIA tabs 规范要求 tabpanel 可聚焦以便键盘进入内容区
            tabIndex={0}
            className={cn(
              "min-h-0 min-w-0 flex-1 px-6 pb-5 pt-1 focus:outline-none",
              "overflow-y-auto [&::-webkit-scrollbar-thumb]:rounded-sm [&::-webkit-scrollbar-thumb]:bg-[var(--scrollbar-thumb)] [&::-webkit-scrollbar]:w-1",
            )}
          >
            {activeTab === "general" && (
              <GeneralTab
                settings={settings}
                saveField={saveField}
                sounds={sounds}
                bundledSounds={bundledSounds}
                customSounds={customSounds}
                onCustomSoundsChange={reloadCustomSounds}
                hotkeyRecording={hotkeyRecording}
                setHotkeyRecording={setHotkeyRecording}
                hotkeyButtonRef={hotkeyButtonRef}
                quietHotkeyRecording={quietHotkeyRecording}
                setQuietHotkeyRecording={setQuietHotkeyRecording}
                quietHotkeyButtonRef={quietHotkeyButtonRef}
              />
            )}

            {activeTab === "popup" && <PopupTab settings={settings} saveField={saveField} />}

            {activeTab === "integrations" && <IntegrationsTab />}

            {activeTab === "about" && (
              <AboutTab
                settings={settings}
                saveField={saveField}
                version={version}
                updateState={updateState}
                downloaded={downloaded}
                totalBytes={totalBytes}
                diagnosticsState={diagnosticsState}
                handleCheckUpdate={handleCheckUpdate}
                handleInstallUpdate={handleInstallUpdate}
                handleFeedback={handleFeedback}
                handleExportDiagnostics={handleExportDiagnostics}
              />
            )}
          </div>
        </div>

        {saved ? (
          <div
            className={cn(
              "flex justify-center border-t px-6 py-3.5 animate-fade-in",
              HEADER_BORDER_CLASS,
            )}
          >
            <span className="text-xs tracking-wide text-[var(--success-text)]">
              {t("settings.saved")}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
