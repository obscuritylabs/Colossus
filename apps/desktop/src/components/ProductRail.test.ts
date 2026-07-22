import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProductRail } from "./ProductRail";

function renderRail(terminalAvailable: boolean): string {
  return renderToStaticMarkup(
    createElement(ProductRail, {
      surface: "work",
      attentionCount: 0,
      connectionState: "connected",
      terminalEnabled: true,
      terminalAvailable,
      onSelect: vi.fn(),
      onOpenTerminal: vi.fn(),
    }),
  );
}

function terminalButton(markup: string): string {
  const labelIndex = markup.indexOf("TUI</span>");
  const buttonIndex = markup.lastIndexOf("<button", labelIndex);
  return markup.slice(buttonIndex, markup.indexOf(">", buttonIndex) + 1);
}

describe("ProductRail terminal availability", () => {
  it("does not expose the global terminal action for an external target", () => {
    const markup = renderRail(false);

    expect(terminalButton(markup)).toContain("disabled");
    expect(markup).toContain(
      "Terminal is available only for the selected Managed Local target",
    );
  });

  it("enables the global terminal action for an opted-in Managed Local target", () => {
    expect(terminalButton(renderRail(true))).not.toContain("disabled");
  });
});
