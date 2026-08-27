import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { DesktopStatus, ManagedModelConfiguration } from "../types";
import {
  ModelConfigurationEditor,
  changeProviderProtocol,
  submitModelConfiguration,
} from "./ModelConfigurationEditor";
import type { EditableProvider } from "./ModelConfigurationEditor";

const desktop: DesktopStatus = {
  releaseChannel: "development",
  connection: {
    state: "connected",
    message: "Connected.",
    targetId: "managed-local",
  },
  targets: [],
  selectedTargetId: "managed-local",
  spaces: [],
  selectedSpaceId: null,
  managedState: "ready",
  workspace: {
    workspaceId: "workspace-1",
    displayName: "Colossus",
    displayPath: "~/Colossus",
  },
  provider: {
    configured: true,
    kind: "openai_compatible",
    model: "example-model",
  },
  codexAuth: {
    state: "signed_out",
    message: "Sign in with ChatGPT to use Codex.",
  },
  managedModelConfiguration: {
    providers: [
      {
        profile: "local-provider",
        providerKind: "openai_compatible",
        baseUrl: "http://127.0.0.1:11434/v1",
        hasCredential: false,
        timeoutMs: null,
        effectiveTimeoutMs: 900_000,
      },
    ],
    models: [
      {
        profile: "primary",
        providerProfile: "local-provider",
        model: "example-model",
        contextWindowTokens: 32_768,
        maxOutputTokens: 4_096,
        reasoningEffort: null,
        capabilities: {
          toolCalls: false,
          streaming: false,
          imageInputs: false,
        },
      },
    ],
    roles: { primary: "primary" },
  },
  accessProfile: "minimal",
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
    skills: false,
    tui: false,
    shellTerminal: false,
    files: false,
    artifacts: true,
    planContinuation: false,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
};

describe("ModelConfigurationEditor", () => {
  it("renders safe provider/model metadata and every supported role", () => {
    const markup = renderToStaticMarkup(
      createElement(ModelConfigurationEditor, {
        desktop,
        busy: false,
        onApply: vi.fn(),
        onCodexLogin: vi.fn(),
        onCodexLogout: vi.fn(),
        onBack: vi.fn(),
      }),
    );

    expect(markup).toContain("http://127.0.0.1:11434/v1");
    expect(markup).toContain("example-model");
    expect(markup).toContain("No credential");
    expect(markup).toContain("Automatic · 15 minutes");
    expect(markup).toContain('role="combobox"');
    expect(markup).toContain("OpenAI-compatible");
    expect(markup).toContain("Reasoning effort");
    expect(markup).toContain("Provider default");
    expect(markup).toContain("Unsafe: Full access.");
    expect(markup).toContain("Approval mode is configured separately.");
    expect(markup).not.toContain("Custom timeout (ms)");
    for (const role of [
      "primary",
      "risk evaluator",
      "context summarizer",
      "subagent default",
      "research planner",
      "research worker",
      "research synthesizer",
    ]) {
      expect(markup).toContain(role);
    }
    expect(markup).not.toContain("credentialId");
    expect(markup).not.toContain("apiKey");
  });

  it("clears every bound model and refuses submission after a protocol change", async () => {
    const providers: EditableProvider[] = [
      {
        profile: "primary-provider",
        providerKind: "openai_compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        timeoutMs: null,
        effectiveTimeoutMs: 720_000,
        credentialAction: "replace",
      },
      {
        profile: "other-provider",
        providerKind: "openai_responses",
        baseUrl: "https://api.openai.com/v1",
        timeoutMs: null,
        effectiveTimeoutMs: 720_000,
        credentialAction: "reuse",
      },
    ];
    const models: ManagedModelConfiguration[] = [
      {
        profile: "primary",
        providerProfile: "primary-provider",
        model: "deepseek/deepseek-v4-flash",
        contextWindowTokens: 128_000,
        maxOutputTokens: 16_000,
        reasoningEffort: null,
        capabilities: { toolCalls: true, streaming: true, imageInputs: false },
      },
      {
        profile: "worker",
        providerProfile: "primary-provider",
        model: "another-stale-model",
        contextWindowTokens: 128_000,
        maxOutputTokens: 16_000,
        reasoningEffort: null,
        capabilities: { toolCalls: true, streaming: true, imageInputs: false },
      },
      {
        profile: "other",
        providerProfile: "other-provider",
        model: "gpt-5",
        contextWindowTokens: 128_000,
        maxOutputTokens: 16_000,
        reasoningEffort: null,
        capabilities: { toolCalls: true, streaming: true, imageInputs: false },
      },
    ];

    const changed = changeProviderProtocol(
      providers,
      models,
      0,
      "open_ai_codex",
    );

    expect(changed.providers[0]).toMatchObject({
      providerKind: "open_ai_codex",
      baseUrl: "https://chatgpt.com/backend-api/codex",
      credentialAction: "none",
    });
    expect(changed.models.map((model) => model.model)).toEqual([
      "",
      "",
      "gpt-5",
    ]);

    const apply = vi.fn().mockResolvedValue(true);
    await expect(
      submitModelConfiguration(
        "workspace-1",
        changed.providers,
        changed.models,
        { primary: "primary" },
        "minimal",
        "offline_isolated",
        apply,
      ),
    ).resolves.toBe(false);
    expect(apply).not.toHaveBeenCalled();
  });

  it("disables Apply when a model identifier contains only whitespace", () => {
    const markup = renderToStaticMarkup(
      createElement(ModelConfigurationEditor, {
        desktop: {
          ...desktop,
          managedModelConfiguration: {
            ...desktop.managedModelConfiguration,
            models: desktop.managedModelConfiguration.models.map((model) => ({
              ...model,
              model: " \t ",
            })),
          },
        },
        busy: false,
        onApply: vi.fn(),
        onCodexLogin: vi.fn(),
        onCodexLogout: vi.fn(),
        onBack: vi.fn(),
      }),
    );

    expect(markup).toContain(
      '<button class="button primary onboarding-launch" disabled="">Apply model configuration</button>',
    );
  });
});
