import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle, Loader2 } from "lucide-react";
import { useCallback, useEffect, useReducer } from "react";
import { Button } from "../components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../components/ui/dialog";
import { Label } from "../components/ui/label";
import { Separator } from "../components/ui/separator";
import { useT } from "../i18n/context";
import { cn } from "../lib/utils";
import {
  canAct,
  type FlowMessage,
  flowInitialState,
  flowReducer,
  i18nMessage,
  rawMessage,
} from "./integrationsFlow";
import {
  connectionStatus,
  type IntegrationSource,
  integrationRowActions,
  type ManagedIntegrationStatus,
  presenceStatus,
} from "./integrationsLogic";

export type { IntegrationSource, ManagedIntegrationStatus };

// 次级按钮样式(原先复用 SettingsWindow 的 LICENSE_SECONDARY_BUTTON_CLASS,
// 2026-11 移除 license tab 后本地化,样式保持一致)。
const SECONDARY_BUTTON_CLASS = cn(
  "border-[var(--border-color)] bg-[var(--surface-muted)] hover:bg-[var(--surface-hover)]",
  "text-[var(--text-primary)] shadow-[var(--shadow-soft)]",
  "disabled:opacity-60 disabled:text-[var(--text-tertiary)]",
);

function integrationLabelKey(source: IntegrationSource): string {
  return `settings.integrations.${source}`;
}

function integrationCliKey(source: IntegrationSource): string {
  return `settings.integrations.${source}_cli`;
}

const STATUS_DOT: Record<ReturnType<typeof presenceStatus>["tone"], string> = {
  ok: "var(--status-running)",
  warn: "var(--status-pending)",
  bad: "var(--status-failed)",
  muted: "var(--text-tertiary)",
};

const STATUS_TEXT: Record<ReturnType<typeof presenceStatus>["tone"], string> = {
  ok: "text-[var(--text-primary)]",
  warn: "text-amber-700 dark:text-amber-400",
  bad: "text-red-600 dark:text-red-400",
  muted: "text-[var(--text-tertiary)]",
};

function StatusCell({
  label,
  tone,
  hint,
}: {
  label: string;
  tone: ReturnType<typeof presenceStatus>["tone"];
  hint?: string;
}) {
  return (
    <div className="flex items-center gap-1.5" title={hint}>
      <span
        className="h-1.5 w-1.5 shrink-0 rounded-full"
        style={{ background: STATUS_DOT[tone] }}
        aria-hidden
      />
      <span
        className={cn("text-[11px] font-medium leading-none whitespace-nowrap", STATUS_TEXT[tone])}
      >
        {label}
      </span>
    </div>
  );
}

const TABLE_CLASS = "w-full table-fixed border-collapse text-left";
const TH_CLASS =
  "border-b border-[var(--divider)] bg-[var(--surface-hover)]/60 px-3 py-2 text-[10px] font-medium text-[var(--text-tertiary)]";
const TD_CLASS = "px-3 py-2 align-middle";
const TR_CLASS = "border-b border-[var(--divider)] last:border-b-0";

const DISCONNECT_CONFIRM_POINTS = [
  "settings.integrations.disconnect_confirm_point_hooks",
  "settings.integrations.disconnect_confirm_point_sessions",
  "settings.integrations.disconnect_confirm_point_notify",
] as const;

const DISCONNECT_DANGER_BUTTON_CLASS = cn(
  "border border-[var(--alert-danger-border)] bg-[var(--status-failed)] text-white",
  "shadow-[var(--shadow-soft)] hover:bg-[var(--status-failed)]/90",
);

const ROW_BTN_CLASS = "h-7 px-2 text-[11px]";

export function IntegrationsTab() {
  const t = useT();
  // W11(brief 20260813-integrations-flow-reducer):异步编排状态机抽成纯 reducer
  // (integrationsFlow.ts),组件层只做 invoke/listen → dispatch。
  const [state, dispatch] = useReducer(flowReducer, undefined, flowInitialState);

  const loadAll = useCallback(async () => {
    const data = await invoke<ManagedIntegrationStatus[]>("list_managed_integrations");
    dispatch({ type: "rows_loaded", rows: data });
  }, []);

  useEffect(() => {
    let cancelled = false;
    loadAll()
      .catch((err) => console.error("list_managed_integrations failed", err))
      .finally(() => {
        if (!cancelled) dispatch({ type: "initial_loaded" });
      });
    return () => {
      cancelled = true;
    };
  }, [loadAll]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    void (async () => {
      unsubs.push(
        await listen("hook-drift-detected", () => {
          void loadAll();
        }),
      );
      unsubs.push(
        await listen("hook-drift-repaired", () => {
          void loadAll();
        }),
      );
      unsubs.push(
        await listen("cc-integration-updated", () => {
          void loadAll();
        }),
      );
      unsubs.push(
        await listen("sessions-updated", () => {
          void loadAll();
        }),
      );
    })();
    return () => {
      for (const u of unsubs) u();
    };
  }, [loadAll]);

  const refreshAll = async () => {
    dispatch({ type: "refresh_started" });
    try {
      const data = await invoke<ManagedIntegrationStatus[]>("list_managed_integrations");
      dispatch({ type: "refresh_done", rows: data });
    } catch {
      dispatch({ type: "refresh_failed" });
    }
  };

  const connectOne = async (source: IntegrationSource) => {
    dispatch({ type: "action_started", source, kind: "connect" });
    try {
      const result = await invoke<{ status?: string; message?: string }>(
        "connect_managed_integration",
        { source },
      );
      // 后端直返 message 是原始文案(raw);缺省回落 i18n action_failed。
      const message: FlowMessage =
        result.status !== "ok"
          ? result.message
            ? rawMessage(result.message)
            : i18nMessage("settings.integrations.action_failed")
          : null;
      await loadAll();
      dispatch({ type: "action_done", source, message });
    } catch {
      dispatch({ type: "action_failed", source });
    }
  };

  const disconnectOne = async (source: IntegrationSource) => {
    dispatch({ type: "action_started", source, kind: "disconnect" });
    try {
      const result = await invoke<{ status?: string; message?: string; sessions_removed?: number }>(
        "disconnect_managed_integration",
        { source },
      );
      let message: FlowMessage = null;
      if (result.status !== "ok") {
        message = result.message
          ? rawMessage(result.message)
          : i18nMessage("settings.integrations.action_failed");
      } else if (typeof result.sessions_removed === "number" && result.sessions_removed > 0) {
        message = i18nMessage("settings.integrations.disconnect_cleared_sessions", {
          n: result.sessions_removed,
        });
      }
      await loadAll();
      dispatch({ type: "action_done", source, message });
    } catch {
      dispatch({ type: "action_failed", source });
    }
  };

  const confirmDisconnect = async () => {
    if (!state.confirmSource) return;
    const source = state.confirmSource;
    dispatch({ type: "confirm_cancel" });
    await disconnectOne(source);
  };

  const repairOne = async (source: IntegrationSource) => {
    dispatch({ type: "action_started", source, kind: "repair" });
    try {
      if (source === "cc") {
        await invoke("repair_cc_integration");
      } else {
        await invoke("repair_hook_drift", { source });
      }
      await loadAll();
      dispatch({ type: "action_done", source, message: null });
    } catch {
      dispatch({ type: "action_failed", source });
    }
  };

  if (state.loading) {
    return (
      <p className="py-6 text-sm text-[var(--text-secondary)]">
        {t("settings.integrations.loading")}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between gap-3 py-1">
        <div className="min-w-0 space-y-1">
          <Label className="text-sm font-semibold text-[var(--text-primary)]">
            {t("settings.integrations.title")}
          </Label>
          <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
            {t("settings.integrations.desc")}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          className={cn("h-7 shrink-0 px-2.5 text-[11px]", SECONDARY_BUTTON_CLASS)}
          disabled={state.refreshingAll}
          onClick={() => void refreshAll()}
        >
          {state.refreshingAll ? (
            <>
              <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
              {t("settings.integrations.checking_all")}
            </>
          ) : (
            t("settings.integrations.check_all")
          )}
        </Button>
      </div>

      <div className="overflow-hidden rounded-lg border border-[var(--border-color)] bg-[var(--surface-muted)]">
        <table className={TABLE_CLASS}>
          <colgroup>
            <col />
            <col className="w-[4.75rem]" />
            <col className="w-[5.25rem]" />
            <col className="w-[7.5rem]" />
          </colgroup>
          <thead>
            <tr>
              <th className={cn(TH_CLASS, "font-medium")}>
                {t("settings.integrations.col_agent")}
              </th>
              <th className={TH_CLASS}>{t("settings.integrations.col_presence")}</th>
              <th className={TH_CLASS}>{t("settings.integrations.col_connection")}</th>
              <th className={cn(TH_CLASS, "text-right")}>
                {t("settings.integrations.col_actions")}
              </th>
            </tr>
          </thead>
          <tbody>
            {state.rows.map((row) => {
              const busy = state.rowBusy[row.source];
              const presence = presenceStatus(row);
              const connection = connectionStatus(row);
              const { canDisconnect, canRepair, canConnect } = integrationRowActions(row);
              const actionBusy = !canAct(state, row.source);

              return (
                <tr key={row.source} className={TR_CLASS}>
                  <td className={TD_CLASS}>
                    <div className="min-w-0">
                      <p className="truncate text-[13px] font-medium leading-tight text-[var(--text-primary)]">
                        {t(integrationLabelKey(row.source))}
                      </p>
                      <p className="truncate text-[10px] leading-tight text-[var(--text-tertiary)]">
                        {t(integrationCliKey(row.source))}
                      </p>
                    </div>
                  </td>
                  <td className={TD_CLASS}>
                    <StatusCell label={t(presence.labelKey)} tone={presence.tone} />
                  </td>
                  <td className={TD_CLASS}>
                    <StatusCell
                      label={t(connection.labelKey)}
                      tone={connection.tone}
                      hint={connection.hintKey ? t(connection.hintKey) : undefined}
                    />
                  </td>
                  <td className={cn(TD_CLASS, "text-right")}>
                    <div className="flex justify-end gap-1">
                      {canRepair ? (
                        <Button
                          size="sm"
                          className={ROW_BTN_CLASS}
                          disabled={actionBusy}
                          onClick={() => void repairOne(row.source)}
                        >
                          {busy === "repair"
                            ? t("settings.integrations.repairing")
                            : t("settings.integrations.repair")}
                        </Button>
                      ) : null}
                      {canConnect ? (
                        <Button
                          size="sm"
                          className={ROW_BTN_CLASS}
                          disabled={actionBusy}
                          onClick={() => void connectOne(row.source)}
                        >
                          {busy === "connect"
                            ? t("settings.integrations.connecting")
                            : t("settings.integrations.connect")}
                        </Button>
                      ) : null}
                      {canDisconnect ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className={cn(ROW_BTN_CLASS, SECONDARY_BUTTON_CLASS)}
                          disabled={actionBusy}
                          onClick={() => dispatch({ type: "confirm_open", source: row.source })}
                        >
                          {busy === "disconnect"
                            ? t("settings.integrations.disconnecting")
                            : t("settings.integrations.disconnect")}
                        </Button>
                      ) : null}
                      {!canRepair && !canConnect && !canDisconnect ? (
                        <span className="px-1 text-[11px] text-[var(--text-tertiary)]">—</span>
                      ) : null}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {state.message ? (
        <>
          <Separator className="bg-[var(--border-color)]" />
          <p className="text-xs text-[var(--text-secondary)]">
            {state.message.kind === "i18n"
              ? t(state.message.key, state.message.params)
              : state.message.text}
          </p>
        </>
      ) : null}

      <Dialog
        open={state.confirmSource !== null}
        onOpenChange={(open) => !open && dispatch({ type: "confirm_cancel" })}
      >
        <DialogContent hideClose className="gap-0 overflow-hidden p-0">
          <div className="border-b border-[var(--divider)] px-5 py-4">
            <DialogHeader className="space-y-0 text-left">
              <div className="flex items-start gap-3">
                <div
                  className={cn(
                    "flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px]",
                    "border border-[var(--alert-danger-border)] bg-[var(--alert-danger-bg)]",
                    "text-[var(--alert-danger-accent)]",
                  )}
                >
                  <AlertTriangle className="h-4 w-4" aria-hidden />
                </div>
                <div className="min-w-0 space-y-1 pt-0.5">
                  <DialogTitle>{t("settings.integrations.disconnect_confirm_title")}</DialogTitle>
                  <DialogDescription>
                    {state.confirmSource
                      ? t("settings.integrations.disconnect_confirm_lead", {
                          name: t(integrationLabelKey(state.confirmSource)),
                        })
                      : ""}
                  </DialogDescription>
                </div>
              </div>
            </DialogHeader>
          </div>

          <div className="space-y-3 px-5 py-4">
            <ul className="space-y-2.5">
              {DISCONNECT_CONFIRM_POINTS.map((key) => (
                <li
                  key={key}
                  className="flex gap-2.5 text-[13px] leading-relaxed text-[var(--text-secondary)]"
                >
                  <span
                    className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--alert-danger-accent)]"
                    aria-hidden
                  />
                  <span>{t(key)}</span>
                </li>
              ))}
            </ul>
            <p className="text-xs leading-relaxed text-[var(--text-tertiary)]">
              {t("settings.integrations.disconnect_confirm_footnote")}
            </p>
          </div>

          <DialogFooter
            className={cn(
              "gap-2 border-t border-[var(--divider)] bg-[var(--surface-muted)] px-5 py-3",
              "sm:flex-row sm:justify-end sm:space-x-0",
            )}
          >
            <Button
              variant="outline"
              className={SECONDARY_BUTTON_CLASS}
              onClick={() => dispatch({ type: "confirm_cancel" })}
            >
              {t("settings.integrations.disconnect_confirm_cancel")}
            </Button>
            <Button
              className={DISCONNECT_DANGER_BUTTON_CLASS}
              onClick={() => void confirmDisconnect()}
            >
              {t("settings.integrations.disconnect_confirm_action")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
