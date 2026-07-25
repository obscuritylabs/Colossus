import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { WorkSurface } from "./WorkSurface";

function renderSurface(
  artifacts: readonly ArtifactViewItem[],
  capabilities = { files: true, artifacts: true },
): string {
  vi.stubGlobal("window", {
    matchMedia: () => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
  try {
    return renderToStaticMarkup(
      createElement(WorkSurface, {
        title: "Primary",
        view: undefined,
        conversationViews: [],
        connection: {
          state: "connected",
          message: "Connected securely.",
          targetId: "managed-local",
        },
        connecting: false,
        cancelling: false,
        runLoadError: "",
        actionError: null,
        participants: [],
        artifacts,
        composer: createElement("div"),
        filesPanel: createElement("div", null, "Workspace file explorer"),
        filesAvailable: capabilities.files,
        artifactsAvailable: capabilities.artifacts,
        workNavigationOpen: false,
        onConnect: vi.fn(),
        onCancel: vi.fn(),
        onRespond: vi.fn(async () => undefined),
        onResume: vi.fn(),
        onSuggestion: vi.fn(),
        onOpenWorkNavigation: vi.fn(),
        onCloseWorkNavigation: vi.fn(),
      }),
    );
  } finally {
    vi.unstubAllGlobals();
  }
}

describe("WorkSurface side panels", () => {
  it("keeps new work in the flexible conversation row when agent flow is absent", () => {
    const markup = renderSurface([]);

    expect(markup).toContain('<main class="work-surface is-new-work"');
    expect(markup).toContain('<section class="work-welcome">');
    expect(markup).not.toContain('class="agent-flow"');
  });

  it("keeps an empty artifact panel collapsed behind a count-bearing toggle", () => {
    const markup = renderSurface([]);

    expect(markup).toContain('aria-controls="work-side-drawer"');
    expect(markup.match(/aria-controls="work-side-drawer"/g)).toHaveLength(2);
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain("Open files panel");
    expect(markup).toContain("Open artifacts panel, 0 artifacts");
    expect(markup).toContain('<span class="artifact-count"');
    expect(markup).toContain(">0</span>");
    expect(markup).toContain('<div class="work-layout">');
    expect(markup).not.toContain("is-work-drawer-open");
  });

  it("shows the released artifact count without forcing the panel open", () => {
    const markup = renderSurface([
      {
        id: "artifact-1",
        fileName: "report.md",
        mediaType: "text/markdown",
        sizeLabel: "1 KB",
        stateLabel: "Available",
        createdLabel: "Recent",
      },
    ]);

    expect(markup).toContain("Open artifacts panel, 1 artifact");
    expect(markup).toContain(">1</span>");
    expect(markup).not.toContain("is-work-drawer-open");
  });

  it("does not imply drawers the runtime has not advertised", () => {
    const markup = renderSurface([], { files: false, artifacts: false });

    expect(markup).not.toContain("Open files panel");
    expect(markup).not.toContain("Open artifacts panel");
    expect(markup).not.toContain('id="work-side-drawer"');
  });
});
