import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ComposerModelChip } from "./ComposerModelChip";

describe("ComposerModelChip", () => {
  it("offers an accessible settings shortcut for a local model", () => {
    const markup = renderToStaticMarkup(
      createElement(ComposerModelChip, {
        role: "primary",
        context: {
          targetKind: "managed_local",
          configuration: { providers: [], models: [], roles: {} },
        },
        onOpenSettings: vi.fn(),
      }),
    );
    expect(markup).toContain('type="button"');
    expect(markup).toContain(
      'aria-label="Model settings: Model not configured',
    );
    expect(markup).not.toContain("@primary");
  });

  it("does not offer local model settings for an external connection", () => {
    const markup = renderToStaticMarkup(
      createElement(ComposerModelChip, {
        role: "primary",
        context: {
          targetKind: "external_daemon",
          configuration: { providers: [], models: [], roles: {} },
        },
        onOpenSettings: vi.fn(),
      }),
    );
    expect(markup).toContain("Server-managed model");
    expect(markup).not.toContain("<button");
  });
});
