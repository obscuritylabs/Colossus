import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { DesktopStatus, SpaceSummary } from "../types";
import {
  buildManagedSettingsFixture,
  ManagedSettingsPane,
} from "./ManagedSettingsPane";

const space: SpaceSummary = {
  spaceId: "space-colossus",
  targetId: "managed-local",
  displayName: "Colossus",
  displayPath: "D:\\tools\\Colossus",
  archived: false,
  lastOpenedAtMs: 1_721_490_000_000,
  state: "ready",
  message: "Managed runtime ready.",
  selected: true,
  attentionCount: 0,
  lastActivityAt: null,
  providerConfigured: true,
};

function desktop(): DesktopStatus {
  return {
    releaseChannel: "development",
    connection: {
      state: "connected",
      message: "Connected securely.",
      targetId: "managed-local",
    },
    targets: [
      {
        targetId: "managed-local",
        kind: "managed_local",
        label: "Managed Local",
        state: "ready",
        message: "Managed runtime ready.",
        selected: true,
        terminalAvailable: true,
        workspace: {
          workspaceId: "workspace-colossus",
          displayName: "Colossus",
          displayPath: "D:\\tools\\Colossus",
        },
        failureCode: null,
      },
    ],
    selectedTargetId: "managed-local",
    spaces: [space],
    selectedSpaceId: space.spaceId,
    managedState: "ready",
    workspace: {
      workspaceId: "workspace-colossus",
      displayName: "Colossus",
      displayPath: "D:\\tools\\Colossus",
    },
    provider: {
      configured: true,
      kind: "openai_compatible",
      model: "colossus-primary",
    },
    codexAuth: {
      state: "signed_out",
      message: "Sign in with ChatGPT to use Codex.",
    },
    managedModelConfiguration: {
      providers: [
        {
          profile: "openapi",
          providerKind: "openai_compatible",
          baseUrl: "https://llm.example.test/v1",
          hasCredential: true,
          timeoutMs: 30_000,
          effectiveTimeoutMs: 30_000,
        },
      ],
      models: [
        {
          profile: "primary",
          providerProfile: "openapi",
          model: "colossus-primary",
          contextWindowTokens: 128_000,
          maxOutputTokens: 16_384,
          capabilities: { toolCalls: true, streaming: true },
          reasoningEffort: null,
        },
      ],
      roles: { primary: "primary" },
    },
    accessProfile: "development",
    executionBoundary: "workspace_isolated",
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
      skills: true,
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
}

function renderPane(): string {
  return renderToStaticMarkup(
    createElement(ManagedSettingsPane, {
      desktop: desktop(),
      connecting: false,
      updateChecking: false,
      updateMessage: "",
      onChooseWorkspace: vi.fn(),
      onConfigureManaged: vi.fn(),
      onRestartManaged: vi.fn(),
      onAddExternalTarget: vi.fn(),
      onRemoveExternalTarget: vi.fn(),
      onSetTerminalEnabled: vi.fn(),
      onOpenTerminal: vi.fn(),
      onCheckForUpdates: vi.fn(),
      onInstallUpdate: vi.fn(),
      onImportCaBundle: vi.fn(),
      onRemoveCaBundle: vi.fn(),
    }),
  );
}

describe("ManagedSettingsPane", () => {
  it("builds a revisioned, renderer-safe snapshot from Desktop status", () => {
    const snapshot = buildManagedSettingsFixture(desktop());

    expect(snapshot.globalConfiguration.revision).toBe(4);
    expect(snapshot.globalConfiguration.providers[0]?.label).toBe("openapi");
    expect(snapshot.globalConfiguration.models[0]?.label).toBe("primary");
    expect(snapshot.spaces[0]?.configuration.acceptedGlobalRevision).toBe(4);
    expect(snapshot.spaces[0]?.effectiveYaml).toContain(
      "workspaceIdentity: <desktop-managed>",
    );
    expect(snapshot.lockedInvariants.map((entry) => entry.id)).toContain(
      "runtime.bootstrapAuthentication",
    );

    const serialized = JSON.stringify(snapshot);
    expect(serialized).not.toContain("apiKey");
    expect(serialized).not.toContain("secretValue");
    expect(serialized).not.toContain("accessToken");
  });

  it("renders scope, provenance, lifecycle, and dirty-state controls", () => {
    const markup = renderPane();

    expect(markup).toContain('aria-label="Configuration scope"');
    expect(markup).toContain("Global");
    expect(markup).toContain("Space");
    expect(markup).toContain("Runtime defaults");
    expect(markup).toContain("built in");
    expect(markup).toContain("Authority summary");
    expect(markup).toContain("No local changes");
    expect(markup).toContain('disabled=""');
  });

  it("keeps all renderer markup free of secret inputs and values", () => {
    const markup = renderPane();

    expect(markup).not.toContain('type="password"');
    expect(markup).not.toContain("apiKey");
    expect(markup).not.toContain("clientSecret");
    expect(markup).not.toContain("credentialValue");
  });
});
