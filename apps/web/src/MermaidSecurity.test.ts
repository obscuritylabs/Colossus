// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { sanitizeMermaidSvg } from "../../ui/src/markdown/mermaid-security";

describe("released Mermaid SVG boundary", () => {
  it("copies bounded geometry and text while discarding executable content and external references", () => {
    const tree = sanitizeMermaidSvg(
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 100" onload="steal()">
      <style>text{fill:url(https://example.invalid/secret)}</style>
      <script>steal()</script><foreignObject><div>unsafe</div></foreignObject>
      <a href="https://example.invalid"><text>linked</text></a>
      <image href="https://example.invalid/pixel"/><use href="https://example.invalid/shape"/>
      <defs><marker id="arrow"><path d="M 0 0 L 10 5 L 0 10 Z"/></marker></defs>
      <rect width="100" height="40" style="fill:red" class="model-class"/>
      <path d="M 0 0 L 100 0" marker-end="url(#arrow)"/>
      <text x="12" dy="1em">Safe label</text>
    </svg>`,
      "diagram-fixture",
      new DOMParser(),
    );
    const serialized = JSON.stringify(tree);
    expect(tree?.tag).toBe("svg");
    expect(serialized).toContain("Safe label");
    expect(serialized).toContain('"dy":"1em"');
    expect(serialized).toContain("url(#diagram-fixture-");
    for (const forbidden of [
      "onload",
      "steal",
      "unsafe",
      "linked",
      "example.invalid",
      "style",
      "model-class",
      "foreignObject",
      '"tag":"image"',
      '"tag":"use"',
    ])
      expect(serialized).not.toContain(forbidden);
  });

  it("rejects oversized, deep, malformed or entity-bearing SVG instead of inserting it", () => {
    for (const svg of [
      '<!DOCTYPE svg [<!ENTITY x "payload">]><svg xmlns="http://www.w3.org/2000/svg">&x;</svg>',
      '<svg xmlns="http://www.w3.org/2000/svg">' +
        "<g>".repeat(40) +
        "</g>".repeat(40) +
        "</svg>",
      '<svg xmlns="http://www.w3.org/2000/svg">' +
        '<rect width="1"/>'.repeat(2_001) +
        "</svg>",
      '<svg xmlns="http://www.w3.org/2000/svg"><text>' +
        "x".repeat(240_000) +
        "</text></svg>",
      "<html>outside SVG</html>",
    ])
      expect(
        sanitizeMermaidSvg(svg, "diagram-fixture", new DOMParser()),
      ).toBeNull();
  });
});
