import { cn } from "../lib/utils";

export function PopupShortcutBadge({
  shortcutModPrefix,
  shortcutN,
  compact = false,
}: {
  shortcutModPrefix: string;
  shortcutN: number;
  compact?: boolean;
}) {
  return (
    <span
      className={cn(
        "popup-kbd inline-flex select-none items-center",
        compact
          ? "rounded-sm px-1 py-[1px] text-[8px] font-semibold tracking-[0.03em] leading-none"
          : "rounded-md px-1.5 py-[1px] text-[10px] font-semibold tracking-[0.04em]",
      )}
    >
      <span className="sr-only">
        Shortcut {shortcutModPrefix}
        {shortcutN + 1}
      </span>
      <span aria-hidden="true">
        {shortcutModPrefix}
        {shortcutN + 1}
      </span>
    </span>
  );
}
