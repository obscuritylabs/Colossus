import { describe, expect, it } from "vitest";
import {
  importedWorkspaceConfiguration,
  safeSetupIcon,
  setupCredentialStatus,
  setupProviderReady,
} from "./setupPackages";
import type { SetupProvider } from "./setupPackages";

const provider: SetupProvider = {
  profile: "company",
  displayName: "Company",
  descriptionMarkdown: "",
  kind: "openai_compatible",
  baseUrl: "https://ai.example.com/v1",
  timeoutMs: null,
  credentialRequired: true,
  credentialId: null,
  icon: null,
  darkIcon: null,
  models: [],
};
describe("imported provider setup", () => {
  it("keeps credentials optional for import and required before provider use", () => {
    expect(setupCredentialStatus(provider, false)).toBe("Needs API key");
    expect(setupProviderReady(provider, false)).toBe(false);
    expect(
      setupProviderReady({ ...provider, credentialId: "saved" }, false),
    ).toBe(true);
    expect(
      setupCredentialStatus({ ...provider, credentialId: "saved" }, false),
    ).toContain("not checked");
  });
  it("distinguishes account authentication from endpoints that need no key", () => {
    expect(
      setupProviderReady(
        { ...provider, kind: "open_ai_codex", credentialRequired: false },
        false,
      ),
    ).toBe(false);
    expect(
      setupProviderReady({ ...provider, kind: "open_ai_codex" }, true),
    ).toBe(true);
    expect(
      setupCredentialStatus({ ...provider, credentialRequired: false }, false),
    ).toBe("No key required");
  });
  it("renders only bounded native PNG data and never remote images or SVG", () => {
    for (const value of [
      "https://example.com/logo.png",
      "file:///icon.png",
      "data:image/svg+xml,<svg/>",
      "data:image/png;base64,%%%",
      "data:image/png;base64," + "A".repeat(90001),
    ]) {
      expect(safeSetupIcon(value)).toBeUndefined();
    }
    expect(safeSetupIcon("data:image/png;base64,YWJj")).toBe(
      "data:image/png;base64,YWJj",
    );
  });
});

describe("imported workspace configuration", () => {
  const selected: import("./types").ManagedModelConfiguration = {
    profile: "engineering",
    providerProfile: "company",
    model: "company/engineering",
    contextWindowTokens: 131072,
    maxOutputTokens: 8192,
    reasoningEffort: "high",
    capabilities: { toolCalls: true, streaming: true, imageInputs: false },
  };
  it("preserves profiles, reasoning, limits, siblings and timeout while deferring a missing key", () => {
    const sibling = {
      ...selected,
      profile: "general",
      model: "company/general",
    };
    const result = importedWorkspaceConfiguration(
      { ...provider, timeoutMs: 120000, models: [selected, sibling] },
      { ...selected, maxOutputTokens: 4096 },
      "workspace",
      "minimal",
      "offline_isolated",
      "replace",
      null,
      {
        primary: "general",
        planner: "general",
        research: "other-provider-model",
      },
    );
    expect(result.providers).toEqual([
      {
        profile: "company",
        providerKind: "openai_compatible",
        baseUrl: provider.baseUrl,
        timeoutMs: 120000,
        credentialAction: "replace",
      },
    ]);
    expect(result.models).toEqual([
      sibling,
      { ...selected, maxOutputTokens: 4096 },
    ]);
    expect(result.roles).toEqual({
      primary: "engineering",
      planner: "general",
    });
  });
  it("supports a provider-only package and reuses only its selected credential", () => {
    const result = importedWorkspaceConfiguration(
      provider,
      { ...selected, providerProfile: "stale" },
      "workspace",
      "minimal",
      "offline_isolated",
      "reuse",
      "saved-key",
    );
    expect(result.models).toEqual([selected]);
    expect(result.providers[0]?.credentialId).toBe("saved-key");
  });
});
