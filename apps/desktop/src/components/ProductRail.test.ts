import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProductRail } from "./ProductRail";

function renderRail(
  terminalAvailable: boolean,
  capabilities = {
    delegation: false,
    plugins: false,
    tui: true,
    shellTerminal: true,
    files: true,
    artifacts: true,
    planContinuation: true,
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
      onOpenShell: vi.fn(),
    }),
  );
}

function buttonFor(markup: string, label: string): string {
  const labelIndex = markup.indexOf(`${label}</span>`);
  const buttonIndex = markup.lastIndexOf("<button", labelIndex);
  return markup.slice(buttonIndex, markup.indexOf(">", buttonIndex) + 1);
}

describe("ProductRail terminal availability", () => {
  it("keeps workspace files within Work instead of adding a global destination", () => {
    const markup = renderRail(true);

    expect(markup).not.toContain(">Files</span>");
    expect(markup).not.toContain(">Activity</span>");
  });

  it("does not expose the global terminal action for an external target", () => {
    const markup = renderRail(false);

    expect(buttonFor(markup, "TUI")).toContain("disabled");
    expect(markup).toContain(
      "Terminal is available only for the selected Managed Local target",
    );
  });

  it("enables the global terminal action for an opted-in Managed Local target", () => {
    expect(buttonFor(renderRail(true), "TUI")).not.toContain("disabled");
  });

  it("offers the embedded shell independently of TUI readiness", () => {
    const markup = renderRail(false, {
      delegation: false,
      plugins: false,
      tui: false,
      shellTerminal: true,
      files: false,
      artifacts: false,
      planContinuation: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    });

    expect(markup).toContain(">Terminal</span>");
    expect(buttonFor(markup, "Terminal")).not.toContain("disabled");
  });

  it("hides optional runtime areas until they are explicitly advertised", () => {
    const markup = renderRail(true, {
      delegation: false,
      plugins: false,
      tui: false,
      shellTerminal: false,
      files: false,
      artifacts: false,
      planContinuation: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    });

    expect(markup).not.toContain(">Agents</span>");
    expect(markup).not.toContain(">Library</span>");
    expect(markup).not.toContain(">TUI</span>");
    expect(markup).not.toContain(">Terminal</span>");
  });

  it("shows orchestration for an authenticated delegation capability", () => {
    const markup = renderRail(true, {
      delegation: true,
      plugins: false,
      tui: false,
      shellTerminal: false,
      files: false,
      artifacts: false,
      planContinuation: false,
      updateAvailable: false,
      agentWorkflows: false,
      attachments: false,
    });

    expect(markup).toContain(">Agents</span>");
  });
});
