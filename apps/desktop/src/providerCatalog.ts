import type { ManagedModelConfiguration, ProviderKind } from "./types";

/** Shared runtime presets. Secrets and credentials never enter these records. */
export interface ProviderPreset {
  id: string;
  label: string;
  protocol: "chat_completions" | "responses" | "codex";
  baseUrl: string | null;
  credentialEnv: string | null;
  setup?: {
    packageId: string;
    provider: import("./setupPackages").SetupProvider;
  };
}

export interface ProviderCatalogModel {
  id: string;
  object?: string | null;
  owned_by?: string | null;
  display_name?: string;
  description?: string;
  context_window_tokens?: number;
  max_output_tokens?: number;
  tool_calls?: boolean;
  image_inputs?: boolean;
  streaming?: boolean;
  server_compaction?: boolean;
  supported_reasoning_efforts?: string[];
}

export function presetProviderKind(preset: ProviderPreset): ProviderKind {
  return preset.protocol === "codex"
    ? "open_ai_codex"
    : preset.protocol === "responses"
      ? "openai_responses"
      : "openai_compatible";
}

export function matchingProviderPreset(
  presets: readonly ProviderPreset[],
  kind: ProviderKind,
  baseUrl: string,
): ProviderPreset | undefined {
  return (
    presets.find(
      (preset) =>
        presetProviderKind(preset) === kind &&
        preset.baseUrl !== null &&
        preset.baseUrl.replace(/\/+$/, "") === baseUrl.replace(/\/+$/, ""),
    ) ??
    presets.find(
      (preset) =>
        presetProviderKind(preset) === kind && preset.baseUrl === null,
    )
  );
}

/** Import model-card declarations separately from saved feature preferences. */
export function applyCatalogModel(
  current: ManagedModelConfiguration,
  model: ProviderCatalogModel,
): ManagedModelConfiguration {
  const contextWindowTokens =
    model.context_window_tokens ?? current.contextWindowTokens;
  return {
    ...current,
    model: model.id,
    contextWindowTokens,
    maxOutputTokens: Math.min(
      model.max_output_tokens ?? current.maxOutputTokens,
      Math.floor(contextWindowTokens / 2),
    ),
    capabilities: {
      ...current.capabilities,
      declared: {
        toolCalls: model.tool_calls ?? null,
        streaming: model.streaming ?? null,
        imageInputs: model.image_inputs ?? null,
        serverCompaction: model.server_compaction ?? null,
      },
    },
  };
}

/** Model changes invalidate metadata from the previous catalog selection. */
export function resetModelMetadata(
  current: ManagedModelConfiguration,
): ManagedModelConfiguration {
  return {
    ...current,
    contextWindowTokens: 32_768,
    maxOutputTokens: 4_096,
    reasoningEffort: null,
    capabilities: {
      ...current.capabilities,
      declared: {
        toolCalls: null,
        streaming: null,
        imageInputs: null,
        serverCompaction: null,
      },
    },
  };
}

export function selectCatalogModel(
  current: ManagedModelConfiguration,
  model: ProviderCatalogModel,
): ManagedModelConfiguration {
  return applyCatalogModel(resetModelMetadata(current), model);
}

export function filterCatalogModels(
  models: readonly ProviderCatalogModel[],
  query: string,
) {
  const needle = query.trim().toLocaleLowerCase();
  return models.filter((model) =>
    [model.id, model.display_name, model.owned_by].some((value) =>
      value?.toLocaleLowerCase().includes(needle),
    ),
  );
}
