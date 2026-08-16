import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  ResearchSourcesPanel,
  researchSources,
  workspaceSourcePath,
} from "./ResearchSourcesPanel";

describe("ResearchSourcesPanel", () => {
  it("extracts only the bounded released Sources section", () => {
    const sources = researchSources(
      "# Report\n\n- Finding [R1]\n\n## Sources\n\n- [R1] Primary documentation — https://example.test/docs\n- [R2] Workspace note — repo://README.md\n",
    );

    expect(sources).toEqual([
      {
        label: "R1",
        title: "Primary documentation",
        uri: "https://example.test/docs",
      },
      { label: "R2", title: "Workspace note", uri: "repo://README.md" },
    ]);
  });

  it("links web evidence and offers selected-Space files through the viewer", () => {
    const markup = renderToStaticMarkup(
      createElement(ResearchSourcesPanel, {
        output:
          "## Sources\n\n- [R1] Web — https://example.test\n- [R2] Repo — repo://README.md",
        running: false,
        onOpenWorkspaceFile: vi.fn(),
      }),
    );

    expect(markup).toContain('href="https://example.test"');
    expect(markup).toContain("Open file");
    expect(markup).toContain("README.md");
  });

  it("only derives bounded safe relative file paths", () => {
    expect(workspaceSourcePath("repo://docs/runtime.md#section")).toBe(
      "docs/runtime.md",
    );
    expect(workspaceSourcePath("Cargo.toml")).toBe("Cargo.toml");
    expect(workspaceSourcePath("repo://../secret.txt")).toBeNull();
    expect(workspaceSourcePath("file:///tmp/secret.txt")).toBeNull();
    expect(workspaceSourcePath("mcp://server/resource")).toBeNull();
  });
});
