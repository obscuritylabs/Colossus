import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProductRail } from "./ProductRail";

function renderRail(
  terminalAvailable: boolean,
  capabilities = {
    delegation: false,
    skills: false,
    tui: true,
    files: true,
    artifacts: true,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
): string {
  return renderToStaticMarkup(
    createElement(ProductRail, {
      surface: "work",
      attentionCount: 0,
      connectionState: "connected",
      terminalEnabled: true,
      terminalAvailable,
      capabilities,
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
  it("keeps workspace files within Work instead of adding a global destination", () => {
    const markup = renderRail(true);

    expect(markup).not.toContain(">Files</span>");
  });

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

  it("hides optional runtime areas until they are explicitly advertised", () => {
    const markup = renderRail(true, {
      delegation: false,
      skills: false,
      tui: false,
      files: false,
      artifacts: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    });

    expect(markup).not.toContain(">Agents</span>");
    expect(markup).not.toContain(">Library</span>");
    expect(markup).not.toContain(">TUI</span>");
  });

  it("shows orchestration for an authenticated delegation capability", () => {
    const markup = renderRail(true, {
      delegation: true,
      skills: false,
      tui: false,
      files: false,
      artifacts: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    });

    expect(markup).toContain(">Agents</span>");
  });
});
