// The inspector owns this mounted view; Markdown navigation remains inert.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MarkdownContent } from "@colossus/ui/conversation";

export class MarkdownPreview {
  readonly element = document.createElement("div");
  private readonly renderer: Root;

  constructor(text: string) {
    this.renderer = createRoot(this.element);
    this.renderer.render(
      createElement(MarkdownContent, { content: text, className: "markdown" }),
    );
  }

  dispose() {
    this.renderer.unmount();
  }
}
