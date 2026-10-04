import { fromMarkdown } from "mdast-util-from-markdown";
import type { RootContent, Root, Nodes } from "mdast";
import { node } from "./ui.js";

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
    root = fromMarkdown(text);
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
