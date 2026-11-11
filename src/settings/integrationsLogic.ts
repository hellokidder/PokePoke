export type IntegrationSource =
  | "cc"
  | "codex"
  | "cursor"
  | "opencode"
  | "pi"
  | "omp"
  | "kimi"
  | "grok"
  | "dsh";

export interface ManagedIntegrationStatus {
  source: IntegrationSource;
  agent_present: boolean;
  connected: boolean;
  hooks_configured: boolean;
  hooks_outdated: boolean;
  installed: boolean;
  issue: string | null;
  repair_available: boolean;
  disconnected: boolean;
  feature_enabled?: boolean | null;
}

export type StatusTone = "ok" | "warn" | "bad" | "muted";

export function presenceStatus(row: ManagedIntegrationStatus): {
  labelKey: string;
  tone: StatusTone;
} {
  if (row.agent_present) {
    return { labelKey: "settings.integrations.presence_yes", tone: "ok" };
  }
  return { labelKey: "settings.integrations.presence_no", tone: "muted" };
}

export function connectionStatus(row: ManagedIntegrationStatus): {
  labelKey: string;
  tone: StatusTone;
  hintKey?: string;
} {
  if (!row.agent_present) {
    return { labelKey: "settings.integrations.conn_unavailable", tone: "muted" };
  }
  if (row.disconnected) {
    return { labelKey: "settings.integrations.conn_manual_off", tone: "muted" };
  }
  if (row.connected) {
    return { labelKey: "settings.integrations.conn_connected", tone: "ok" };
  }
  if (row.hooks_outdated) {
    return { labelKey: "settings.integrations.conn_outdated", tone: "warn" };
  }
  if (row.source === "cc" && row.issue && row.issue !== "healthy") {
    return { labelKey: "settings.integrations.conn_needs_repair", tone: "bad" };
  }
  if (row.hooks_configured) {
    return { labelKey: "settings.integrations.conn_partial", tone: "warn" };
  }
  const hintKey =
    row.source === "codex" && row.feature_enabled === false
      ? "settings.integrations.codex_feature_off"
      : undefined;
  return {
    labelKey: "settings.integrations.conn_not_connected",
    tone: "muted",
    hintKey,
  };
}

export function integrationRowActions(row: ManagedIntegrationStatus): {
  canDisconnect: boolean;
  canRepair: boolean;
  canConnect: boolean;
} {
  const canDisconnect = row.connected && !row.disconnected;
  const canRepair =
    row.repair_available &&
    !row.disconnected &&
    !row.connected &&
    (row.hooks_outdated || (row.source === "cc" && row.issue != null && row.issue !== "healthy"));
  const canConnect = row.agent_present && row.repair_available && !row.connected && !canRepair;
  return { canDisconnect, canRepair, canConnect };
}
