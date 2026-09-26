import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ProviderIcon } from "./ProviderIcon";

describe("ProviderIcon", () => {
  it.each([
    "codex",
    "openai",
    "openrouter",
    "groq",
    "together",
    "deepseek",
    "mistral",
    "ollama",
    "lmstudio",
  ])(
    "bundles a decorative asset for %s without fetching a remote image",
    (presetId) => {
      const markup = renderToStaticMarkup(
        createElement(ProviderIcon, { presetId }),
      );
      expect(markup).toContain("<img");
      expect(markup).toContain('alt=""');
      expect(markup).toContain('aria-hidden="true"');
      expect(markup).not.toMatch(/src="https?:/);
    },
  );

  it("renders a generic icon when a connection is unknown", () => {
    const markup = renderToStaticMarkup(
      createElement(ProviderIcon, { presetId: "custom-responses" }),
    );
    expect(markup).toContain('data-provider-brand="custom"');
    expect(markup).toContain("<svg");
    expect(markup).not.toContain("<img");
  });
});
import { createElement } from "react";
