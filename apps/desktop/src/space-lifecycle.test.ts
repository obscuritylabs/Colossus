import { describe, expect, it } from "vitest";

import type { DesktopStatus } from "./types";
import { projectSpaceArchived, projectSpaceRestored } from "./space-lifecycle";

const STATUS: DesktopStatus = {
  releaseChannel: "development",
  connection: {
    state: "connected",
    message: "Ready",
    targetId: "space-one",
  },
  targets: [
    {
      targetId: "space-one",
      kind: "managed_local",
      label: "Managed Local",
      state: "ready",
      message: "Ready",
      selected: true,
      terminalAvailable: true,
      workspace: null,
      failureCode: null,
    },
  ],
  selectedTargetId: "space-one",
  spaces: [
    {
      spaceId: "space-one",
      targetId: "space-one",
      displayName: "One",
      displayPath: "~/one",
      archived: false,
      lastOpenedAtMs: 1,
      lastActivityAt: null,
      state: "ready",
      message: "Ready",
      selected: true,
      attentionCount: 0,
      providerConfigured: true,
    },
    {
      spaceId: "space-two",
      targetId: "space-two",
      displayName: "Two",
      displayPath: "~/two",
      archived: false,
      lastOpenedAtMs: 0,
      lastActivityAt: null,
      state: "sleeping",
      message: "Starts when selected.",
      selected: false,
      attentionCount: 0,
      providerConfigured: true,
    },
  ],
  selectedSpaceId: "space-one",
  managedState: "ready",
  workspace: null,
  provider: { configured: true, kind: "open_ai_codex", model: "fixture" },
  codexAuth: { state: "signed_in", message: "Ready" },
  managedModelConfiguration: { providers: [], models: [], roles: {} },
  accessProfile: "allow_all",
  executionBoundary: "full_access",
  approvalMode: "ask",
  terminalEnabled: false,
  additionalCaBundle: {
    configured: false,
    certificateCount: 0,
    fingerprintsSha256: [],
  },
  capabilities: {
    research: true,
    delegation: true,
    plugins: true,
    tui: true,
    shellTerminal: true,
    files: true,
    artifacts: true,
    planContinuation: true,
    updateAvailable: false,
    agentWorkflows: true,
    attachments: true,
  },
};

describe("fixture Space lifecycle projection", () => {
  it("archives the selected Space and routes to the most recent active Space", () => {
    const archived = projectSpaceArchived(STATUS, "space-one");

    expect(archived.selectedSpaceId).toBe("space-two");
    expect(archived.selectedTargetId).toBe("space-two");
    expect(archived.connection.targetId).toBe("space-two");
    expect(archived.workspace).toMatchObject({
      workspaceId: "space-two",
      displayName: "Two",
    });
    expect(archived.spaces[0]).toMatchObject({
      archived: true,
      selected: false,
      state: "archived",
    });
    expect(archived.spaces[1]).toMatchObject({
      archived: false,
      selected: true,
      state: "ready",
    });
    expect(archived.targets).toContainEqual(
      expect.objectContaining({
        targetId: "space-two",
        selected: true,
        state: "ready",
      }),
    );
  });

  it("restores an archived Space without silently selecting it", () => {
    const restored = projectSpaceRestored(
      projectSpaceArchived(STATUS, "space-one"),
      "space-one",
    );

    expect(restored.selectedSpaceId).toBe("space-two");
    expect(restored.spaces[0]).toMatchObject({
      archived: false,
      selected: false,
      state: "sleeping",
    });
  });
});
