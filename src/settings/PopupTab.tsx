import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import { Separator } from "../components/ui/separator";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { QuietModeSettingsCard } from "../quiet-mode/QuietModeSettingsCard";
import { DEFAULT_QUIET_MODE_SHORTCUT } from "../lib/shortcutModifiers";
import {
  AUTO_CLOSE_OPTIONS,
  DIVIDER_CLASS,
  TOGGLE_GROUP_CLASS,
  TOGGLE_ITEM_CLASS,
  SettingsSection,
  SettingsSelect,
  SettingsToggleRow,
  type Settings,
  type PopupPosition,
  type PopupScreenStrategy,
} from "./shared";

interface PopupTabProps {
  settings: Settings;
  saveField: (key: keyof Settings, value: unknown) => Promise<void>;
}

export function PopupTab({ settings, saveField }: PopupTabProps) {
  const t = useT();

  const autoCloseLabel = (v: number | null) =>
    v == null
      ? t("settings.popup_auto_close_never")
      : v < 60
        ? t("settings.popup_auto_close_seconds", { n: v })
        : t("settings.popup_auto_close_minutes", { n: v / 60 });

  const popupPositionOptions = (
    ["top-right", "top-left", "bottom-right", "bottom-left"] as PopupPosition[]
  ).map((pos) => ({
    value: pos,
    label: t(`settings.popup_position_${pos.replace("-", "_")}`),
  }));

  const autoCloseOptions = AUTO_CLOSE_OPTIONS.map((value) => ({
    value: String(value ?? "never"),
    label: autoCloseLabel(value),
  }));

  return (
    <>
      <section className="py-4 first:pt-2">
        <QuietModeSettingsCard
          quietModeShortcut={settings.quiet_mode_shortcut || DEFAULT_QUIET_MODE_SHORTCUT}
        />
      </section>
      <Separator className={DIVIDER_CLASS} />

      <section className="py-4">
        <SettingsToggleRow
          title={t("settings.focus_suppression")}
          description={t("settings.focus_suppression_desc")}
          checked={settings.focus_suppression_enabled}
          onCheckedChange={(value) => saveField("focus_suppression_enabled", value)}
        />
      </section>
      <Separator className={DIVIDER_CLASS} />

      <SettingsSection title={t("settings.popup_size")} description={t("settings.popup_size_desc")}>
        <div className="flex gap-3">
          {/* 标准尺寸预览卡片:头像+标题+pill、正文两行、footer 时间+⌘N,全部同一卡片内 */}
          <button
            type="button"
            className={cn(
              "flex flex-1 flex-col items-center gap-2 rounded-xl border-2 p-4 transition-all cursor-pointer",
              settings.popup_size === "default"
                ? "border-[var(--brand-purple)] bg-[var(--brand-purple)]/10"
                : "border-[var(--border-color)] bg-[var(--surface-muted)] hover:border-[var(--border-hover)]",
            )}
            onClick={() => saveField("popup_size", "default")}
          >
            <div className="flex w-full max-w-[180px] flex-col gap-1.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-primary)] px-2.5 py-2 shadow-sm">
              {/* 上半区:avatar 左 + (title 行 / body 两行) 右 */}
              <div className="flex gap-1.5">
                <div className="mt-0.5 h-5 w-5 shrink-0 rounded-md bg-[var(--brand-purple)]/30" />
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <div className="flex items-center gap-1">
                    <div className="h-2 flex-1 rounded-full bg-[var(--text-secondary)]/40" />
                    <div className="h-2 w-8 rounded-full bg-[var(--text-tertiary)]/30" />
                  </div>
                  <div className="h-1.5 w-full rounded-full bg-[var(--text-tertiary)]/25" />
                  <div className="h-1.5 w-3/4 rounded-full bg-[var(--text-tertiary)]/20" />
                </div>
              </div>
              {/* footer:time 左 + ⌘N 右 */}
              <div className="flex items-center justify-between">
                <div className="h-1.5 w-8 rounded-full bg-[var(--text-tertiary)]/25" />
                <div className="h-2.5 w-6 rounded-[3px] bg-[var(--text-tertiary)]/30" />
              </div>
            </div>
            <span className="text-xs font-medium text-[var(--text-primary)]">
              {t("settings.popup_size_default")}
            </span>
            <span className="text-[10px] text-[var(--text-tertiary)]">
              {t("settings.popup_size_default_desc")}
            </span>
          </button>

          {/* 迷你尺寸预览卡片:单行、无正文,footer 元素挤在同一行。
              preview 用 flex-1 撑高、内部 items-center,以便和标准卡片的
              文字标签在同一水平线上对齐 */}
          <button
            type="button"
            className={cn(
              "flex flex-1 flex-col items-center gap-2 rounded-xl border-2 p-4 transition-all cursor-pointer",
              settings.popup_size === "mini"
                ? "border-[var(--brand-purple)] bg-[var(--brand-purple)]/10"
                : "border-[var(--border-color)] bg-[var(--surface-muted)] hover:border-[var(--border-hover)]",
            )}
            onClick={() => saveField("popup_size", "mini")}
          >
            <div className="flex w-full flex-1 items-center justify-center">
              <div className="flex w-full max-w-[180px] items-center gap-1.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-primary)] px-2.5 py-1.5 shadow-sm">
                <div className="h-3 w-3 shrink-0 rounded-[4px] bg-[var(--brand-purple)]/30" />
                <div className="h-1.5 flex-1 rounded-full bg-[var(--text-secondary)]/40" />
                <div className="h-2 w-5 rounded-[3px] bg-[var(--text-tertiary)]/30" />
              </div>
            </div>
            <span className="text-xs font-medium text-[var(--text-primary)]">
              {t("settings.popup_size_mini")}
            </span>
            <span className="text-[10px] text-[var(--text-tertiary)]">
              {t("settings.popup_size_mini_desc")}
            </span>
          </button>
        </div>
      </SettingsSection>
      <Separator className={DIVIDER_CLASS} />

      <SettingsSection
        title={t("settings.popup_position")}
        description={t("settings.popup_position_desc")}
      >
        <SettingsSelect
          value={settings.popup_position}
          options={popupPositionOptions}
          onChange={(value) => saveField("popup_position", value)}
        />
      </SettingsSection>

      <SettingsSection
        title={t("settings.popup_auto_close")}
        description={t("settings.popup_auto_close_desc")}
      >
        <SettingsSelect
          value={String(settings.popup_auto_close_secs ?? "never")}
          options={autoCloseOptions}
          onChange={(value) =>
            saveField("popup_auto_close_secs", value === "never" ? null : Number(value))
          }
        />
      </SettingsSection>

      <SettingsSection
        title={t("settings.popup_screen")}
        description={t("settings.popup_screen_desc")}
        showSeparator={false}
      >
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={1}
          value={settings.popup_screen_strategy}
          onValueChange={(value) =>
            value && saveField("popup_screen_strategy", value as PopupScreenStrategy)
          }
          className={TOGGLE_GROUP_CLASS}
        >
          <ToggleGroupItem value="terminal" className={TOGGLE_ITEM_CLASS}>
            {t("settings.popup_screen_terminal")}
          </ToggleGroupItem>
          <ToggleGroupItem value="primary" className={TOGGLE_ITEM_CLASS}>
            {t("settings.popup_screen_primary")}
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingsSection>
    </>
  );
}
