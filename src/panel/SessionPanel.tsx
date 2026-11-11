import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle, Power, Settings as SettingsIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import type { Session } from "../types";
import { AlertBanner, EmptyState } from "./SessionPanelAlerts";
import { SessionPanelCard } from "./SessionPanelCard";
import { isActive, sortSessionsForPanel } from "./sessionPanelHelpers";
import { usePanelWindowLifecycle } from "./usePanelWindowLifecycle";

interface CcIntegrationStatus {
  installed: boolean;
  binary_executable: boolean;
  settings_exists: boolean;
  hooks_configured: boolean;
  connected: boolean;
  repair_available: boolean;
  issue: string;
}

interface PanelSettings {
  sound_pending: string;
  sound_idle: string;
  sound_failed: string;
  locale: string;
  ui_theme: "light" | "dark" | "system";
  notifications_enabled: boolean;
  popup_position: "top-left" | "top-right" | "bottom-left" | "bottom-right";
  popup_screen_strategy: "terminal" | "primary";
  tray_icon_style: "colored" | "white";
}

function useTimeAgo() {
  const t = useT();
  return (dateStr: string): string => {
    const now = Date.now();
    const then = new Date(dateStr).getTime();
    const seconds = Math.floor((now - then) / 1000);
    if (seconds < 60) return t("time.just_now");
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return t("time.minutes_ago", { n: minutes });
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return t("time.hours_ago", { n: hours });
    const days = Math.floor(hours / 24);
    return t("time.days_ago", { n: days });
  };
}

export {
  isActive,
  projectName,
  sortSessionsForPanel,
  sourceLabel,
  workspacePath,
} from "./sessionPanelHelpers";

export default function SessionPanel() {
  const t = useT();
  const timeAgo = useTimeAgo();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [ccIntegration, setCcIntegration] = useState<CcIntegrationStatus | null>(null);
  const [settings, setSettings] = useState<PanelSettings | null>(null);
  const [driftSources, setDriftSources] = useState<string[]>([]);
  const [driftRepairing, setDriftRepairing] = useState<string | null>(null);

  usePanelWindowLifecycle(settings !== null);

  const loadSessions = async () => {
    const data = await invoke<Session[]>("get_sessions");
    setSessions(data);
  };

  const loadCcIntegration = async () => {
    const data = await invoke<CcIntegrationStatus>("check_cc_integration");
    setCcIntegration(data);
  };

  const loadSettings = async () => {
    const data = await invoke<PanelSettings>("get_settings");
    setSettings(data);
  };

  const loadHookDriftState = async () => {
    const sources = await invoke<string[]>("get_hook_drift_state");
    setDriftSources(sources);
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only bootstrap + event listeners
  useEffect(() => {
    loadSessions();
    loadCcIntegration();
    loadSettings();
    loadHookDriftState();
    const unlistenSessions = listen("sessions-updated", () => {
      loadSessions();
    });
    const unlistenCc = listen<CcIntegrationStatus>("cc-integration-updated", (event) => {
      setCcIntegration(event.payload);
    });
    const unlistenSettings = listen("settings-updated", () => {
      loadSettings();
    });
    const unlistenDriftDetected = listen<string[]>("hook-drift-detected", (event) => {
      setDriftSources(event.payload);
    });
    const unlistenDriftRepaired = listen<string>("hook-drift-repaired", (event) => {
      setDriftSources((prev) => prev.filter((s) => s !== event.payload));
    });
    return () => {
      unlistenSessions.then((fn) => fn());
      unlistenCc.then((fn) => fn());
      unlistenSettings.then((fn) => fn());
      unlistenDriftDetected.then((fn) => fn());
      unlistenDriftRepaired.then((fn) => fn());
    };
  }, []);

  const handleClick = async (id: string) => {
    await invoke("open_session_source", { id });
    invoke("hide_panel");
  };

  const openSettings = async () => {
    await invoke("open_settings_window");
    invoke("hide_panel");
  };

  const toggleNotifications = async (next: boolean) => {
    if (!settings) return;
    const previous = settings;
    const updated: PanelSettings = { ...settings, notifications_enabled: next };
    setSettings(updated);
    try {
      await invoke("save_settings", { settings: updated });
    } catch (err) {
      console.error("save_settings failed, rolling back notifications toggle", err);
      setSettings(previous);
      loadSettings();
    }
  };

  const repairCcIntegration = async (e: React.MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation();
    const result = await invoke<{ status?: CcIntegrationStatus }>("repair_cc_integration");
    if (result?.status) {
      setCcIntegration(result.status);
    } else {
      loadCcIntegration();
    }
  };

  const repairDrift = async (source: string) => {
    setDriftRepairing(source);
    try {
      await invoke("repair_hook_drift", { source });
    } catch (err) {
      console.error("repair_hook_drift failed", err);
    } finally {
      setDriftRepairing((cur) => (cur === source ? null : cur));
    }
  };

  const driftLabelKey = (source: string) => {
    switch (source) {
      case "cc":
        return "panel.drift_cc";
      case "codex":
        return "panel.drift_codex";
      case "cursor":
        return "panel.drift_cursor";
      default:
        return source;
    }
  };

  const sorted = useMemo(() => sortSessionsForPanel(sessions), [sessions]);

  const showCcAlert =
    !!ccIntegration && !ccIntegration.connected && ccIntegration.issue !== "hooks_missing";
  const ccIssueKey = ccIntegration
    ? `panel.cc_health_${ccIntegration.issue}`
    : "panel.cc_health_unknown";

  return (
    <div
      className={cn(
        "h-full w-full flex flex-col overflow-hidden rounded-[14px] panel-base",
        "text-[13px]",
      )}
    >
      <div className="flex items-center justify-between px-[18px] pt-[18px] pb-[14px] border-b border-[var(--border-color)]">
        <h1 className="m-0 text-[17px] font-bold tracking-[0.02em] brand-gradient-text drop-shadow-[0_1px_0_rgba(255,255,255,0.8)] dark:drop-shadow-[0_0_18px_rgba(167,139,250,0.15)]">
          Poke Poke
        </h1>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-[var(--text-tertiary)] hover:bg-red-400/12 hover:text-red-400 focus:outline-none focus-visible:ring-1 focus-visible:ring-red-400/50"
          onClick={() => invoke("quit_app")}
          title={t("panel.quit_app")}
          aria-label={t("panel.quit_app")}
        >
          <Power size={15} strokeWidth={1.75} aria-hidden="true" />
        </Button>
      </div>

      {showCcAlert && (
        <AlertBanner variant="danger">
          <div className="flex min-w-0 flex-col gap-1">
            <div className="text-[13px] font-semibold text-[var(--alert-danger-title)]">
              {t("panel.cc_health_title")}
            </div>
            <div className="text-[12px] leading-[1.4] text-[var(--alert-danger-text)]">
              {t(ccIssueKey)}
            </div>
          </div>
          <Button
            variant="secondary"
            size="sm"
            className="self-start surface-btn border-[var(--alert-danger-border)] text-[var(--alert-danger-title)] hover:bg-[var(--alert-danger-bg)] shadow-none"
            onClick={repairCcIntegration}
            disabled={!ccIntegration.repair_available}
          >
            {ccIntegration.repair_available
              ? t("panel.cc_health_repair")
              : t("panel.cc_health_unavailable")}
          </Button>
        </AlertBanner>
      )}

      {driftSources.length > 0 && (
        <AlertBanner variant="warning">
          <div className="flex items-center gap-1.5">
            <AlertTriangle
              size={14}
              strokeWidth={2.25}
              className="text-amber-400"
              aria-hidden="true"
            />
            <span className="text-[13px] font-semibold text-[var(--alert-warning-title)]">
              {t("panel.drift_title")}
            </span>
          </div>
          <div className="text-[12px] leading-[1.4] text-[var(--alert-warning-text)]">
            {driftSources.map((s) => t(driftLabelKey(s))).join(" · ")}
          </div>
          <div className="flex flex-wrap gap-1.5 mt-0.5">
            {driftSources.map((s) => (
              <button
                key={s}
                type="button"
                className={cn(
                  "rounded-md border border-[var(--alert-warning-border)] bg-[var(--alert-warning-bg)] px-2.5 py-1 text-[11.5px] font-medium text-[var(--alert-warning-title)]",
                  "whitespace-nowrap transition-colors duration-[120ms]",
                  "hover:border-[var(--alert-warning-accent)]/45 hover:bg-[var(--alert-warning-bg)]",
                  "active:opacity-80",
                  "disabled:opacity-55 disabled:cursor-not-allowed",
                  "focus-visible:ring-1 focus-visible:ring-[var(--alert-warning-accent)]/50 focus-visible:outline-none",
                )}
                onClick={(e) => {
                  e.stopPropagation();
                  repairDrift(s);
                }}
                disabled={driftRepairing === s}
              >
                {driftRepairing === s
                  ? t("panel.drift_repairing")
                  : `${t("panel.drift_repair")} ${t(driftLabelKey(s))}`}
              </button>
            ))}
          </div>
        </AlertBanner>
      )}

      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1.5 overflow-y-auto">
        {sorted.length === 0 ? (
          <EmptyState message={t("panel.empty")} />
        ) : (
          sorted.map((session) => (
            <SessionPanelCard
              key={session.id}
              session={session}
              active={isActive(session)}
              timeAgo={timeAgo}
              onClick={() => handleClick(session.id)}
            />
          ))
        )}
      </div>

      <div className="glass-inner flex items-center gap-2 border-t border-[var(--border-color)] px-3 py-2.5 text-[12px] tracking-[0.02em]">
        <label
          htmlFor="panel-notifications-toggle"
          className={cn(
            "inline-flex items-center gap-2 px-1.5 py-1 rounded-md cursor-pointer",
            "text-[11px] text-[var(--text-secondary)] transition-colors",
            "hover:text-[var(--text-primary)]",
            !settings && "opacity-50 cursor-progress",
          )}
          title={
            settings?.notifications_enabled
              ? t("panel.notifications_on")
              : t("panel.notifications_off")
          }
        >
          <span className="leading-none">{t("panel.notifications_label")}</span>
          <Switch
            id="panel-notifications-toggle"
            checked={!!settings?.notifications_enabled}
            onCheckedChange={toggleNotifications}
            disabled={!settings}
            className="h-4 w-7 focus:outline-none data-[state=checked]:bg-[var(--brand-purple)]/80 data-[state=unchecked]:bg-[var(--border-color)] [&>span]:h-3 [&>span]:w-3 [&>span]:data-[state=checked]:translate-x-3"
          />
        </label>
        <span className="ml-auto text-[var(--text-tertiary)]">
          {t("panel.sessions", { n: sorted.length })}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 text-[var(--text-tertiary)] hover:bg-[var(--bg-tertiary)] hover:text-[var(--text-primary)] focus:outline-none focus-visible:ring-1 focus-visible:ring-[var(--brand-purple)]/30"
          onClick={openSettings}
          title="Settings"
          aria-label="Settings"
        >
          <SettingsIcon size={15} strokeWidth={1.75} aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
