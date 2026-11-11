import { cn } from "../lib/utils";

export function formatQuietRemaining(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

interface QuietModeProgressRingProps {
  remainingSecs: number;
  totalSecs: number;
  size?: "sm" | "lg";
  className?: string;
}

export function QuietModeProgressRing({
  remainingSecs,
  totalSecs,
  size = "lg",
  className,
}: QuietModeProgressRingProps) {
  const safeTotal = Math.max(1, totalSecs);
  const progress = Math.min(1, Math.max(0, remainingSecs / safeTotal));
  const radius = size === "lg" ? 46 : 30;
  const stroke = size === "lg" ? 3.5 : 2.5;
  const dim = (radius + stroke) * 2 + 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - progress);
  const center = dim / 2;

  return (
    <div
      className={cn("relative inline-flex shrink-0 items-center justify-center", className)}
      style={{ width: dim, height: dim }}
    >
      <svg className="absolute -rotate-90" width={dim} height={dim} aria-hidden="true">
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke="var(--border-color)"
          strokeWidth={stroke}
          opacity={0.45}
        />
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke="var(--brand-blue)"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          style={{
            filter: "drop-shadow(0 0 8px color-mix(in srgb, var(--brand-blue) 28%, transparent))",
          }}
          className="transition-[stroke-dashoffset] duration-1000 ease-linear"
        />
      </svg>
      <span
        className={cn(
          "relative font-semibold tabular-nums tracking-tight text-[var(--brand-blue)]",
          size === "lg" ? "text-[26px] leading-none" : "text-sm leading-none",
        )}
      >
        {formatQuietRemaining(remainingSecs)}
      </span>
    </div>
  );
}
