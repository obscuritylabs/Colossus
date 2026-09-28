import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { HighlightedCode } from "./SourcePreview";

it("bounds line-heavy previews and treats file content as text", () => {
  const content =
    "<script>alert('untrusted')</script>\n" + "row\n".repeat(7000);
  const markup = renderToStaticMarkup(
    createElement(HighlightedCode, {
      colorTheme: "dark",
      file: {
        name: "many.txt",
        path: "many.txt",
        content,
        language: "text",
        sizeBytes: content.length,
        lineCount: 7002,
      },
    }),
  );
  expect(markup).toContain("This preview is truncated");
  expect(markup.match(/class="file-code-line"/g)).toHaveLength(6000);
  expect(markup).not.toContain("<script>");
  expect(markup).toContain("&lt;script&gt;");
});
