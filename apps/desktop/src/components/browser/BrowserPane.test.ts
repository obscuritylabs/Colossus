import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { BrowserPane } from "./BrowserPane";
import type { BrowserController } from "./useBrowser";

function render(
  overrides: Partial<BrowserController["snapshot"]["tabs"][number]> = {},
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
    },
    error: "",
    busy: false,
    fixture: true,
    command: vi.fn(async () => {}),
    viewport: vi.fn(async () => {}),
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
