import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { MarkdownContent } from "../MarkdownContent";
import { BrowserLinkContext, webLink } from "./BrowserLink";

describe("browser link actions", () => {
  it("requires a trusted controller and a deliberate button action", () => {
    const open = vi.fn();
    const html = renderToStaticMarkup(
      createElement(
        BrowserLinkContext,
        { value: open },
        createElement(MarkdownContent, {
          content:
            "[Docs](https://example.com/docs) [File](file:///secret) ![Remote](https://example.com/image.png)",
        }),
      ),
    );
    expect(html).toContain('title="Open in browser: https://example.com/docs"');
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("file:///secret");
    expect(open).not.toHaveBeenCalled();
  });
  it("rejects credentials, control characters, and non-web schemes", () => {
    for (const value of [
      "javascript:alert(1)",
      "file:///x",
      "https://user:secret@example.com",
      "https://example.com/\nx",
      "//example.com",
    ])
      expect(webLink(value)).toBeNull();
    expect(webLink("https://example.com/docs")).toBe(
      "https://example.com/docs",
    );
  });
});
