import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BellOff } from "lucide-react";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { Button } from "../components/ui/button";
import { DEFAULT_QUIET_MODE_SHORTCUT, formatFullShortcut } from "../lib/shortcutModifiers";
import type { QuietModeStatus } from "./types";
import { QuietModeProgressRing } from "./QuietModeProgressRing";

const IDLE_STATUS: QuietModeStatus = {
  active: false,
  remaining_secs: null,
  total_secs: null,
};

interface QuietModeSettingsCardProps {
  quietModeShortcut: string;
}

export function QuietModeSettingsCard({ quietModeShortcut }: QuietModeSettingsCardProps) {
  const t = useT();
  const [status, setStatus] = useState<QuietModeStatus>(IDLE_STATUS);
  const [busy, setBusy] = useState(false);

  const hotkeyLabel = formatFullShortcut(quietModeShortcut || DEFAULT_QUIET_MODE_SHORTCUT);

  useEffect(() => {
    void invoke<QuietModeStatus>("get_quiet_mode_status").then(setStatus);
    const unlisten = listen<QuietModeStatus>("quiet-mode-changed", (e) => {
      setStatus(e.payload);
    });
    const tick = window.setInterval(() => {
      void invoke<QuietModeStatus>("get_quiet_mode_status").then(setStatus);
    }, 1000);
    return () => {
      unlisten.then((fn) => fn());
      window.clearInterval(tick);
    };
  }, []);

  const openPicker = () => {
    void invoke("open_quiet_mode_picker");
  };

  const endQuietMode = async () => {
    setBusy(true);
    try {
      await invoke("cancel_quiet_mode");
    } finally {
      setBusy(false);
    }
  };

  const remaining = status.remaining_secs ?? 0;
  const total = status.total_secs ?? remaining;

  return (
    <div className="flex items-start justify-between gap-3">
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <div
          className={cn(
            "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border",
            status.active
              ? "border-[var(--brand-blue)]/30 bg-[var(--brand-blue)]/12 text-[var(--brand-blue)]"
              : "border-[var(--border-color)] bg-[var(--surface-muted)] text-[var(--text-secondary)]",
          )}
        >
          <BellOff className="h-4 w-4" strokeWidth={1.75} aria-hidden="true" />
        </div>
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-sm font-semibold text-foreground">{t("quiet_mode.title")}</h3>
            {status.active ? (
              <span className="inline-flex items-center rounded-full bg-[var(--brand-blue)]/15 px-2 py-0.5 text-[10px] font-medium tracking-wide text-[var(--brand-blue)]">
                {t("quiet_mode.settings_active")}
              </span>
            ) : null}
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("quiet_mode.subtitle")}
            <span className="ml-1.5 whitespace-nowrap text-[var(--text-tertiary)]">
              <kbd className="popup-kbd rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold">
                {hotkeyLabel}
              </kbd>
              <span className="ml-1">{t("quiet_mode.settings_hotkey_suffix")}</span>
            </span>
          </p>
        </div>
      </div>

      {status.active && status.remaining_secs != null ? (
        <div className="flex shrink-0 items-center gap-2">
          <QuietModeProgressRing remainingSecs={remaining} totalSecs={total} size="sm" />
          <Button
            variant="outline"
            size="sm"
            className="h-8 shrink-0 border-[var(--border-color)] bg-[var(--bg-primary-solid)]"
            disabled={busy}
            onClick={() => void endQuietMode()}
          >
            {t("quiet_mode.end_now")}
          </Button>
        </div>
      ) : (
        <Button
          variant="outline"
          size="sm"
          className="h-8 shrink-0 border-[var(--border-color)] bg-[var(--bg-primary-solid)]"
          onClick={() => {
            if (busy) return;
            openPicker();
          }}
        >
          {t("quiet_mode.settings_open")}
        </Button>
      )}
    </div>
  );
}
