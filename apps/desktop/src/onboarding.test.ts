import { describe, expect, it, vi } from "vitest";

import {
  buildManagedRuntimeRequest,
  managedOnboardingRequired,
  managedProviderDefaults,
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
  replaceCredential: false,
};

describe("Managed Local onboarding", () => {
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

  it("submits no provider credential or origin through renderer IPC", async () => {
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

  it("provides deterministic, non-secret defaults for supported providers", () => {
    expect(managedProviderDefaults("openai_responses")).toEqual({
      model: "gpt-5",
    });
    expect(managedProviderDefaults("openai_compatible")).toEqual({
      model: "deepseek/deepseek-v4-flash",
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
          message: "Checking the bundled runtime without contacting a model…",
        },
      ],
      [
        {
          state: "passed",
          message:
            "Offline runtime self-test passed. No provider was contacted.",
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
