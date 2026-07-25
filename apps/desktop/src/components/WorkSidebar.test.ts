import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Run } from "../types";
import { WorkSidebar } from "./WorkSidebar";

const RUN: Run = {
  runId: "run-sidebar",
  sessionId: "session-sidebar",
  title: "Improve the Work sidebar",
  role: "primary",
  mode: "execute",
  status: "running",
  createdAt: "2026-07-24T18:00:00Z",
  updatedAt: "2026-07-24T18:01:00Z",
  startedAt: "2026-07-24T18:00:01Z",
  finishedAt: null,
  lastSequence: 2,
  pendingInteractionCount: 0,
  terminal: null,
  etag: "etag-sidebar",
  selectedSkills: [],
};

describe("WorkSidebar", () => {
  it("shows workspace context once and uses the durable run title", () => {
    const markup = renderToStaticMarkup(
      createElement(WorkSidebar, {
        runs: [RUN],
        workspace: {
          workspaceId: "workspace-colossus",
          displayName: "Colossus",
          displayPath: "~/tools/Colossus",
        },
        activeSessionId: RUN.sessionId,
        query: "",
        busy: false,
        error: "",
        hasMore: false,
        disabled: false,
        drawerOpen: false,
        onQueryChange: vi.fn(),
        onNewWork: vi.fn(),
        onSelect: vi.fn(),
        onLoadMore: vi.fn(),
        onDrawerOpen: vi.fn(),
        onDrawerClose: vi.fn(),
      }),
    );

    expect(markup).toContain("Workspace");
    expect(markup).toContain("Colossus");
    expect(markup).toContain("Improve the Work sidebar");
    expect(markup).not.toContain("<strong>Primary</strong>");
  });
});
