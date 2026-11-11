import type { ReactNode } from "react";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "../lib/utils";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { Label } from "../components/ui/label";
import { Separator } from "../components/ui/separator";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../components/ui/dropdown-menu";

interface PendingUpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
  content_length: number | null;
}

interface CheckUpdateResult {
  available: boolean;
  info: PendingUpdateInfo | null;
}

interface UpdateProgressPayload {
  downloaded: number;
  total: number | null;
  phase: "downloading" | "download_finished";
}

type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up_to_date" }
  | { kind: "available"; info: PendingUpdateInfo }
  | { kind: "installing" }
  | { kind: "failed" };

type DiagnosticsState =
  | { kind: "idle" }
  | { kind: "exporting" }
  | { kind: "done"; path: string }
  | { kind: "failed" };

type PopupPosition = "top-right" | "top-left" | "bottom-right" | "bottom-left";
type PopupSize = "default" | "mini";
type PopupScreenStrategy = "terminal" | "primary";
type TrayIconStyle = "colored" | "white";
type UiTheme = "light" | "dark" | "system";

interface CustomSoundInfo {
  id: string;
  display_name: string;
  value: string;
}

interface Settings {
  sound_pending: string;
  sound_idle: string;
  sound_failed: string;
  locale: string;
  ui_theme: UiTheme;
  notifications_enabled: boolean;
  popup_position: PopupPosition;
  popup_size: PopupSize;
  popup_screen_strategy: PopupScreenStrategy;
  tray_icon_style: TrayIconStyle;
  popup_auto_close_secs: number | null;
  focus_suppression_enabled: boolean;
  popup_shortcut_modifiers: string;
  quiet_mode_shortcut: string;
  update_auto_check: boolean;
  update_skip_version: string | null;
}

const AUTO_CLOSE_OPTIONS: (number | null)[] = [null, 5, 15, 30, 60, 180];

const HOTKEY_INPUT_CLASS = cn(
  "flex h-9 w-auto shrink-0 items-center rounded-md border px-3 font-mono text-sm",
  "card-base cursor-pointer text-[var(--text-primary)]",
);

type TabKey = "general" | "popup" | "integrations" | "about";
const TABS: TabKey[] = ["general", "popup", "integrations", "about"];

// 外层:透明 + overflow-hidden 裁剪圆角(同 Popup 契约,禁止外向 box-shadow)
const SETTINGS_CONTAINER_CLASS = cn(
  "relative h-full w-full overflow-hidden rounded-[14px] bg-transparent",
  "text-[13px]",
);

// 内层:实心底 + 仅 inset 高光,不用 border / 外向 shadow
const SETTINGS_BODY_CLASS = cn(
  "relative z-[1] flex h-full w-full flex-col overflow-hidden rounded-[14px]",
  "bg-[var(--bg-primary-solid)]",
  "shadow-[inset_0_1px_0_var(--inset-highlight)]",
);

// 主题感知分隔线与次级按钮
const DIVIDER_CLASS = "bg-[var(--divider)]";
const HEADER_BORDER_CLASS = "border-[var(--divider)]";
const SURFACE_BTN_CLASS = cn(
  "border-[var(--border-color)] bg-[var(--surface-muted)]",
  "hover:bg-[var(--surface-hover)]",
);
const SURFACE_INPUT_CLASS = "surface-input font-mono text-xs";

// 品牌渐变标题
const SETTINGS_TITLE_CLASS = cn(
  "m-0 text-[17px] font-bold tracking-[0.02em]",
  "brand-gradient-text",
);

// Switch 样式
const SETTINGS_SWITCH_CLASS = cn(
  "data-[state=checked]:bg-[var(--brand-purple)]/85",
  "data-[state=unchecked]:bg-[var(--border-color)]",
  "focus-visible:ring-ring/50",
);

// 下拉触发器
const SELECT_TRIGGER_CLASS = cn(
  "h-9 w-full justify-between card-base font-normal text-[var(--text-primary)]",
);

// 下拉内容 -- Portal 到 body,不能用 card-base 半透明底;须用 solid 色
const DROPDOWN_CONTENT_CLASS = cn(
  "border border-[var(--border-color)] shadow-[var(--shadow-medium)]",
  "bg-[var(--bg-primary-solid)] text-[var(--text-primary)]",
);

const TOGGLE_GROUP_CLASS = "flex w-full gap-2";

// Toggle Item
const TOGGLE_ITEM_CLASS = cn(
  "flex-1 rounded-md card-base text-[var(--text-secondary)]",
  "data-[state=on]:border-[var(--brand-purple)]/40 data-[state=on]:bg-[var(--brand-purple)]/15",
  "data-[state=on]:text-[var(--text-primary)]",
);

export const WHITE_TRAY_ICON_IMAGE_CLASS = "drop-shadow-[0_1px_1px_rgba(15,15,35,0.45)]";

function SettingsSection({
  title,
  description,
  children,
  showSeparator = true,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  showSeparator?: boolean;
}) {
  return (
    <>
      <section className="space-y-3 py-4 first:pt-2">
        <div className="space-y-1">
          <Label className="text-sm font-semibold text-[var(--text-primary)]">{title}</Label>
          {description ? (
            <p className="text-xs leading-relaxed text-[var(--text-secondary)]">{description}</p>
          ) : null}
        </div>
        {children}
      </section>
      {showSeparator ? <Separator className="bg-[var(--border-color)]" /> : null}
    </>
  );
}

function SettingsSelect<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  className?: string;
}) {
  const selected = options.find((option) => option.value === value);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" className={cn(SELECT_TRIGGER_CLASS, className)}>
          <span className="truncate">{selected?.label ?? value}</span>
          <ChevronDown className="h-4 w-4 shrink-0 opacity-50" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className={cn(DROPDOWN_CONTENT_CLASS, "w-[var(--radix-dropdown-menu-trigger-width)]")}
      >
        {options.map((option) => (
          <DropdownMenuItem
            key={option.value}
            className="cursor-pointer focus:bg-[var(--surface-hover)]"
            onClick={() => onChange(option.value)}
          >
            <span className="flex-1">{option.label}</span>
            {option.value === value ? (
              <Check className="h-4 w-4 text-[rgba(167,139,250,1)]" aria-hidden="true" />
            ) : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function SettingsToggleRow({
  title,
  description,
  checked,
  onCheckedChange,
}: {
  title: string;
  description?: string;
  checked: boolean;
  onCheckedChange: (value: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0 space-y-1">
        <Label className="text-sm font-semibold text-foreground">{title}</Label>
        {description ? (
          <p className="text-xs leading-relaxed text-muted-foreground">{description}</p>
        ) : null}
      </div>
      <Switch
        checked={checked}
        onCheckedChange={onCheckedChange}
        className={cn(SETTINGS_SWITCH_CLASS, "mt-0.5 shrink-0")}
      />
    </div>
  );
}

export function invokeErrorMessage(error: unknown): string {
  if (typeof error === "string") {
    if (error.startsWith('"') && error.endsWith('"')) {
      try {
        return JSON.parse(error) as string;
      } catch {
        return error;
      }
    }
    return error;
  }
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { message: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

export function customSoundErrorText(raw: string, t: (key: string) => string): string {
  const code = invokeErrorMessage(raw)
    .trim()
    .replace(/^Error:\s*/i, "")
    .trim();
  if (code === "cancelled") return "";
  const known = [
    "unsupported_format",
    "file_too_large",
    "duration_too_long",
    "too_many_sounds",
    "total_size_exceeded",
    "in_use",
    "not_found",
  ] as const;
  const matched = known.find((key) => code === key || code.includes(key));
  switch (matched ?? code) {
    case "unsupported_format":
      return t("settings.sound_custom_error_unsupported");
    case "file_too_large":
      return t("settings.sound_custom_error_too_large");
    case "duration_too_long":
      return t("settings.sound_custom_error_too_long");
    case "too_many_sounds":
      return t("settings.sound_custom_error_too_many");
    case "total_size_exceeded":
      return t("settings.sound_custom_error_total_size");
    case "in_use":
      return t("settings.sound_custom_error_in_use");
    case "not_found":
      return t("settings.sound_custom_error_not_found");
    default:
      return t("settings.sound_custom_error_generic");
  }
}

export type {
  PendingUpdateInfo,
  CheckUpdateResult,
  UpdateProgressPayload,
  UpdateState,
  DiagnosticsState,
  PopupPosition,
  PopupSize,
  PopupScreenStrategy,
  TrayIconStyle,
  UiTheme,
  Settings,
  CustomSoundInfo,
  TabKey,
};

export {
  AUTO_CLOSE_OPTIONS,
  HOTKEY_INPUT_CLASS,
  TABS,
  SETTINGS_CONTAINER_CLASS,
  SETTINGS_BODY_CLASS,
  DIVIDER_CLASS,
  HEADER_BORDER_CLASS,
  SURFACE_BTN_CLASS,
  SURFACE_INPUT_CLASS,
  SETTINGS_TITLE_CLASS,
  SETTINGS_SWITCH_CLASS,
  TOGGLE_GROUP_CLASS,
  TOGGLE_ITEM_CLASS,
};

export { SettingsSection, SettingsSelect, SettingsToggleRow };
