import { Badge } from "../components/ui/badge";
import { useT } from "../i18n/context";
import SourceIcon from "../icons/SourceIcon";
import { cn } from "../lib/utils";
import type { Session, SessionStatus } from "../types";
import { projectName, sourceLabel, workspacePath } from "./sessionPanelHelpers";

const STATUS_DOT: Record<SessionStatus, { color: string; label: string; breathing: boolean }> = {
  running: { color: "var(--status-running)", label: "running", breathing: true },
  pending: { color: "var(--status-pending)", label: "pending", breathing: true },
  idle: { color: "var(--status-idle)", label: "idle", breathing: false },
  last_failed: { color: "var(--status-failed)", label: "last_failed", breathing: false },
};

function StatusDot({ status }: { status: SessionStatus }) {
  const t = useT();
  const c = STATUS_DOT[status];
  return (
    <span
      className="relative inline-flex h-2.5 w-2.5 shrink-0 items-center justify-center lens-halo"
      title={t(`status.${c.label}`)}
    >
      <span
        className={cn(
          "absolute inset-[-6px] rounded-full opacity-30",
          c.breathing && "animate-lens-halo",
        )}
        style={{
          background: `radial-gradient(circle, ${c.color}66 0%, transparent 70%)`,
        }}
        aria-hidden="true"
      />
      <span
        className="relative h-[7px] w-[7px] rounded-full"
        style={{
          background: c.color,
          boxShadow: `0 0 6px ${c.color}aa`,
        }}
      />
    </span>
  );
}

function sessionCardClass(active: boolean): string {
  return cn(
    "group relative flex w-full items-start gap-3 rounded-xl px-3 py-2.5 cursor-pointer card-base",
    // button 语义保留可访问性;appearance-none 去掉系统按钮皮,勿用 p-0/bg-transparent/border-0 盖掉 card-base
    "appearance-none text-left font-inherit",
    active
      ? "border-[var(--brand-purple)]/40 shadow-[0_0_0_1px_rgba(109,79,232,0.12)]"
      : "dark:opacity-70 dark:hover:opacity-90 opacity-95 hover:opacity-100",
  );
}

export function SessionPanelCard({
  session,
  active,
  timeAgo,
  onClick,
}: {
  session: Session;
  active: boolean;
  timeAgo: (dateStr: string) => string;
  onClick: () => void;
}) {
  return (
    <button type="button" className={sessionCardClass(active)} onClick={onClick}>
      <div className="pt-0.5 shrink-0">
        <SourceIcon source={session.source} status={session.status} colorSeed={session.task_id} />
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between gap-2 mb-0.5 min-h-[20px]">
          <span className="truncate font-semibold text-[14px] leading-tight text-[var(--text-primary)]">
            {projectName(session)}
          </span>
          <div className="flex items-center gap-1.5 shrink-0">
            <Badge
              variant="outline"
              className="px-1.5 py-0 h-[18px] border-[var(--border-color)] bg-[var(--bg-tertiary)] text-[10px] font-medium tracking-wide text-[var(--text-secondary)]"
            >
              {sourceLabel(session)}
            </Badge>
            <StatusDot status={session.status} />
          </div>
        </div>
        <div className="truncate min-h-[17px] text-[12px] leading-[1.4] text-[var(--text-tertiary)]">
          {workspacePath(session) || "\u00a0"}
        </div>
        <div className="mt-0.5 min-h-[15px] text-[11px] leading-[1.4] text-[var(--text-tertiary)]">
          {timeAgo(session.updated_at)}
        </div>
      </div>
    </button>
  );
}
