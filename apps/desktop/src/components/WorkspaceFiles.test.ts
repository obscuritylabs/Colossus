import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { WorkspaceSummary } from "../types";
import { WorkspaceFiles } from "./WorkspaceFiles";

const workspace: WorkspaceSummary = {
  workspaceId: "workspace-opaque-1",
  displayName: "Colossus",
  displayPath: "~/tools/Colossus",
};

function renderFiles(available: boolean): string {
  return renderToStaticMarkup(
    createElement(WorkspaceFiles, {
      workspace,
      available,
      listDirectory: vi.fn(),
      readFile: vi.fn(),
      onOpenSettings: vi.fn(),
      openRequest: null,
    }),
  );
}

describe("WorkspaceFiles", () => {
  it("establishes the explorer and read-only preview hierarchy", () => {
    const markup = renderFiles(true);

    expect(markup).toContain('class="workspace-files-drawer"');
    expect(markup).toContain('aria-label="Workspace files"');
    expect(markup).toContain("<h1>Colossus</h1>");
    expect(markup).toContain("~/tools/Colossus");
    expect(markup).toContain("Read-only");
    expect(markup).toContain("Select a file to preview");
    expect(markup).toContain("existing policy and approval path");
    expect(markup).not.toContain('id="primary-workspace"');
  });

  it("does not imply workspace access when the selected target is ineligible", () => {
    const markup = renderFiles(false);

    expect(markup).toContain("Managed Local files unavailable");
    expect(markup).toContain("enable Development or Allow all access");
    expect(markup).toContain("Open settings");
  });
});
