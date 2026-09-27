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
    spaces: [],
    selectedSpaceId: null,
    managedState:
      selectedWorkspace === null ? "needs_workspace" : "needs_provider",
    workspace: selectedWorkspace,
    provider: { configured: false, kind: null, model: "" },
    codexAuth: {
      state: "signed_out",
      message: "Sign in with ChatGPT to use Codex.",
    },
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
      delegation: false,
      plugins: false,
      tui: false,
      shellTerminal: false,
      files: false,
      artifacts: false,
      planContinuation: false,
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
  error = "",
): string {
  return renderToStaticMarkup(
    createElement(OnboardingSurface, {
      desktop: { ...desktop(selectedWorkspace), ...overrides },
      busy: false,
      error,
      onChooseWorkspace: vi.fn(),
      onConfigure: vi.fn(),
      onApplyConfiguration: vi.fn(),
      onRunSelfTest: vi.fn(),
      onCodexLogin: vi.fn(),
      onCodexLogout: vi.fn(),
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
  it("starts with folder selection and allows offline verification", () => {
    const markup = renderOnboarding(null);

    expect(markup).toContain("Choose your project folder");
    expect(markup).toContain("Choose folder");
    expect(markup).not.toContain("provider-setup-form");
    expect(openingButtonTag(markup, "Run check")).not.toContain("disabled");
  });

  it("shows workspace selection failures before provider setup", () => {
    const markup = renderOnboarding(
      null,
      {},
      false,
      "The workspace selection is no longer valid.",
    );

    expect(markup).toContain('class="page-error"');
    expect(markup).toContain('role="alert"');
    expect(markup).toContain("The workspace selection is no longer valid.");
  });

  it("shows provider setup and enables offline verification after folder selection", () => {
    const markup = renderOnboarding(workspace);

    expect(markup).toContain('class="provider-setup-form"');
    expect(markup).toContain("~/tools/Colossus");
    expect(markup).not.toContain('type="password"');
    expect(markup).toContain("API base URL");
    expect(markup).toContain("Load models");
    expect(markup).toContain("Model limits and capabilities");
    expect(markup).toContain("separate secure window");
    expect(markup).toContain(
      '<span class="app-select-value">Allow all — all built-in tools</span>',
    );
    expect(markup).toContain("Full access is unsafe.");
    expect(markup).toContain("Approval settings still apply.");
    expect(openingButtonTag(markup, "Run check")).not.toContain("disabled");
    expect(markup).toContain(
      "Runs an offline check without setting up a provider or running a model.",
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
        managedModelConfiguration: {
          providers: [
            {
              profile: "primary-provider",
              providerKind: "openai_responses",
              baseUrl: "https://api.example.test/v1",
              hasCredential: true,
              timeoutMs: null,
              effectiveTimeoutMs: 300000,
            },
          ],
          models: [
            {
              profile: "primary",
              providerProfile: "primary-provider",
              model: "configured-model",
              contextWindowTokens: 32768,
              maxOutputTokens: 4096,
              reasoningEffort: null,
              capabilities: {
                toolCalls: false,
                imageInputs: false,
                streaming: false,
              },
            },
          ],
          roles: { primary: "primary" },
        },
        accessProfile: "minimal",
        executionBoundary: "offline_isolated",
      },
      true,
    );

    expect(markup).toContain("Edit workspace setup");
    expect(markup).toContain(">Cancel</button>");
    expect(markup).toContain('value="configured-model"');
    expect(markup).toContain(
      '<span class="app-select-value">OpenAI Responses</span>',
    );
    expect(markup).toContain(
      '<span class="app-select-value">Minimal — no workspace tools</span>',
    );
    expect(markup).toContain(
      '<span class="app-select-value">Offline isolated</span>',
    );
    expect(markup).not.toContain("Full access is unsafe.");
    expect(markup).toContain("Use a different API key");
    expect(markup).toContain("Your saved API key will be reused.");
    expect(markup).toContain('type="checkbox"');
    expect(markup).not.toContain('type="checkbox" checked=""');
  });

  it("uses native ChatGPT auth for the Codex subscription provider", () => {
    const markup = renderOnboarding(workspace, {
      provider: {
        configured: true,
        kind: "open_ai_codex",
        model: "gpt-5-codex",
      },
    });

    expect(markup).toContain("ChatGPT subscription (Codex)");
    expect(markup).toContain("Sign in with ChatGPT");
    expect(markup).toContain(
      "Connect through Codex using your ChatGPT account.",
    );
    expect(markup).not.toContain("Use a different API key");
    expect(openingButtonTag(markup, "Save and start")).toContain("disabled");
  });

  it("opens existing multiple providers in the full editor and shows apply errors", () => {
    const markup = renderOnboarding(
      workspace,
      {
        provider: {
          configured: true,
          kind: "openai_responses",
          model: "primary-model",
        },
        managedModelConfiguration: {
          providers: [
            {
              profile: "secondary-provider",
              providerKind: "openai_compatible",
              baseUrl: "https://secondary.example.test/v1",
              hasCredential: false,
              timeoutMs: null,
              effectiveTimeoutMs: 300000,
            },
            {
              profile: "primary-provider",
              providerKind: "openai_responses",
              baseUrl: "https://primary.example.test/v1",
              hasCredential: true,
              timeoutMs: null,
              effectiveTimeoutMs: 300000,
            },
          ],
          models: [
            {
              profile: "secondary",
              providerProfile: "secondary-provider",
              model: "secondary-model",
              contextWindowTokens: 32000,
              maxOutputTokens: 2000,
              reasoningEffort: null,
              capabilities: {
                toolCalls: false,
                streaming: false,
                imageInputs: false,
              },
            },
            {
              profile: "primary",
              providerProfile: "primary-provider",
              model: "primary-model",
              contextWindowTokens: 64000,
              maxOutputTokens: 8000,
              reasoningEffort: null,
              capabilities: {
                toolCalls: true,
                streaming: true,
                imageInputs: false,
              },
            },
          ],
          roles: { primary: "primary" },
        },
      },
      false,
      "The model configuration could not be applied.",
    );
    expect(markup).toContain('value="https://primary.example.test/v1"');
    expect(markup).toContain('value="64000"');
    expect(markup).toContain("https://secondary.example.test/v1");
    expect(markup).toContain("Save and start");
    expect(markup).not.toContain("Choose another folder");
    expect(markup).not.toContain("Back to basic setup");
    expect(markup).toContain("The model configuration could not be applied.");
  });
});
