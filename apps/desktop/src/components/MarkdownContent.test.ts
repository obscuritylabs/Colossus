import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  MarkdownContent,
  MAX_MARKDOWN_AST_NODES,
  MAX_MARKDOWN_CHARACTERS,
  markdownContentPropsAreEqual,
} from "./MarkdownContent";

function render(content: string): string {
  return renderToStaticMarkup(createElement(MarkdownContent, { content }));
}

describe("MarkdownContent", () => {
  it("renders sanitized Markdown and GitHub-flavored elements", () => {
    const markup = render(`
# Result

- [x] verified
  - nested detail
- **important** and \`inline code\`

| Agent | State |
| --- | --- |
| Builder | Ready |

\`\`\`ts
const ready = true;
\`\`\`
`);

    expect(markup).toContain("<h4>Result</h4>");
    expect(markup).toContain('<input type="checkbox"');
    expect(markup).toContain("disabled");
    expect(markup).toContain("nested detail");
    expect(markup).toContain("<strong>important</strong>");
    expect(markup).toContain("<code>inline code</code>");
    expect(markup).toContain('class="markdown-table-scroll"');
    expect(markup).toContain("<table>");
    expect(markup).toContain(
      'aria-label="Scrollable Markdown code block" tabindex="0"',
    );
    expect(markup).toContain('<code class="language-ts">');
  });

  it("normalizes the shallowest response heading beneath its article", () => {
    const markup = render("## Summary\n\n#### Detail");

    expect(markup).toContain("<h4>Summary</h4>");
    expect(markup).toContain("<h6>Detail</h6>");
    expect(markup).not.toContain("<h2>");
  });

  it("does not interpret raw HTML", () => {
    const markup = render(`
Before

<script src="https://attacker.test/payload.js">alert("xss")</script>
<div onclick="steal()">untrusted HTML</div>

After
`);

    expect(markup).not.toContain("<script");
    expect(markup).not.toContain("<div onclick");
    expect(markup).not.toContain("payload.js");
    expect(markup).not.toContain("steal()");
    expect(markup).toContain("Before");
    expect(markup).toContain("After");
  });

  it("replaces images without exposing or fetching their destinations", () => {
    const markup = render(`
![tracking pixel](https://attacker.test/track.png)

![embedded payload](data:image/svg+xml;base64,PHN2Zz4=)
`);

    expect(markup).not.toContain("<img");
    expect(markup).not.toContain("attacker.test");
    expect(markup).not.toContain("data:image");
    expect(markup).toContain("Image omitted: tracking pixel");
    expect(markup).toContain("Image omitted: embedded payload");
  });

  it("makes every model-authored link inert inside the privileged webview", () => {
    const markup = render(`
[Documentation](https://example.com/guide?q=colossus)
[Local docs](http://127.0.0.1:8080/docs)
[Script](javascript:alert(1))
[File](file:///etc/passwd)
[Relative](../../settings)
[Protocol relative](//example.com/path)
[Tauri](tauri://localhost/private)
`);

    expect(markup).not.toContain("<a ");
    expect(markup).not.toContain("href=");
    expect(markup.match(/class="markdown-link-inert"/g)).toHaveLength(7);
    expect(markup).not.toContain("javascript:");
    expect(markup).not.toContain("file://");
    expect(markup).not.toContain("../../settings");
    expect(markup).not.toContain("tauri://");
  });

  it("escapes HTML-looking content inside code blocks", () => {
    const markup = render("```html\n<img src=x onerror=alert(1)>\n```");

    expect(markup).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(markup).not.toContain("<img");
  });

  it("falls back to plain text above the Markdown parsing budget", () => {
    const content = `# Not parsed\n${"x".repeat(MAX_MARKDOWN_CHARACTERS)}`;
    const markup = render(content);

    expect(markup).toContain("markdown-content-fallback");
    expect(markup).toContain("Large response shown as plain text");
    expect(markup).not.toContain("<h4>Not parsed</h4>");
    expect(markup).toContain("# Not parsed");
  });

  it("collapses structurally dense Markdown before creating a large React tree", () => {
    const itemCount = MAX_MARKDOWN_CHARACTERS / 4;
    const content = "- x\n".repeat(itemCount);
    const markup = render(content);

    expect(itemCount).toBeGreaterThan(MAX_MARKDOWN_AST_NODES);
    expect(content.length).toBe(MAX_MARKDOWN_CHARACTERS);
    expect(markup).toContain("Complex response shown as plain text");
    expect(markup).not.toContain("<li>");
    expect(markup).toContain("- x");
  });

  it("collapses deeply nested Markdown before creating a deep React tree", () => {
    const content = `${"> ".repeat(40)}deep content`;
    const markup = render(content);

    expect(content.length).toBeLessThan(MAX_MARKDOWN_CHARACTERS);
    expect(markup).toContain("Complex response shown as plain text");
    expect(markup.match(/<blockquote>/g)).toHaveLength(1);
    expect(markup).toContain("&gt; &gt; &gt;");
  });

  it("memoizes unchanged response content and presentation", () => {
    expect(
      markdownContentPropsAreEqual(
        { content: "same", className: "response" },
        { content: "same", className: "response" },
      ),
    ).toBe(true);
    expect(
      markdownContentPropsAreEqual({ content: "before" }, { content: "after" }),
    ).toBe(false);
  });
});
