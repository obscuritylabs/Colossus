// Inert DOM adapter for inspector previews. Presentation and budgets are shared.
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { MarkdownContent } from "@colossus/ui/conversation";
export function markdown(text: string): HTMLElement {
  const container = document.createElement("div"),
    root = createRoot(container);
  flushSync(() =>
    root.render(
      createElement(MarkdownContent, { content: text, className: "markdown" }),
    ),
  );
  const rendered = container.firstElementChild!.cloneNode(true) as HTMLElement;
  root.unmount();
  return rendered;
}
