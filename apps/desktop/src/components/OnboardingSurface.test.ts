import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { DesktopStatus, WorkspaceSummary } from "../types";
import { OnboardingSurface } from "./OnboardingSurface";

const workspace: WorkspaceSummary = {
  workspaceId: "workspace-opaque-1",
  displayName: "Colossus",
  displayPath: "~/tools/Colossus",
};

function desktop(selectedWorkspace: WorkspaceSummary | null): DesktopStatus {
  return {
    releaseChannel: "development",
    connection: {
      state: "not_configured",
      message: "Managed Local needs setup.",
      targetId: null,
    },
    targets: [],
    selectedTargetId: null,
    managedState:
      selectedWorkspace === null ? "needs_workspace" : "needs_provider",
    workspace: selectedWorkspace,
    provider: { configured: false, kind: null, model: "" },
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
      tui: false,
      files: false,
      artifacts: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    },
  };
}

function renderOnboarding(
  selectedWorkspace: WorkspaceSummary | null,
  overrides: Partial<DesktopStatus> = {},
  dismissible = false,
): string {
  return renderToStaticMarkup(
    createElement(OnboardingSurface, {
      desktop: { ...desktop(selectedWorkspace), ...overrides },
      busy: false,
      error: "",
      onChooseWorkspace: vi.fn(),
      onConfigure: vi.fn(),
      onApplyConfiguration: vi.fn(),
      onRunSelfTest: vi.fn(),
      onUseExternal: vi.fn(),
      dismissible,
      onCancel: vi.fn(),
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

describe("OnboardingSurface", () => {
  it("starts with folder selection and keeps offline verification disabled", () => {
    const markup = renderOnboarding(null);

    expect(markup).toContain("Choose a folder");
    expect(markup).not.toContain("provider-setup-form");
    expect(openingButtonTag(markup, "Run offline self-test")).toContain(
      "disabled",
    );
  });

  it("shows provider setup and enables offline verification after folder selection", () => {
    const markup = renderOnboarding(workspace);

    expect(markup).toContain('class="provider-setup-form"');
    expect(markup).toContain("~/tools/Colossus");
    expect(markup).not.toContain('type="password"');
    expect(markup).not.toContain("API base URL");
    expect(markup).toContain("native secure prompt");
    expect(openingButtonTag(markup, "Run offline self-test")).not.toContain(
      "disabled",
    );
    expect(markup).toContain(
      "It does not configure a provider or enable model runs.",
    );
    expect(markup).not.toContain(">Cancel</button>");
  });

  it("prefills and can dismiss the settings provider editor", () => {
    const markup = renderOnboarding(
      workspace,
      {
        provider: {
          configured: true,
          kind: "openai_responses",
          model: "configured-model",
        },
        accessProfile: "minimal",
      },
      true,
    );

    expect(markup).toContain("Configure Managed Local");
    expect(markup).toContain(">Cancel</button>");
    expect(markup).toContain('value="configured-model"');
    expect(markup).toContain(
      '<option value="openai_responses" selected="">OpenAI Responses</option>',
    );
    expect(markup).toContain(
      '<option value="minimal" selected="">Minimal — no workspace tools</option>',
    );
    expect(markup).toContain("Replace the stored API key");
    expect(markup).toContain(
      "The existing provider key remains in the OS keychain.",
    );
    expect(markup).toContain('type="checkbox"');
    expect(markup).not.toContain('type="checkbox" checked=""');
  });
});
