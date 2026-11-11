import SourceIcon from "../icons/SourceIcon";
import { cn } from "../lib/utils";

export function AlertBanner({
  variant,
  children,
}: {
  variant: "danger" | "warning";
  children: React.ReactNode;
}) {
  const accentColor =
    variant === "danger" ? "var(--alert-danger-accent)" : "var(--alert-warning-accent)";

  return (
    <div
      className={cn(
        "relative mx-[18px] mt-3 mb-2 flex flex-col gap-1.5 rounded-xl border px-3.5 py-2.5 overflow-hidden",
        "animate-fade-in",
        variant === "danger" && "border-[var(--alert-danger-border)] bg-[var(--alert-danger-bg)]",
        variant === "warning" &&
          "border-[var(--alert-warning-border)] bg-[var(--alert-warning-bg)]",
      )}
    >
      <div
        className="absolute left-0 top-0 bottom-0 w-1"
        style={{ background: accentColor }}
        aria-hidden="true"
      />
      <div className="relative z-10">{children}</div>
    </div>
  );
}

export function EmptyState({ message }: { message: string }) {
  return (
    <div className="flex h-full min-h-[260px] flex-col items-center justify-center px-6 text-center">
      <div className="relative h-16 w-16 animate-monster-sway opacity-90">
        <SourceIcon source={null} status="idle" colorSeed="empty-state" />
      </div>
      <div className="mt-4 text-[14px] font-medium text-[var(--text-secondary)]">{message}</div>
    </div>
  );
}
