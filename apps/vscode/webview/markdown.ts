import { fromMarkdown } from "mdast-util-from-markdown";
import { gfmTableFromMarkdown } from "mdast-util-gfm-table";
import { gfmTable } from "micromark-extension-gfm-table";
import type { RootContent, Root, Nodes } from "mdast";
import { node } from "./ui.js";

let tableNumber = 0;

// Match Desktop's response budget. Only create known DOM elements; HTML, images,
// links and code never receive navigation, resource-loading or executable authority.
export function markdown(text: string): HTMLElement {
  const container = node("div", "", "markdown");
  const plain = () => {
    container.classList.add("plain-text");
    container.textContent = text;
    return container;
  };
  if (text.length > 16_384) return plain();
  let root: Root;
  try {
    root = fromMarkdown(text, {
      extensions: [gfmTable()],
      mdastExtensions: [gfmTableFromMarkdown()],
    });
  } catch {
    return plain();
  }
  const stack: { n: Nodes; depth: number }[] = [{ n: root, depth: 0 }];
  let count = 0;
  while (stack.length) {
    const { n, depth } = stack.pop()!;
    if (++count > 1_000 || depth > 32) return plain();
    if ("children" in n)
      for (const child of n.children)
        stack.push({ n: child, depth: depth + 1 });
  }
  function render(n: RootContent): Node {
    if (n.type === "text" || n.type === "html")
      return document.createTextNode(n.value);
    if (n.type === "image" || n.type === "imageReference")
      return document.createTextNode(`Image omitted: ${n.alt ?? ""}`);
    if (n.type === "definition") return document.createTextNode("");
    if (n.type === "code") {
      const pre = node("pre");
      pre.tabIndex = 0;
      pre.setAttribute("aria-label", "Code block");
      pre.append(node("code", n.value));
      return pre;
    }
    if (n.type === "inlineCode") return node("code", n.value);
    if (n.type === "table") {
      const scroll = node("div", "", "markdown-table-scroll");
      scroll.tabIndex = 0;
      scroll.setAttribute("role", "region");
      scroll.setAttribute(
        "aria-label",
        `Scrollable Markdown table ${++tableNumber}`,
      );
      const table = node("table");
      const head = node("thead");
      const body = node("tbody");
      n.children.forEach((row, index) => {
        const tr = node("tr");
        row.children.forEach((cell, column) => {
          const el = node(index === 0 ? "th" : "td");
          if (index === 0) el.setAttribute("scope", "col");
          const align = n.align?.[column];
          if (align === "left" || align === "right" || align === "center")
            el.className = `align-${align}`;
          for (const child of cell.children) el.append(render(child));
          tr.append(el);
        });
        (index === 0 ? head : body).append(tr);
      });
      table.append(head, body);
      scroll.append(table);
      return scroll;
    }
    const tag =
      n.type === "paragraph"
        ? "p"
        : n.type === "heading"
          ? "h3"
          : n.type === "strong"
            ? "strong"
            : n.type === "emphasis"
              ? "em"
              : n.type === "blockquote"
                ? "blockquote"
                : n.type === "list"
                  ? n.ordered
                    ? "ol"
                    : "ul"
                  : n.type === "listItem"
                    ? "li"
                    : n.type === "break"
                      ? "br"
                      : n.type === "thematicBreak"
                        ? "hr"
                        : "span";
    const el = node(tag);
    if (n.type === "list" && n.ordered && n.start != null)
      (el as HTMLOListElement).start = n.start;
    if ("children" in n)
      for (const child of n.children) el.append(render(child));
    return el;
  }
  for (const child of root.children) container.append(render(child));
  return container;
}
