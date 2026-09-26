import { describe, expect, it } from "vitest";
import {
  applyCatalogModel,
  filterCatalogModels,
  matchingProviderPreset,
  presetProviderKind,
  resetModelMetadata,
  selectCatalogModel,
} from "./providerCatalog";
import type { ManagedModelConfiguration } from "./types";

const configured: ManagedModelConfiguration = {
  profile: "primary",
  providerProfile: "remote",
  model: "old-model",
  contextWindowTokens: 128000,
  maxOutputTokens: 16000,
  reasoningEffort: "high",
  capabilities: { toolCalls: true, streaming: true, imageInputs: true },
};

describe("provider catalog selection", () => {
  it("maps a shared preset to its adapter and matches URL trailing slashes", () => {
    const preset = {
      id: "example",
      label: "Example",
      protocol: "responses" as const,
      baseUrl: "https://example.test/v1",
      credentialEnv: "EXAMPLE_KEY",
    };
    expect(presetProviderKind(preset)).toBe("openai_responses");
    expect(
      matchingProviderPreset(
        [preset],
        "openai_responses",
        "https://example.test/v1/",
      ),
    ).toBe(preset);
  });

  it("reserves input context when a provider advertises a larger output allowance", () => {
    expect(
      applyCatalogModel(configured, {
        id: "small",
        context_window_tokens: 8192,
        max_output_tokens: 8192,
      }),
    ).toMatchObject({ contextWindowTokens: 8192, maxOutputTokens: 4096 });
  });

  it("keeps unknown catalog fields unknown and resets metadata from a different model", () => {
    const selected = selectCatalogModel(configured, { id: "unknown" });
    expect(selected).toMatchObject({
      model: "unknown",
      contextWindowTokens: 32768,
      maxOutputTokens: 4096,
      reasoningEffort: null,
      capabilities: { toolCalls: false, streaming: false, imageInputs: false },
    });
    expect(
      selectCatalogModel(selected, { id: "known", tool_calls: true }),
    ).toMatchObject({
      capabilities: { toolCalls: true, streaming: false, imageInputs: false },
    });
  });

  it("imports metadata when the selected catalog ID was previously entered manually", () => {
    expect(
      selectCatalogModel(configured, {
        id: "old-model",
        context_window_tokens: 200000,
      }),
    ).toMatchObject({ model: "old-model", contextWindowTokens: 200000 });
    expect(resetModelMetadata(configured)).toMatchObject({
      profile: "primary",
      providerProfile: "remote",
      contextWindowTokens: 32768,
      capabilities: { toolCalls: false },
    });
  });

  it("searches model identifiers, display names and owners without altering results", () => {
    const models = [
      { id: "a", display_name: "Small Reasoner" },
      { id: "b", owned_by: "Example" },
    ];
    expect(filterCatalogModels(models, " reasoner ")).toEqual([models[0]]);
    expect(filterCatalogModels(models, "EXAMPLE")).toEqual([models[1]]);
  });
});
