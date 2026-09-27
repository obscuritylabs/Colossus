import type { ManagedModelConfiguration, ProviderKind } from "./types";

/** Shared runtime presets. Secrets and credentials never enter these records. */
export interface ProviderPreset {
  id: string;
  label: string;
  protocol: "chat_completions" | "responses" | "codex";
  baseUrl: string | null;
  credentialEnv: string | null;
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

/** Only advertised fields overwrite the user's explicit configuration. */
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
      toolCalls: model.tool_calls ?? current.capabilities.toolCalls,
      streaming: model.streaming ?? current.capabilities.streaming,
      imageInputs: model.image_inputs ?? current.capabilities.imageInputs,
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
    capabilities: { toolCalls: false, streaming: false, imageInputs: false },
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
