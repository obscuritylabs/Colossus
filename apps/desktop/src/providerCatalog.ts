import type { ManagedModelConfiguration, ProviderKind } from "./types";

/** Shared runtime presets. Secrets and credentials never enter these records. */
export interface ProviderPreset {
  id: string;
  label: string;
  protocol: "chat_completions" | "responses" | "codex" | "anthropic_messages";
  baseUrl: string | null;
  credentialEnv: string | null;
  setup?: {
    packageId: string;
    provider: import("./setupPackages").SetupProvider;
  };
}

import type { ProviderCatalogModel } from "@colossus/ui/lib/provider-catalog";
export type { ProviderCatalogModel } from "@colossus/ui/lib/provider-catalog";
export { filterCatalogModels } from "@colossus/ui/lib/provider-catalog";

/** Native credential labels allow at most 96 UTF-8 bytes, including the suffix. */
export function providerCredentialLabel(provider: string): string {
  const suffix = " API key";
  const encoder = new TextEncoder();
  let prefix = "";
  for (const character of provider.trim() || "Provider") {
    if (encoder.encode(prefix + character + suffix).length > 96) break;
    prefix += character;
  }
  return prefix.trimEnd() + suffix;
}

export function presetProviderKind(preset: ProviderPreset): ProviderKind {
  if (preset.protocol === "anthropic_messages") return "anthropic_messages";
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
        documentInputs: model.document_inputs ?? null,
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
        documentInputs: null,
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
