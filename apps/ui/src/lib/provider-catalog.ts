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
  document_inputs?: boolean;
  streaming?: boolean;
  server_compaction?: boolean;
  supported_reasoning_efforts?: string[];
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

export interface ProviderPreset {
  id: string;
  label: string;
  protocol: "chat_completions" | "responses" | "codex" | "anthropic_messages";
  baseUrl: string | null;
  credentialEnv: string | null;
}
export function presetProviderKind(preset: ProviderPreset): string {
  return {
    chat_completions: "openai_compatible",
    responses: "openai_responses",
    codex: "open_ai_codex",
    anthropic_messages: "anthropic_messages",
  }[preset.protocol];
}
export function matchingProviderPreset(
  presets: readonly ProviderPreset[],
  kind: string,
  baseUrl: string,
) {
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
