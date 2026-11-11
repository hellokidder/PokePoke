import { describe, expect, test } from "vitest";
import {
  connectionStatus,
  integrationRowActions,
  presenceStatus,
  type ManagedIntegrationStatus,
} from "./integrationsLogic";

function row(
  overrides: Partial<ManagedIntegrationStatus> & Pick<ManagedIntegrationStatus, "source">,
): ManagedIntegrationStatus {
  return {
    agent_present: true,
    connected: false,
    hooks_configured: false,
    hooks_outdated: false,
    installed: true,
    issue: null,
    repair_available: true,
    disconnected: false,
    ...overrides,
  };
}

describe("integrationsLogic", () => {
  test("presence reflects agent_present", () => {
    expect(presenceStatus(row({ source: "cc", agent_present: true })).labelKey).toBe(
      "settings.integrations.presence_yes",
    );
    expect(presenceStatus(row({ source: "cc", agent_present: false })).labelKey).toBe(
      "settings.integrations.presence_no",
    );
  });

  test("connection shows manual off when disconnected", () => {
    const status = connectionStatus(
      row({ source: "cc", disconnected: true, connected: false, issue: "hooks_missing" }),
    );
    expect(status.labelKey).toBe("settings.integrations.conn_manual_off");
    expect(status.tone).toBe("muted");
  });

  test("disconnected cc does not surface repair in actions", () => {
    const actions = integrationRowActions(
      row({
        source: "cc",
        disconnected: true,
        connected: false,
        issue: "hooks_missing",
      }),
    );
    expect(actions.canRepair).toBe(false);
    expect(actions.canConnect).toBe(true);
    expect(actions.canDisconnect).toBe(false);
  });

  test("connected row offers disconnect only", () => {
    const actions = integrationRowActions(row({ source: "cursor", connected: true }));
    expect(actions.canDisconnect).toBe(true);
    expect(actions.canConnect).toBe(false);
    expect(actions.canRepair).toBe(false);
  });

  test("hooks_outdated offers repair not connect", () => {
    const actions = integrationRowActions(
      row({ source: "codex", hooks_outdated: true, hooks_configured: true }),
    );
    expect(actions.canRepair).toBe(true);
    expect(actions.canConnect).toBe(false);
  });

  test("codex feature off adds hint key", () => {
    const status = connectionStatus(
      row({ source: "codex", feature_enabled: false, agent_present: true }),
    );
    expect(status.hintKey).toBe("settings.integrations.codex_feature_off");
  });
});
