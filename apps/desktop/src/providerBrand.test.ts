import { describe, expect, it } from "vitest";
import { providerConnectionBrand, providerPresetBrand } from "./providerBrand";

describe("provider branding", () => {
  it.each([
    ["openai", "https://api.openai.com/v1"],
    ["openrouter", "https://openrouter.ai/api/v1"],
    ["groq", "https://api.groq.com/openai/v1"],
    ["together", "https://api.together.xyz/v1"],
    ["deepseek", "https://api.deepseek.com/v1"],
    ["mistral", "https://api.mistral.ai/v1"],
    ["ollama", "http://localhost:11434/v1"],
    ["lmstudio", "http://localhost:1234/v1"],
  ])("recognizes the %s preset and saved connection", (brand, baseUrl) => {
    expect(providerPresetBrand(brand)).toBe(brand);
    for (const kind of ["openai_compatible", "openai_responses"] as const) {
      expect(providerConnectionBrand({ kind, baseUrl })).toBe(brand);
      expect(providerConnectionBrand({ kind, baseUrl: `${baseUrl}/` })).toBe(
        brand,
      );
    }
  });

  it("identifies the Codex subscription by its adapter", () => {
    expect(providerPresetBrand("codex")).toBe("openai");
    expect(providerConnectionBrand({ kind: "open_ai_codex" })).toBe("openai");
  });

  it.each([
    "https://openrouter.ai.example.test/api/v1",
    "https://example.test/openrouter.ai/api/v1",
    "https://openrouter.ai@example.test/v1",
    "https://user:secret@openrouter.ai/api/v1",
    "http://openrouter.ai/api/v1",
    "https://openrouter.ai:8443/api/v1",
    "https://example.test:11434/v1",
    "http://localhost:11434/proxy/v1",
    "http://localhost:1234/v1?upstream=custom",
    "not a URL",
    "",
  ])("uses a generic icon for the custom endpoint %s", (baseUrl) => {
    expect(
      providerConnectionBrand({ kind: "openai_compatible", baseUrl }),
    ).toBeNull();
  });

  it("recognizes normalized hosts and standard loopback addresses", () => {
    expect(
      providerConnectionBrand({
        kind: "openai_responses",
        baseUrl: "https://API.OPENAI.COM:443/v1/",
      }),
    ).toBe("openai");
    expect(
      providerConnectionBrand({
        kind: "openai_compatible",
        baseUrl: "http://127.0.0.1:11434/v1",
      }),
    ).toBe("ollama");
    expect(
      providerConnectionBrand({
        kind: "openai_compatible",
        baseUrl: "http://[::1]:1234/v1",
      }),
    ).toBe("lmstudio");
  });

  it("does not brand missing connections or unknown and custom presets", () => {
    expect(providerConnectionBrand()).toBeNull();
    expect(providerConnectionBrand({ kind: "openai_compatible" })).toBeNull();
    for (const id of [
      "custom-chat",
      "custom-responses",
      "new-provider",
      "constructor",
    ]) {
      expect(providerPresetBrand(id)).toBeNull();
    }
  });
});
