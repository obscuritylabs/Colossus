import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ArtifactWorkspace } from "./ArtifactWorkspace";
import type { ArtifactViewItem } from "./ArtifactWorkspace";

function artifact(index: number): ArtifactViewItem {
  return {
    id: `artifact-${index}`,
    fileName: `artifact-${index}.md`,
    mediaType: "text/markdown",
    sizeLabel: `${index} KB`,
    stateLabel: "Available",
    createdLabel: "Recent",
  };
}

describe("ArtifactWorkspace", () => {
  it("keeps every bounded artifact reachable and selects tabs past the fifth item", () => {
    const artifacts = Array.from({ length: 7 }, (_, index) =>
      artifact(index + 1),
    );
    const markup = renderToStaticMarkup(
      createElement(ArtifactWorkspace, {
        artifacts,
        selectedId: "artifact-7",
      }),
    );

    expect(markup.match(/role="tab"/g)).toHaveLength(7);
    expect(markup).toContain("artifact-7.md");
    expect(markup).toMatch(
      /aria-selected="true"[^>]*data-artifact-id="artifact-7"/,
    );
  });
});
