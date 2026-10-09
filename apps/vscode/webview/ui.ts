import type { Preferences } from "../src/settings.js";

export function element<T extends HTMLElement = HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}
export function node(tag: string, text = "", className = ""): HTMLElement {
  const n = document.createElement(tag);
  n.textContent = text;
  n.className = className;
  return n;
}
export function brandMarks(root: HTMLElement) {
  for (const mark of root.querySelectorAll<HTMLImageElement>(
    "[data-brand-mark]",
  ))
    mark.src = document.body.dataset.colossusMark ?? "";
}
export function icon(name: string) {
  const span = node("span", "", `icon icon-${name}`);
  span.setAttribute("aria-hidden", "true");
  return span;
}

// VS Code updates body classes when the user switches themes. Use the matching
// Colossus Desktop token family; keep high contrast tied to the host.
function theme() {
  document.documentElement.dataset.theme =
    document.body.classList.contains("vscode-high-contrast") ||
    document.body.classList.contains("vscode-high-contrast-light")
      ? "high-contrast"
      : document.body.classList.contains("vscode-light")
        ? "light"
        : "dark";
}
theme();
new MutationObserver(theme).observe(document.body, {
  attributes: true,
  attributeFilter: ["class"],
});

export function applyPalette(palette: Preferences["palette"]) {
  document.documentElement.dataset.palette =
    palette === "editor" ? "neutral" : palette;
}
applyPalette("editor");
window.addEventListener(
  "message",
  (
    event: MessageEvent<{
      preferences?: { palette?: Preferences["palette"] };
      view?: { preferences?: { palette?: Preferences["palette"] } };
      type?: string;
      palette?: Preferences["palette"];
    }>,
  ) => {
    const palette =
      event.data.preferences?.palette ??
      event.data.view?.preferences?.palette ??
      (event.data.type === "palette" ? event.data.palette : undefined);
    if (
      palette === "editor" ||
      palette === "colossus" ||
      palette === "black" ||
      palette === "hacker"
    )
      applyPalette(palette);
  },
);
