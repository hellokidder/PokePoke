import { useState } from "react";
import { cn } from "../lib/utils";
import { POPUP_CLOSE_ANCHOR_CLASS, POPUP_CLOSE_INLINE_CLASS } from "./popupShared";

export function PopupCloseButton({
  onClick,
  disabled,
  ariaLabel,
  title,
  className,
  compact = false,
  inline = false,
}: {
  onClick: () => void;
  disabled?: boolean;
  ariaLabel: string;
  title: string;
  className?: string;
  compact?: boolean;
  inline?: boolean;
}) {
  const [hovered, setHovered] = useState(false);
  const showHover = hovered && !disabled;
  const iconSize = compact || inline ? 11 : 13;

  return (
    <button
      type="button"
      className={cn(
        "absolute z-[10] flex items-center justify-center",
        inline ? POPUP_CLOSE_INLINE_CLASS : POPUP_CLOSE_ANCHOR_CLASS,
        inline ? "p-1" : compact ? "p-1.5" : "p-2",
        "cursor-pointer",
        "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-red-400/50",
        "disabled:cursor-not-allowed disabled:opacity-30",
        className,
      )}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      disabled={disabled}
      aria-label={ariaLabel}
      title={title}
    >
      <span
        className={cn(
          "pointer-events-none grid shrink-0 place-items-center rounded-full transition-[background-color,color,box-shadow] duration-150",
          inline ? "size-[18px] bg-white/[0.06]" : "panel-base shadow-[0_2px_8px_rgba(0,0,0,0.28)]",
          compact && !inline ? "size-5" : !inline ? "size-6" : undefined,
          showHover
            ? "bg-red-500/25 text-red-400 shadow-[0_0_10px_rgba(248,113,113,0.35)]"
            : "text-[var(--text-tertiary)]",
        )}
      >
        <svg
          className="block shrink-0"
          width={iconSize}
          height={iconSize}
          viewBox="0 0 14 14"
          fill="none"
          aria-hidden="true"
        >
          <path
            d="M3.5 3.5L10.5 10.5M10.5 3.5L3.5 10.5"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </svg>
      </span>
    </button>
  );
}
