import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { Button } from "../components/ui/button";
import { Label } from "../components/ui/label";
import { Switch } from "../components/ui/switch";
import { Separator } from "../components/ui/separator";
import {
  DIVIDER_CLASS,
  SURFACE_BTN_CLASS,
  SETTINGS_SWITCH_CLASS,
  SettingsSection,
  type Settings,
  type UpdateState,
  type DiagnosticsState,
} from "./shared";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

interface AboutTabProps {
  settings: Settings;
  saveField: (key: keyof Settings, value: unknown) => Promise<void>;
  version: string;
  updateState: UpdateState;
  downloaded: number;
  totalBytes: number | null;
  diagnosticsState: DiagnosticsState;
  handleCheckUpdate: () => Promise<void>;
  handleInstallUpdate: () => Promise<void>;
  handleFeedback: () => Promise<void>;
  handleExportDiagnostics: () => Promise<void>;
}

export function AboutTab({
  settings,
  saveField,
  version,
  updateState,
  downloaded,
  totalBytes,
  diagnosticsState,
  handleCheckUpdate,
  handleInstallUpdate,
  handleFeedback,
  handleExportDiagnostics,
}: AboutTabProps) {
  const t = useT();

  const progressPercent =
    totalBytes && totalBytes > 0
      ? Math.min(100, Math.floor((downloaded / totalBytes) * 100))
      : null;

  return (
    <SettingsSection
      title={t("settings.about")}
      description={t("settings.about_desc")}
      showSeparator={false}
    >
      <div className="space-y-4">
        <div className="flex items-center justify-between gap-3">
          <Label className="text-sm text-foreground">{t("settings.update_auto_check")}</Label>
          <Switch
            checked={settings.update_auto_check}
            onCheckedChange={(value) => saveField("update_auto_check", value)}
            className={SETTINGS_SWITCH_CLASS}
          />
        </div>

        <div className="flex items-center justify-between gap-3">
          <span className="text-xs tracking-wide text-muted-foreground">
            {t("settings.version")} {version || "-"}
          </span>
          {updateState.kind === "available" ? (
            <Button
              size="sm"
              className="h-8 bg-[rgba(124,109,230,0.85)] text-primary-foreground hover:bg-[rgba(124,109,230,1)]"
              onClick={() => handleInstallUpdate()}
            >
              {t("settings.install_and_restart")}
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              className={cn("h-8", SURFACE_BTN_CLASS)}
              onClick={handleCheckUpdate}
              disabled={updateState.kind === "checking" || updateState.kind === "installing"}
            >
              {updateState.kind === "checking"
                ? t("settings.checking")
                : updateState.kind === "installing"
                  ? t("settings.installing")
                  : t("settings.check_for_updates")}
            </Button>
          )}
        </div>

        {updateState.kind === "available" ? (
          <p className="text-xs text-[rgba(167,139,250,1)]">
            {t("settings.update_available", { version: updateState.info.version })}
          </p>
        ) : null}
        {updateState.kind === "up_to_date" ? (
          <p className="text-xs text-muted-foreground">{t("settings.up_to_date")}</p>
        ) : null}
        {updateState.kind === "failed" ? (
          <p className="text-xs text-destructive">{t("settings.check_failed")}</p>
        ) : null}
        {updateState.kind === "installing" && (
          <div className="mt-2">
            <div className="popup-progress-track h-1 w-full overflow-hidden rounded-[2px]">
              <div
                className={cn(
                  "h-full rounded-[2px] bg-[linear-gradient(90deg,rgba(168,85,247,0.95),rgba(236,72,153,0.95))] transition-[width] duration-[120ms] ease-out",
                  progressPercent === null &&
                    "w-[40%] animate-[popup-update-indeterminate_1.2s_ease-in-out_infinite]",
                )}
                style={progressPercent !== null ? { width: `${progressPercent}%` } : undefined}
              />
            </div>
            <div className="mt-1 text-xs tracking-[0.02em] text-muted-foreground">
              {progressPercent !== null
                ? t("settings.download_progress", { pct: progressPercent })
                : t("settings.download_progress_bytes", {
                    bytes: formatBytes(downloaded),
                  })}
            </div>
          </div>
        )}

        <Separator className={DIVIDER_CLASS} />

        <div className="space-y-2">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <Label className="text-sm text-foreground">{t("settings.diagnostics")}</Label>
              <p className="text-xs leading-relaxed text-muted-foreground">
                {t("settings.diagnostics_desc")}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className={cn("h-8 shrink-0", SURFACE_BTN_CLASS)}
              onClick={handleExportDiagnostics}
              disabled={diagnosticsState.kind === "exporting"}
            >
              {diagnosticsState.kind === "exporting"
                ? t("settings.diagnostics_exporting")
                : t("settings.diagnostics_export")}
            </Button>
          </div>
          {diagnosticsState.kind === "done" ? (
            <p
              className="truncate text-xs text-[var(--success-text)]"
              title={diagnosticsState.path}
            >
              {t("settings.diagnostics_done", { path: diagnosticsState.path })}
            </p>
          ) : null}
          {diagnosticsState.kind === "failed" ? (
            <p className="text-xs text-destructive">{t("settings.diagnostics_failed")}</p>
          ) : null}
        </div>

        <Separator className={DIVIDER_CLASS} />

        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <Label className="text-sm text-foreground">{t("settings.feedback")}</Label>
            <p className="text-xs leading-relaxed text-muted-foreground">
              {t("settings.feedback_desc")}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className={cn("h-8 shrink-0", SURFACE_BTN_CLASS)}
            onClick={handleFeedback}
          >
            {t("settings.feedback_button")}
          </Button>
        </div>
      </div>
    </SettingsSection>
  );
}
