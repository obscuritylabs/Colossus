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
    accessProfile: "development",
    terminalEnabled: false,
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
    expect(markup).not.toContain("certificateSha256");
    expect(markup).not.toContain("credentialService");
    expect(markup).not.toContain("publicApiDir");
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
