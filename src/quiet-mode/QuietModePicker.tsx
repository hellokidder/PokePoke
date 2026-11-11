import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { BellOff, X } from "lucide-react";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { Button } from "../components/ui/button";
import { DEFAULT_QUIET_MODE_SHORTCUT, formatFullShortcut } from "../lib/shortcutModifiers";
import type { QuietModeStatus } from "./types";
import { QuietModeProgressRing } from "./QuietModeProgressRing";

const DURATION_PRESETS = [5, 15, 30, 60] as const;

const IDLE_STATUS: QuietModeStatus = {
  active: false,
  remaining_secs: null,
  total_secs: null,
};

function readInitialStatus(): QuietModeStatus {
  if (typeof window === "undefined") return IDLE_STATUS;
  const params = new URLSearchParams(window.location.hash.slice(1));
  if (params.get("active") === "1") {
    return {
      active: true,
      remaining_secs: Number(params.get("remaining") ?? 0),
      total_secs: Number(params.get("total") ?? 0),
    };
  }
  return IDLE_STATUS;
}

function sameQuietStatus(a: QuietModeStatus, b: QuietModeStatus): boolean {
  return (
    a.active === b.active && a.remaining_secs === b.remaining_secs && a.total_secs === b.total_secs
  );
}

export default function QuietModePicker() {
  const t = useT();
  const [status, setStatus] = useState<QuietModeStatus>(readInitialStatus);
  const [durationMins, setDurationMins] = useState(30);
  const [busy, setBusy] = useState(false);
  const [hotkeyLabel, setHotkeyLabel] = useState(() =>
    formatFullShortcut(DEFAULT_QUIET_MODE_SHORTCUT),
  );
  // 窗体刚获得焦点时 WKWebView 可能在光标下合成 mousedown,短暂禁用点击避免白闪
  const [pointerReady, setPointerReady] = useState(false);

  const applyStatus = (next: QuietModeStatus) => {
    setStatus((prev) => (sameQuietStatus(prev, next) ? prev : next));
  };

  const refresh = () => {
    void invoke<QuietModeStatus>("get_quiet_mode_status").then(applyStatus);
    void invoke<{ quiet_mode_duration_mins?: number; quiet_mode_shortcut?: string }>(
      "get_settings",
    ).then((s) => {
      if (s.quiet_mode_duration_mins && s.quiet_mode_duration_mins > 0) {
        setDurationMins(s.quiet_mode_duration_mins);
      }
      setHotkeyLabel(formatFullShortcut(s.quiet_mode_shortcut ?? DEFAULT_QUIET_MODE_SHORTCUT));
    });
  };

  useEffect(() => {
    document.documentElement.classList.add("quiet-mode-window");
    const frame = window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => setPointerReady(true));
    });
    return () => {
      window.cancelAnimationFrame(frame);
      document.documentElement.classList.remove("quiet-mode-window");
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only quiet mode sync
  useEffect(() => {
    refresh();
    const unlistenChanged = listen<QuietModeStatus>("quiet-mode-changed", (e) => {
      applyStatus(e.payload);
    });
    const unlistenSettings = listen("settings-updated", () => {
      void invoke<{ quiet_mode_shortcut?: string }>("get_settings").then((s) => {
        setHotkeyLabel(formatFullShortcut(s.quiet_mode_shortcut ?? DEFAULT_QUIET_MODE_SHORTCUT));
      });
    });
    const tick = window.setInterval(() => {
      void invoke<QuietModeStatus>("get_quiet_mode_status").then(applyStatus);
    }, 1000);
    return () => {
      unlistenChanged.then((fn) => fn());
      unlistenSettings.then((fn) => fn());
      window.clearInterval(tick);
    };
  }, []);

  const close = () => {
    void invoke("close_quiet_mode_picker");
  };

  const start = async () => {
    setBusy(true);
    try {
      await invoke("start_quiet_mode", { durationMins });
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    setBusy(true);
    try {
      await invoke("cancel_quiet_mode");
    } finally {
      setBusy(false);
    }
  };

  const remaining = status.remaining_secs ?? 0;
  const total = status.total_secs ?? remaining;
  const isActive = status.active && status.remaining_secs != null;

  return (
    <div className="relative h-full w-full bg-transparent p-[14px] text-[var(--text-primary)]">
      <div
        className={cn(
          "quiet-picker-card relative flex h-full flex-col overflow-hidden rounded-[14px]",
          !pointerReady && "pointer-events-none",
        )}
      >
        <div
          data-tauri-drag-region
          className="absolute left-0 top-0 z-10 h-9 w-[300px] cursor-move"
          aria-hidden="true"
        />
        <div
          className="pointer-events-none absolute inset-x-0 top-0 h-28"
          style={{
            background:
              "radial-gradient(ellipse 75% 100% at 50% 0%, color-mix(in srgb, var(--brand-blue) 12%, transparent), transparent 70%)",
          }}
          aria-hidden="true"
        />
        <div className="relative flex items-start justify-between border-b border-[var(--divider)] px-5 pb-3.5 pt-4">
          <div className="flex min-w-0 items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--brand-blue)]/25 bg-gradient-to-br from-[var(--brand-blue)]/20 to-[var(--brand-purple)]/10 text-[var(--brand-blue)] shadow-[inset_0_1px_0_var(--inset-highlight)]">
              <BellOff className="h-[18px] w-[18px]" strokeWidth={1.75} aria-hidden="true" />
            </div>
            <div className="min-w-0 pt-0.5">
              <h1 className="m-0 text-[15px] font-bold tracking-[0.02em] brand-gradient-text">
                {t("quiet_mode.title")}
              </h1>
              <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">
                {t("quiet_mode.subtitle")}
              </p>
            </div>
          </div>
          <button
            type="button"
            data-tauri-no-drag
            className="quiet-interactive-btn rounded-md p-1 text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
            onClick={close}
            aria-label={t("quiet_mode.close")}
          >
            <X className="h-4 w-4" strokeWidth={1.75} />
          </button>
        </div>

        <div className="relative flex min-h-0 flex-1 flex-col px-5 py-4">
          <AnimatePresence mode="wait" initial={false}>
            {isActive ? (
              <motion.div
                key="active"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.2 }}
                className="flex flex-1 flex-col items-center justify-center gap-3 py-1"
              >
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2 w-2">
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-[var(--brand-blue)] opacity-40" />
                    <span className="relative inline-flex h-2 w-2 rounded-full bg-[var(--brand-blue)]" />
                  </span>
                  <span className="text-[11px] font-medium tracking-wide text-[var(--brand-blue)]">
                    {t("quiet_mode.active_label")}
                  </span>
                </div>

                <QuietModeProgressRing remainingSecs={remaining} totalSecs={total} size="lg" />

                <p className="max-w-[240px] text-center text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  {t("quiet_mode.active_hint")}
                </p>
              </motion.div>
            ) : (
              <motion.div
                key="idle"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                transition={{ duration: 0.2 }}
                className="flex flex-1 flex-col gap-3"
              >
                <p className="text-[11px] font-medium tracking-wide text-[var(--text-tertiary)]">
                  {t("quiet_mode.pick_duration")}
                </p>
                <div className="grid grid-cols-2 gap-2">
                  {DURATION_PRESETS.map((mins) => {
                    const selected = durationMins === mins;
                    return (
                      <button
                        key={mins}
                        type="button"
                        data-tauri-no-drag
                        className={cn(
                          "quiet-interactive-btn group relative flex flex-col items-center justify-center rounded-xl border px-3 py-2.5 transition-colors",
                          selected
                            ? "border-[var(--brand-blue)]/50 bg-gradient-to-b from-[var(--brand-blue)]/14 to-[var(--brand-purple)]/6 shadow-[0_0_0_1px_color-mix(in_srgb,var(--brand-blue)_18%,transparent)]"
                            : "border-[var(--border-color)] bg-[var(--surface-muted)] hover:border-[var(--border-hover)] hover:bg-[var(--surface-hover)]",
                        )}
                        onClick={() => setDurationMins(mins)}
                      >
                        <span
                          className={cn(
                            "text-xl font-semibold tabular-nums leading-none tracking-tight",
                            selected
                              ? "text-[var(--brand-blue)]"
                              : "text-[var(--text-primary)] group-hover:text-[var(--text-primary)]",
                          )}
                        >
                          {mins}
                        </span>
                        <span
                          className={cn(
                            "mt-1 text-[10px] font-medium",
                            selected
                              ? "text-[var(--brand-blue)]/80"
                              : "text-[var(--text-tertiary)]",
                          )}
                        >
                          {t("quiet_mode.duration_unit")}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </motion.div>
            )}
          </AnimatePresence>

          <div className="mt-auto space-y-3 pt-2">
            {isActive ? (
              <Button
                className="quiet-interactive-btn h-10 w-full border-[var(--border-color)] bg-[var(--surface-muted)] text-[var(--text-primary)] hover:bg-[var(--surface-hover)] focus-visible:ring-0"
                variant="outline"
                disabled={busy}
                onClick={() => void cancel()}
              >
                {t("quiet_mode.end_now")}
              </Button>
            ) : (
              <Button
                className="quiet-interactive-btn h-10 w-full bg-gradient-to-r from-[var(--brand-purple)] to-[var(--brand-blue)] text-white shadow-[0_4px_14px_color-mix(in_srgb,var(--brand-blue)_28%,transparent)] hover:opacity-95 focus-visible:ring-0"
                disabled={busy}
                onClick={() => void start()}
              >
                {t("quiet_mode.start")}
              </Button>
            )}

            <p className="text-center text-[10px] text-[var(--text-tertiary)]">
              <kbd className="popup-kbd rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold">
                {hotkeyLabel}
              </kbd>
              <span className="ml-1.5">{t("quiet_mode.hotkey_hint_suffix")}</span>
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
