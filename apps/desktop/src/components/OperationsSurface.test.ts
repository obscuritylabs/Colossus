import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { DesktopStatus, RuntimeTarget } from "../types";
import { OperationsSurface } from "./OperationsSurface";

const managedTarget: RuntimeTarget = {
  targetId: "managed-local",
  kind: "managed_local",
  label: "Managed Local",
  state: "ready",
  message: "Managed runtime ready.",
  selected: false,
  terminalAvailable: true,
  workspace: {
    workspaceId: "workspace-1",
    displayName: "Colossus",
    displayPath: "~/tools/Colossus",
  },
  failureCode: null,
};

const externalTarget: RuntimeTarget = {
  targetId: "external-lab",
  kind: "external_daemon",
  label: "Lab fleet",
  state: "ready",
  message: "External daemon ready.",
  selected: true,
  terminalAvailable: false,
  workspace: null,
  failureCode: null,
};

function desktop(overrides: Partial<DesktopStatus> = {}): DesktopStatus {
  return {
    releaseChannel: "development",
    connection: {
      state: "connected",
      message: "Connected securely.",
      targetId: "external-lab",
    },
    targets: [managedTarget, externalTarget],
    selectedTargetId: "external-lab",
    managedState: "ready",
    workspace: managedTarget.workspace,
    provider: {
      configured: true,
      kind: "openai_compatible",
      model: "deepseek/deepseek-v4-flash",
    },
    managedModelConfiguration: { providers: [], models: [], roles: {} },
    accessProfile: "development",
    terminalEnabled: false,
    additionalCaBundle: {
      configured: false,
      certificateCount: 0,
      fingerprintsSha256: [],
    },
    capabilities: {
      delegation: false,
      skills: false,
      tui: true,
      files: false,
      artifacts: true,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    },
    ...overrides,
  };
}

function renderSurface(
  surface: "fleet" | "settings",
  status = desktop(),
): string {
  return renderToStaticMarkup(
    createElement(OperationsSurface, {
      surface,
      connection: status.connection,
      desktop: status,
      connecting: false,
      updateChecking: false,
      updateMessage: "",
      runs: [],
      artifacts: [],
      activity: [],
      demoParticipants: null,
      onConnect: vi.fn(),
      onOpenRun: vi.fn(),
      onSelectTarget: vi.fn(),
      onAddExternalTarget: vi.fn(),
      onRemoveExternalTarget: vi.fn(),
      onChooseWorkspace: vi.fn(),
      onConfigureManaged: vi.fn(),
      onRestartManaged: vi.fn(),
      onSetTerminalEnabled: vi.fn(),
      onOpenTerminal: vi.fn(),
      onExportDiagnostics: vi.fn(),
      onCheckForUpdates: vi.fn(),
      onInstallUpdate: vi.fn(),
      onImportCaBundle: vi.fn(),
      onRemoveCaBundle: vi.fn(),
    }),
  );
}

function openingButtonTag(markup: string, label: string): string {
  const labelIndex = markup.indexOf(label);
  expect(labelIndex).toBeGreaterThan(-1);
  const buttonIndex = markup.lastIndexOf("<button", labelIndex);
  expect(buttonIndex).toBeGreaterThan(-1);
  return markup.slice(buttonIndex, markup.indexOf(">", buttonIndex) + 1);
}

function checkboxTag(markup: string): string {
  return markup.match(/<input[^>]*type="checkbox"[^>]*>/)?.[0] ?? "";
}

describe("OperationsSurface runtime targets", () => {
  it("shows and marks the selected external target in fleet view", () => {
    const markup = renderSurface("fleet");

    expect(markup).toContain("2 available nodes");
    expect(markup).toContain("Managed Local");
    expect(markup).toContain("Lab fleet");
    expect(openingButtonTag(markup, "Lab fleet")).toContain(
      'aria-pressed="true"',
    );
    expect(openingButtonTag(markup, "Managed Local")).toContain(
      'aria-pressed="false"',
    );
  });

  it("renders only orchestration capabilities advertised by the selected target", () => {
    const markup = renderSurface(
      "fleet",
      desktop({
        capabilities: {
          delegation: true,
          skills: false,
          tui: false,
          files: false,
          artifacts: false,
          updateAvailable: false,
          agentWorkflows: false,
          attachments: false,
        },
      }),
    );

    expect(markup).toContain("Delegated agents");
    expect(markup).not.toContain("Durable workflows");
    expect(markup).not.toContain("Declarative skills");
  });

  it("labels the actually selected target instead of assuming Managed Local", () => {
    const markup = renderSurface("settings");

    expect(markup).toContain("<h3>Lab fleet</h3>");
    expect(markup).not.toContain("<h3>Connected</h3>");
  });

  it("lists external targets without exposing native connection material", () => {
    const markup = renderSurface("settings");

    expect(markup).toContain("Advanced daemon connections");
    expect(markup).toContain("Lab fleet");
    expect(markup).toContain("Remove Lab fleet");
    expect(markup).toContain("Not configured");
    expect(markup).not.toContain("certificateSha256");
    expect(markup).not.toContain("credentialService");
    expect(markup).not.toContain("publicApiDir");
  });

  it("shows the renderer-safe effective Managed Local configuration", () => {
    const markup = renderSurface(
      "settings",
      desktop({
        managedModelConfiguration: {
          providers: [
            {
              profile: "internal",
              providerKind: "openai_compatible",
              baseUrl: "https://models.example.test/v1",
              hasCredential: true,
              timeoutMs: 45_000,
            },
          ],
          models: [
            {
              profile: "primary",
              providerProfile: "internal",
              model: "example/model",
              contextWindowTokens: 64_000,
              maxOutputTokens: 8_000,
              capabilities: { toolCalls: true, streaming: true },
            },
          ],
          roles: { primary: "primary" },
        },
      }),
    );

    expect(markup).toContain("Effective configuration");
    expect(markup).toContain("Effective Managed Local configuration");
    expect(markup).toContain(
      '<details class="effective-configuration-disclosure">',
    );
    expect(markup).toContain("Show configuration");
    expect(markup).toContain("Hide configuration");
    expect(markup).not.toContain(
      '<details class="effective-configuration-disclosure" open="">',
    );
    expect(markup).toContain("https://models.example.test/v1");
    expect(markup).toContain("stored_in_native_keyring");
    expect(markup).toContain("example/model");
    expect(markup).toContain("contextWindowTokens");
    expect(markup).toContain("Active");
    expect(markup).not.toContain("credentialId");
    expect(markup).not.toContain("credentialService");
    expect(markup).not.toContain("certificateSha256");
    expect(markup).not.toContain("caBundlePath");
    expect(markup).not.toContain("publicApiDir");
  });

  it("shows only sanitized CA bundle state and certificate fingerprints", () => {
    const fingerprint = "ab".repeat(32);
    const markup = renderSurface(
      "settings",
      desktop({
        additionalCaBundle: {
          configured: true,
          certificateCount: 1,
          fingerprintsSha256: [fingerprint],
        },
      }),
    );

    expect(markup).toContain("1 additional certificate");
    expect(markup).toContain(fingerprint);
    expect(markup).toContain("Remove bundle");
    expect(markup).not.toContain("caBundlePath");
    expect(markup).not.toContain("/private/");
  });

  it("shows user-triggered channel-scoped update controls", () => {
    const current = renderSurface("settings");
    expect(current).toContain("Desktop updates");
    expect(current).toContain("Check for updates");
    expect(current).not.toContain("Install update");

    const available = renderSurface(
      "settings",
      desktop({
        releaseChannel: "developer_preview",
        capabilities: {
          ...desktop().capabilities,
          updateAvailable: true,
        },
      }),
    );
    expect(available).toContain("developer preview channel");
    expect(available).toContain("Install update");
  });

  it("keeps terminal controls disabled for an external target after prior consent", () => {
    const markup = renderSurface(
      "settings",
      desktop({ terminalEnabled: true }),
    );

    expect(checkboxTag(markup)).toContain("checked");
    expect(checkboxTag(markup)).toContain("disabled");
    expect(markup).toContain(
      "The local TUI is unavailable for an external target.",
    );
    expect(markup).not.toContain("Open shell");
    expect(openingButtonTag(markup, "Open Colossus TUI")).toContain("disabled");
  });

  it("allows local TUI launch only for a terminal-capable selected target", () => {
    const enabled = desktop({
      connection: {
        state: "connected",
        message: "Connected securely.",
        targetId: "managed-local",
      },
      selectedTargetId: "managed-local",
      targets: [
        { ...managedTarget, selected: true },
        { ...externalTarget, selected: false },
      ],
      terminalEnabled: true,
    });
    const markup = renderSurface("settings", enabled);

    expect(markup).toContain('type="checkbox" checked=""');
    expect(markup).not.toContain("Open shell");
    expect(openingButtonTag(markup, "Open Colossus TUI")).not.toContain(
      "disabled",
    );

    const restarting = desktop({
      terminalEnabled: true,
      connection: {
        state: "restarting",
        message: "Restarting Managed Local.",
        targetId: "managed-local",
      },
      selectedTargetId: "managed-local",
      targets: [
        {
          ...managedTarget,
          selected: true,
          state: "restarting",
          terminalAvailable: false,
        },
        { ...externalTarget, selected: false },
      ],
    });
    const restartingMarkup = renderSurface("settings", restarting);
    expect(openingButtonTag(restartingMarkup, "Open Colossus TUI")).toContain(
      "disabled",
    );
  });
});
