import { describe, expect, it, vi } from "vitest";

import {
  buildManagedRuntimeRequest,
  managedOnboardingRequired,
  managedProviderDefaults,
  managedSetupLaunchFailure,
  requiresAdvancedModelSetup,
  runOfflineSelfTest,
  submitManagedRuntimeConfiguration,
} from "./onboarding";
import type { ManagedProviderDraft } from "./onboarding";
import type { DesktopStatus, WorkspaceSummary } from "./types";

const workspace: WorkspaceSummary = {
  workspaceId: "workspace-opaque-1",
  displayName: "Colossus",
  displayPath: "~/tools/Colossus",
};

const draft: ManagedProviderDraft = {
  providerKind: "openai_compatible",
  model: " deepseek/deepseek-v4-flash ",
  accessProfile: "development",
  executionBoundary: "workspace_isolated",
  replaceCredential: false,
};

describe("Managed Local onboarding", () => {
  it("completes setup only when the saved runtime is connected", () => {
    const connection = {
      targetId: "managed-local",
      message: "The runtime exited during startup.",
    };
    expect(
      managedSetupLaunchFailure({
        connection: { ...connection, state: "connected" },
      }),
    ).toBeNull();
    for (const state of [
      "restarting",
      "disconnected",
      "failed",
      "starting",
    ] as const) {
      expect(
        managedSetupLaunchFailure({ connection: { ...connection, state } }),
      ).toMatchObject({
        code: "managed_setup_not_connected",
        message: expect.stringContaining(connection.message),
        retryable: true,
        outcomeUnknown: false,
      });
    }
  });

  it("keeps custom routing, reasoning and timeouts in the full configuration editor", () => {
    const configuration = {
      providers: [
        { profile: "primary-provider", timeoutMs: null as number | null },
      ],
      models: [
        {
          profile: "primary",
          providerProfile: "primary-provider",
          reasoningEffort: null as "high" | null,
        },
      ],
      roles: { primary: "primary" },
    };
    expect(requiresAdvancedModelSetup(configuration)).toBe(false);
    expect(
      requiresAdvancedModelSetup({
        ...configuration,
        providers: [{ ...configuration.providers[0]!, timeoutMs: 900000 }],
      }),
    ).toBe(true);
    expect(
      requiresAdvancedModelSetup({
        ...configuration,
        models: [{ ...configuration.models[0]!, reasoningEffort: "high" }],
      }),
    ).toBe(true);
    expect(
      requiresAdvancedModelSetup({
        ...configuration,
        roles: { ...configuration.roles, research_worker: "primary" },
      }),
    ).toBe(true);
    expect(
      requiresAdvancedModelSetup({
        ...configuration,
        providers: [
          { ...configuration.providers[0]!, profile: "custom-provider" },
        ],
      }),
    ).toBe(true);
  });

  it("requires setup when Managed Local is selected without a provider", () => {
    const desktop = {
      selectedTargetId: "managed-local",
      managedState: "needs_provider",
      workspace,
      provider: { configured: false, kind: null, model: "" },
      targets: [
        {
          targetId: "managed-local",
          kind: "managed_local",
        },
      ],
    } as DesktopStatus;

    expect(managedOnboardingRequired(desktop)).toBe(true);
  });

  it("still requires setup after a failed premature connection attempt", () => {
    const desktop = {
      selectedTargetId: "managed-local",
      managedState: "failed",
      workspace,
      provider: { configured: false, kind: null, model: "" },
      targets: [
        {
          targetId: "managed-local",
          kind: "managed_local",
        },
      ],
    } as DesktopStatus;

    expect(managedOnboardingRequired(desktop)).toBe(true);
  });

  it("does not interrupt an explicitly selected external target", () => {
    const desktop = {
      selectedTargetId: "external-1",
      managedState: "needs_provider",
      workspace,
      provider: { configured: false, kind: null, model: "" },
      targets: [
        {
          targetId: "external-1",
          kind: "external_daemon",
        },
      ],
    } as DesktopStatus;

    expect(managedOnboardingRequired(desktop)).toBe(false);
  });

  it("builds an opaque workspace-scoped provider request without display paths", () => {
    const request = buildManagedRuntimeRequest(workspace, draft);

    expect(request).toEqual({
      workspaceId: "workspace-opaque-1",
      providerKind: "openai_compatible",
      model: "deepseek/deepseek-v4-flash",
      accessProfile: "development",
      executionBoundary: "workspace_isolated",
      replaceCredential: false,
    });
    expect(request).not.toHaveProperty("displayName");
    expect(request).not.toHaveProperty("displayPath");
  });

  it("refuses incomplete workspace or provider submissions", () => {
    expect(buildManagedRuntimeRequest(null, draft)).toBeNull();
    expect(
      buildManagedRuntimeRequest(workspace, { ...draft, model: "" }),
    ).toBeNull();
  });

  it("submits no provider credential value through renderer IPC", async () => {
    const configure = vi.fn().mockResolvedValue(true);

    await expect(
      submitManagedRuntimeConfiguration(workspace, draft, configure),
    ).resolves.toBe(true);
    const request = configure.mock.calls[0]?.[0];
    expect(request).not.toHaveProperty("apiKey");
    expect(request).not.toHaveProperty("baseUrl");
    expect(request).toHaveProperty("replaceCredential", false);
  });

  it("returns a rejected native configuration result", async () => {
    const configure = vi.fn().mockResolvedValue(false);

    await expect(
      submitManagedRuntimeConfiguration(workspace, draft, configure),
    ).resolves.toBe(false);
  });

  it("surfaces a native configuration failure", async () => {
    const configure = vi.fn().mockRejectedValue(new Error("native failure"));

    await expect(
      submitManagedRuntimeConfiguration(workspace, draft, configure),
    ).rejects.toThrow("native failure");
  });

  it("requires a discovered or explicitly entered model instead of a stale hardcoded ID", () => {
    expect(managedProviderDefaults("openai_responses")).toEqual({
      model: "",
    });
    expect(managedProviderDefaults("openai_compatible")).toEqual({
      model: "",
    });
  });

  it("passes custom endpoints, opaque credential references and reviewed model metadata", () => {
    const modelMetadata = {
      contextWindowTokens: 8192,
      maxOutputTokens: 4096,
      toolCalls: true,
      imageInputs: false,
      streaming: false,
    };
    expect(
      buildManagedRuntimeRequest(workspace, {
        ...draft,
        baseUrl: "http://localhost:1234/v1",
        credentialId: "credential-opaque",
        modelMetadata,
      }),
    ).toMatchObject({
      baseUrl: "http://localhost:1234/v1",
      credentialId: "credential-opaque",
      modelMetadata,
    });
  });

  it("reports offline self-test progress without changing provider state", async () => {
    const run = vi.fn().mockResolvedValue(undefined);
    const update = vi.fn();

    await runOfflineSelfTest(run, update);

    expect(run).toHaveBeenCalledOnce();
    expect(update.mock.calls).toEqual([
      [
        {
          state: "running",
          message: "Checking that Colossus can run on this computer…",
        },
      ],
      [
        {
          state: "passed",
          message:
            "Installation check passed. No model provider was contacted.",
        },
      ],
    ]);
    expect(update.mock.calls.flat()).not.toEqual(
      expect.arrayContaining([expect.objectContaining({ configured: true })]),
    );
  });

  it("surfaces a sanitized offline self-test failure", async () => {
    const update = vi.fn();

    await runOfflineSelfTest(
      () =>
        Promise.reject(new Error("Bundled runtime integrity check failed.")),
      update,
    );

    expect(update).toHaveBeenLastCalledWith({
      state: "failed",
      message: "Bundled runtime integrity check failed.",
    });
  });
});
