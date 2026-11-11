import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-react";
import type { RefObject } from "react";
import { useState } from "react";
import { Button } from "../components/ui/button";
import { Label } from "../components/ui/label";
import { Separator } from "../components/ui/separator";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { useT } from "../i18n/context";
import trayIconColored from "../icons/tray-default.png";
import trayIconWhite from "../icons/tray-default-white.png";
import {
  DEFAULT_POPUP_HOTKEY_MODIFIERS,
  DEFAULT_QUIET_MODE_SHORTCUT,
  formatFullShortcut,
  formatPopupShortcutModPrefix,
} from "../lib/shortcutModifiers";
import { cn } from "../lib/utils";
import {
  type CustomSoundInfo,
  customSoundErrorText,
  DIVIDER_CLASS,
  HOTKEY_INPUT_CLASS,
  invokeErrorMessage,
  type Settings,
  SettingsSection,
  SettingsSelect,
  SURFACE_BTN_CLASS,
  TOGGLE_GROUP_CLASS,
  TOGGLE_ITEM_CLASS,
  type TrayIconStyle,
  type UiTheme,
  WHITE_TRAY_ICON_IMAGE_CLASS,
} from "./shared";

interface GeneralTabProps {
  settings: Settings;
  saveField: (key: keyof Settings, value: unknown) => Promise<void>;
  sounds: string[];
  bundledSounds: string[];
  customSounds: CustomSoundInfo[];
  onCustomSoundsChange: () => Promise<void>;
  hotkeyRecording: boolean;
  setHotkeyRecording: (value: boolean) => void;
  hotkeyButtonRef: RefObject<HTMLButtonElement | null>;
  quietHotkeyRecording: boolean;
  setQuietHotkeyRecording: (value: boolean) => void;
  quietHotkeyButtonRef: RefObject<HTMLButtonElement | null>;
}

type SoundField = "sound_pending" | "sound_idle" | "sound_failed";

const BUILTIN_SOUND_LABEL_KEYS = {
  waiting: "settings.sound_builtin_waiting",
  completed: "settings.sound_builtin_completed",
  failed: "settings.sound_builtin_failed",
} as const;

function builtinSoundLabel(t: (key: string) => string, name: string): string {
  const key = BUILTIN_SOUND_LABEL_KEYS[name as keyof typeof BUILTIN_SOUND_LABEL_KEYS];
  if (key) return t(key);
  const fallbackKey = `settings.sound_builtin_${name}`;
  const translated = t(fallbackKey);
  return translated === fallbackKey ? name : translated;
}

function SoundPicker({
  label,
  value,
  soundOptions,
  onChange,
  previewLabel,
}: {
  label: string;
  value: string;
  soundOptions: { value: string; label: string }[];
  onChange: (value: string) => void;
  previewLabel: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs font-medium text-[var(--text-secondary)]">{label}</Label>
      <div className="flex items-center gap-2">
        <SettingsSelect
          className="flex-1"
          value={value}
          options={soundOptions}
          onChange={(next) => {
            onChange(next);
            if (next !== "mute") {
              void invoke("preview_sound", { sound: next }).catch(() => {});
            }
          }}
        />
        {value !== "mute" ? (
          <Button
            variant="outline"
            size="sm"
            className={cn("h-9 shrink-0", SURFACE_BTN_CLASS)}
            onClick={() => void invoke("preview_sound", { sound: value }).catch(() => {})}
          >
            {previewLabel}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export function GeneralTab({
  settings,
  saveField,
  sounds,
  bundledSounds,
  customSounds,
  onCustomSoundsChange,
  hotkeyRecording,
  setHotkeyRecording,
  hotkeyButtonRef,
  quietHotkeyRecording,
  setQuietHotkeyRecording,
  quietHotkeyButtonRef,
}: GeneralTabProps) {
  const t = useT();
  const [customSoundMessage, setCustomSoundMessage] = useState<string | null>(null);
  const [customSoundBusy, setCustomSoundBusy] = useState(false);

  const soundOptions = [
    { value: "mute", label: t("settings.mute") },
    ...bundledSounds.map((sound) => ({
      value: `builtin:${sound}`,
      label: builtinSoundLabel(t, sound),
    })),
    ...customSounds.map((sound) => ({
      value: sound.value,
      label: sound.display_name,
    })),
    ...sounds.map((sound) => ({
      value: `system:${sound}`,
      label: sound,
    })),
  ];

  const handleImportCustomSound = async () => {
    setCustomSoundBusy(true);
    setCustomSoundMessage(null);
    try {
      const imported = await invoke<CustomSoundInfo>("import_custom_sound");
      await onCustomSoundsChange();
      await invoke("preview_sound", { sound: imported.value });
    } catch (e) {
      const message = customSoundErrorText(invokeErrorMessage(e), t);
      if (message) setCustomSoundMessage(message);
    } finally {
      setCustomSoundBusy(false);
    }
  };

  const handleDeleteCustomSound = async (id: string) => {
    setCustomSoundBusy(true);
    setCustomSoundMessage(null);
    try {
      await invoke("delete_custom_sound", { id });
      await onCustomSoundsChange();
    } catch (e) {
      setCustomSoundMessage(customSoundErrorText(invokeErrorMessage(e), t));
    } finally {
      setCustomSoundBusy(false);
    }
  };

  const soundFields: { key: SoundField; label: string }[] = [
    { key: "sound_pending", label: t("settings.sound_pending") },
    { key: "sound_idle", label: t("settings.sound_idle") },
    { key: "sound_failed", label: t("settings.sound_failed") },
  ];

  return (
    <>
      <SettingsSection title={t("settings.sound")} description={t("settings.sound_desc")}>
        <div className="space-y-3">
          {soundFields.map(({ key, label }) => (
            <SoundPicker
              key={key}
              label={label}
              value={settings[key]}
              soundOptions={soundOptions}
              previewLabel={t("settings.preview")}
              onChange={(value) => saveField(key, value)}
            />
          ))}
        </div>
        <Separator className={cn("my-4", DIVIDER_CLASS)} />
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-2">
            <div>
              <p className="text-xs font-medium text-[var(--text-secondary)]">
                {t("settings.sound_custom")}
              </p>
              <p className="text-[11px] text-[var(--text-tertiary)]">
                {t("settings.sound_custom_desc")}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className={cn("h-9 shrink-0", SURFACE_BTN_CLASS)}
              disabled={customSoundBusy}
              onClick={() => void handleImportCustomSound()}
            >
              {t("settings.sound_custom_import")}
            </Button>
          </div>
          {customSoundMessage ? (
            <div
              role="alert"
              className={cn(
                "flex items-start justify-between gap-2 rounded-md",
                "border border-[var(--alert-danger-border)] bg-[var(--alert-danger-bg)]",
                "px-2.5 py-2 text-xs leading-relaxed text-[var(--alert-danger-text)]",
              )}
            >
              <span className="min-w-0">{customSoundMessage}</span>
              <button
                type="button"
                aria-label={t("settings.sound_custom_error_dismiss")}
                className={cn(
                  "shrink-0 cursor-pointer rounded p-0.5 text-[var(--alert-danger-accent)]",
                  "transition-colors hover:bg-[var(--alert-danger-border)]/40",
                )}
                onClick={() => setCustomSoundMessage(null)}
              >
                <X size={13} strokeWidth={2} aria-hidden="true" />
              </button>
            </div>
          ) : null}
          {customSounds.length === 0 ? (
            <p className="text-[11px] text-[var(--text-tertiary)]">
              {t("settings.sound_custom_empty")}
            </p>
          ) : (
            <ul className="space-y-1.5">
              {customSounds.map((sound) => (
                <li
                  key={sound.id}
                  className="flex items-center justify-between gap-2 rounded-md border border-[var(--border-color)] bg-[var(--surface-muted)] px-2.5 py-1.5"
                >
                  <span className="min-w-0 truncate text-xs text-[var(--text-primary)]">
                    {sound.display_name}
                  </span>
                  <div className="flex shrink-0 items-center gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      className={cn("h-7 px-2 text-xs", SURFACE_BTN_CLASS)}
                      disabled={customSoundBusy}
                      onClick={() =>
                        void invoke("preview_sound", { sound: sound.value }).catch(() => {})
                      }
                    >
                      {t("settings.preview")}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className={cn("h-7 px-2 text-xs", SURFACE_BTN_CLASS)}
                      disabled={customSoundBusy}
                      onClick={() => void handleDeleteCustomSound(sound.id)}
                    >
                      {t("settings.sound_custom_delete")}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.language")} description={t("settings.language_desc")}>
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={1}
          value={settings.locale}
          onValueChange={(value) => value && saveField("locale", value)}
          className={TOGGLE_GROUP_CLASS}
        >
          <ToggleGroupItem value="zh" className={TOGGLE_ITEM_CLASS}>
            中文
          </ToggleGroupItem>
          <ToggleGroupItem value="en" className={TOGGLE_ITEM_CLASS}>
            English
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingsSection>

      <SettingsSection title={t("settings.ui_theme")} description={t("settings.ui_theme_desc")}>
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={1}
          value={settings.ui_theme}
          onValueChange={(value) => value && saveField("ui_theme", value as UiTheme)}
          className={TOGGLE_GROUP_CLASS}
        >
          <ToggleGroupItem value="light" className={TOGGLE_ITEM_CLASS}>
            {t("settings.ui_theme_light")}
          </ToggleGroupItem>
          <ToggleGroupItem value="dark" className={TOGGLE_ITEM_CLASS}>
            {t("settings.ui_theme_dark")}
          </ToggleGroupItem>
          <ToggleGroupItem value="system" className={TOGGLE_ITEM_CLASS}>
            {t("settings.ui_theme_system")}
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingsSection>

      <SettingsSection
        title={t("settings.popup_hotkey")}
        description={t("settings.popup_hotkey_desc")}
      >
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1.5">
            <button
              ref={hotkeyButtonRef}
              type="button"
              className={cn(
                HOTKEY_INPUT_CLASS,
                hotkeyRecording &&
                  "border-[var(--brand-purple)]/50 ring-1 ring-[var(--brand-purple)]/30",
              )}
              onClick={() => setHotkeyRecording(true)}
              onBlur={() => setHotkeyRecording(false)}
            >
              {hotkeyRecording
                ? t("settings.hotkey_press_keys")
                : formatPopupShortcutModPrefix(
                    settings.popup_shortcut_modifiers || DEFAULT_POPUP_HOTKEY_MODIFIERS,
                  )}
            </button>
            <span
              className="select-none font-mono text-sm tracking-[0.02em] text-[var(--text-secondary)]"
              aria-hidden="true"
            >
              +N
            </span>
          </div>
          {settings.popup_shortcut_modifiers !== DEFAULT_POPUP_HOTKEY_MODIFIERS &&
          !hotkeyRecording ? (
            <Button
              variant="outline"
              size="sm"
              className={cn("h-9 shrink-0", SURFACE_BTN_CLASS)}
              onClick={() => saveField("popup_shortcut_modifiers", DEFAULT_POPUP_HOTKEY_MODIFIERS)}
            >
              {t("settings.hotkey_reset_default")}
            </Button>
          ) : null}
        </div>
      </SettingsSection>

      <SettingsSection
        title={t("settings.quiet_mode_hotkey")}
        description={t("settings.quiet_mode_hotkey_desc")}
      >
        <div className="flex items-center gap-2">
          <button
            ref={quietHotkeyButtonRef}
            type="button"
            className={cn(
              HOTKEY_INPUT_CLASS,
              quietHotkeyRecording &&
                "border-[var(--brand-purple)]/50 ring-1 ring-[var(--brand-purple)]/30",
            )}
            onClick={() => setQuietHotkeyRecording(true)}
            onBlur={() => setQuietHotkeyRecording(false)}
          >
            {quietHotkeyRecording
              ? t("settings.hotkey_press_keys")
              : formatFullShortcut(settings.quiet_mode_shortcut || DEFAULT_QUIET_MODE_SHORTCUT)}
          </button>
          {settings.quiet_mode_shortcut !== DEFAULT_QUIET_MODE_SHORTCUT && !quietHotkeyRecording ? (
            <Button
              variant="outline"
              size="sm"
              className={cn("h-9 shrink-0", SURFACE_BTN_CLASS)}
              onClick={() => saveField("quiet_mode_shortcut", DEFAULT_QUIET_MODE_SHORTCUT)}
            >
              {t("settings.hotkey_reset_default")}
            </Button>
          ) : null}
        </div>
      </SettingsSection>

      <SettingsSection
        title={t("settings.tray_icon_style")}
        description={t("settings.tray_icon_style_desc")}
        showSeparator={false}
      >
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={1}
          value={settings.tray_icon_style}
          onValueChange={(value) => value && saveField("tray_icon_style", value as TrayIconStyle)}
          className="flex w-fit gap-2"
        >
          {/* 沿用 radio 默认浅色按钮样式,内容换图。白图在浅底反差弱属固有 trade-off */}
          <ToggleGroupItem
            value="colored"
            className={cn(TOGGLE_ITEM_CLASS, "h-9 w-12 px-0")}
            title={t("settings.tray_icon_colored")}
            aria-label={t("settings.tray_icon_colored")}
          >
            <img
              src={trayIconColored}
              alt=""
              className="pointer-events-none h-[18px] w-[18px] select-none"
              style={{ imageRendering: "pixelated" }}
              draggable={false}
            />
          </ToggleGroupItem>
          <ToggleGroupItem
            value="white"
            className={cn(TOGGLE_ITEM_CLASS, "h-9 w-12 px-0")}
            title={t("settings.tray_icon_white")}
            aria-label={t("settings.tray_icon_white")}
          >
            <img
              src={trayIconWhite}
              alt=""
              className={cn(
                "pointer-events-none h-[18px] w-[18px] select-none",
                WHITE_TRAY_ICON_IMAGE_CLASS,
              )}
              style={{ imageRendering: "pixelated" }}
              draggable={false}
            />
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingsSection>
    </>
  );
}
