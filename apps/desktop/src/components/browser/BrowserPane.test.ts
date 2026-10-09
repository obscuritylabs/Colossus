import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { BrowserPane } from "./BrowserPane";
import type { BrowserController } from "./useBrowser";

function render(
  overrides: Partial<BrowserController["snapshot"]["tabs"][number]> = {},
  snapshotOverrides: Partial<BrowserController["snapshot"]> = {},
) {
  const controller: BrowserController = {
    snapshot: {
      available: true,
      generation: 1,
      selectedTabId: "tab",
      tabs: [
        {
          id: "tab",
          title: "Docs",
          url: "https://example.com/",
          canGoBack: false,
          canGoForward: false,
          loading: false,
          error: null,
          notice: null,
          popupUrl: null,
          ...overrides,
        },
      ],
      ...snapshotOverrides,
    },
    error: "",
    busy: false,
    loading: false,
    fixture: true,
    command: vi.fn(async () => {}),
    viewport: vi.fn(async () => {}),
    certificates: vi.fn(async () => ({
      scope: "unsupported" as const,
      caImportAvailable: false,
      pfxImportAvailable: false,
      clientIdentitySelectionReady: false,
      acceptancePending: true,
      message: "Unavailable",
      fingerprintsSha256: [],
    })),
  };
  return renderToStaticMarkup(
    createElement(BrowserPane, {
      controller,
      expanded: false,
      onExpand: vi.fn(),
      onClose: vi.fn(),
    }),
  );
}

describe("browser controls", () => {
  it("reports a missing Chromium component without claiming a working browser", () => {
    const html = render(
      {},
      {
        available: false,
        engine: {
          kind: "embedded_chromium",
          preview: true,
          ready: false,
          message:
            "The verified Chromium component is not included in this build.",
          agentControlAvailable: false,
        },
      },
    );
    expect(html).toContain("Browser unavailable");
    expect(html).toContain("Control unavailable");
    expect(html).not.toContain("Human control");
    expect(html).toContain("The verified Chromium component is not included");
    expect(html).not.toContain("Chromium preview ·");
    expect(html).not.toContain("Resume agent");
  });
  it("keeps stop and close available while a page loads", () => {
    const html = render({ loading: true });
    expect(html).toContain('aria-label="Stop loading"');
    expect(html).toContain("Loading page…");
    expect(html).toContain('aria-label="Close browser pane"');
    expect(html).not.toContain('aria-label="Stop loading" disabled');
  });
  it("renders remote titles and popup addresses as plain text", () => {
    const html = render({
      title: '<img src=x onerror="alert(1)">',
      notice: "Open another tab",
      popupUrl: "https://example.com/?q=<script>secret</script>",
    });
    expect(html).not.toContain("<img src=x");
    expect(html).not.toContain("<script>secret");
    expect(html).toContain("&lt;img");
    expect(html).toContain("Open new tab");
  });
  it("provides a recoverable error without rendering a web page", () => {
    const html = render({ error: "The browser tab stopped responding." });
    expect(html).toContain("Unable to display this page");
    expect(html).toContain("Retry");
    expect(html).not.toContain("Native page content appears here");
  });
});
