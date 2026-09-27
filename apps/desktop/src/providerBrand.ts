import type { ProviderKind } from "./types";

export type ProviderBrand =
  | "openai"
  | "openrouter"
  | "groq"
  | "together"
  | "deepseek"
  | "mistral"
  | "ollama"
  | "lmstudio";

export interface ProviderConnectionIdentity {
  kind: ProviderKind;
  baseUrl?: string | null;
}

export function providerPresetBrand(id: string): ProviderBrand | null {
  switch (id) {
    case "codex":
      return "openai";
    case "openai":
    case "openrouter":
    case "groq":
    case "together":
    case "deepseek":
    case "mistral":
    case "ollama":
    case "lmstudio":
      return id;
    default:
      return null;
  }
}

/** Connection identity is independent of user-editable names and model IDs. */
export function providerConnectionBrand(
  provider?: ProviderConnectionIdentity | null,
): ProviderBrand | null {
  if (!provider) return null;
  if (provider.kind === "open_ai_codex") return "openai";
  if (!provider.baseUrl) return null;
  let url: URL;
  try {
    url = new URL(provider.baseUrl);
  } catch {
    return null;
  }
  if (url.username || url.password) return null;
  if (url.protocol === "https:" && !url.port) {
    switch (url.hostname) {
      case "api.openai.com":
        return "openai";
      case "openrouter.ai":
        return "openrouter";
      case "api.groq.com":
        return "groq";
      case "api.together.xyz":
        return "together";
      case "api.deepseek.com":
        return "deepseek";
      case "api.mistral.ai":
        return "mistral";
    }
  }
  // Local presets have no provider hostname. Recognize their standard loopback
  // endpoints only; arbitrary remote servers and proxies retain the generic icon.
  if (
    (url.protocol === "http:" || url.protocol === "https:") &&
    ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname) &&
    url.pathname.replace(/\/+$/, "") === "/v1" &&
    !url.search &&
    !url.hash
  ) {
    if (url.port === "11434") return "ollama";
    if (url.port === "1234") return "lmstudio";
  }
  return null;
}
