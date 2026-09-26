import { describe, expect, it } from "vitest";
import {
  composerModelIdentity,
  type ComposerModelContext,
} from "./composer-model";

const context: ComposerModelContext = {
  targetKind: "managed_local",
  configuration: {
    providers: [
      {
        profile: "account",
        providerKind: "open_ai_codex",
        baseUrl: "https://chatgpt.com/backend-api/codex",
        hasCredential: false,
        timeoutMs: null,
        effectiveTimeoutMs: 300000,
      },
      {
        profile: "router",
        providerKind: "openai_compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        hasCredential: true,
        timeoutMs: null,
        effectiveTimeoutMs: 300000,
      },
    ],
    models: [
      {
        profile: "main",
        providerProfile: "account",
        model: "gpt-6-astra",
        reasoningEffort: "high",
        contextWindowTokens: 128000,
        maxOutputTokens: 16000,
        capabilities: { toolCalls: true, imageInputs: true, streaming: true },
      },
      {
        profile: "review",
        providerProfile: "router",
        model: "vendor/review-model",
        reasoningEffort: null,
        contextWindowTokens: 128000,
        maxOutputTokens: 16000,
        capabilities: { toolCalls: true, imageInputs: false, streaming: true },
      },
    ],
    roles: { primary: "main", reviewer: "review" },
  },
};

describe("composer model identity", () => {
  it("shows the next message's model, provider and reasoning instead of a role name", () => {
    expect(composerModelIdentity(context, "primary")).toMatchObject({
      model: "gpt-6-astra",
      providerLabel: "Codex subscription",
      reasoningEffort: "high",
    });
    expect(composerModelIdentity(context, " reviewer ")).toMatchObject({
      model: "vendor/review-model",
      providerLabel: "OpenRouter",
      reasoningEffort: null,
    });
  });

  it("reflects the registry's primary fallback for an unconfigured specialized role", () => {
    expect(composerModelIdentity(context, "builder")).toEqual(
      composerModelIdentity(context, "primary"),
    );
  });

  it("does not substitute primary for a broken explicit model mapping", () => {
    const broken = {
      ...context,
      configuration: {
        ...context.configuration,
        roles: { ...context.configuration.roles, reviewer: "removed" },
      },
    };
    expect(composerModelIdentity(broken, "reviewer").model).toBe(
      "Model not configured",
    );
    expect(composerModelIdentity(context, " ").model).toBe(
      "Model not configured",
    );
  });

  it("never displays the local model for a server-managed or unselected target", () => {
    expect(
      composerModelIdentity(
        { ...context, targetKind: "external_daemon" },
        "primary",
      ),
    ).toMatchObject({
      model: "Server-managed model",
      provider: null,
      reasoningEffort: null,
    });
    expect(
      composerModelIdentity({ ...context, targetKind: null }, "primary").model,
    ).toBe("No model selected");
  });

  it("shows a custom endpoint host without disclosing URL credentials or query values", () => {
    const custom = {
      ...context,
      configuration: {
        ...context.configuration,
        providers: [
          {
            ...context.configuration.providers[1]!,
            baseUrl: "https://user:secret@gateway.example.test/v1?key=private",
          },
        ],
      },
    };
    expect(composerModelIdentity(custom, "reviewer").providerLabel).toBe(
      "gateway.example.test",
    );
  });

  it("makes missing providers explicit instead of guessing from the model ID", () => {
    const missing = {
      ...context,
      configuration: { ...context.configuration, providers: [] },
    };
    expect(composerModelIdentity(missing, "primary")).toMatchObject({
      model: "gpt-6-astra",
      providerLabel: "Provider unavailable",
      provider: null,
    });
  });
});
