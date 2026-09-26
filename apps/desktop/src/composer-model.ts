import {
  providerConnectionBrand,
  type ProviderConnectionIdentity,
} from "./providerBrand";
import type {
  ManagedConfiguration,
  ReasoningEffort,
  RuntimeTargetKind,
} from "./types";

export interface ComposerModelContext {
  targetKind: RuntimeTargetKind | null;
  configuration: ManagedConfiguration;
}

export interface ComposerModelIdentity {
  model: string;
  providerLabel: string;
  provider: ProviderConnectionIdentity | null;
  reasoningEffort: ReasoningEffort | null;
}

const brandLabels = {
  openai: "OpenAI",
  openrouter: "OpenRouter",
  groq: "Groq",
  together: "Together AI",
  deepseek: "DeepSeek",
  mistral: "Mistral",
  ollama: "Ollama",
  lmstudio: "LM Studio",
};

/** Present the saved route for the next message; never choose or change a route. */
export function composerModelIdentity(
  context: ComposerModelContext | undefined,
  role: string,
): ComposerModelIdentity {
  const unavailable = { provider: null, reasoningEffort: null };
  if (context?.targetKind === "external_daemon") {
    return {
      ...unavailable,
      model: "Server-managed model",
      providerLabel: "External connection",
    };
  }
  if (context?.targetKind !== "managed_local") {
    return {
      ...unavailable,
      model: "No model selected",
      providerLabel: "Choose a connection",
    };
  }
  const { configuration } = context;
  // The registry uses primary for an unconfigured specialized role. An explicit
  // mapping to a missing model must not display an unrelated primary model.
  const profile =
    configuration.roles[role.trim()] ?? configuration.roles.primary;
  const model = configuration.models.find((entry) => entry.profile === profile);
  if (!role.trim() || !model) {
    return {
      ...unavailable,
      model: "Model not configured",
      providerLabel: "Check model settings",
    };
  }
  const connection = configuration.providers.find(
    (entry) => entry.profile === model.providerProfile,
  );
  if (!connection) {
    return {
      ...unavailable,
      model: model.model,
      providerLabel: "Provider unavailable",
    };
  }
  const provider = {
    kind: connection.providerKind,
    baseUrl: connection.baseUrl,
  };
  const brand = providerConnectionBrand(provider);
  let providerLabel =
    provider.kind === "open_ai_codex"
      ? "Codex subscription"
      : brand
        ? brandLabels[brand]
        : "Custom provider";
  if (!brand) {
    try {
      // Only show the host, never URL credentials, paths, or query parameters.
      const url = new URL(connection.baseUrl);
      if (url.protocol === "https:" || url.protocol === "http:")
        providerLabel = url.host;
    } catch {
      /* Keep the generic label for an incomplete endpoint. */
    }
  }
  return {
    model: model.model,
    providerLabel,
    provider,
    reasoningEffort: model.reasoningEffort,
  };
}
