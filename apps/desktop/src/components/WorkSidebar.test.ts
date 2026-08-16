import { createElement } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Run, SpaceSearchResult, SpaceSummary } from "../types";
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
  archived: false,
};

const SPACE: SpaceSummary = {
  spaceId: "space-colossus",
  targetId: "space-colossus",
  displayName: "Colossus",
  displayPath: "~/tools/Colossus",
  archived: false,
  lastOpenedAtMs: 1,
  lastActivityAt: RUN.updatedAt,
  state: "ready",
  message: "Ready",
  selected: true,
  attentionCount: 0,
  providerConfigured: true,
};

const CAPABILITIES = {
  delegation: true,
  skills: true,
  tui: true,
  shellTerminal: true,
  files: true,
  artifacts: true,
  planContinuation: true,
  updateAvailable: false,
  agentWorkflows: true,
  attachments: true,
};

const BASE_PROPS: ComponentProps<typeof WorkSidebar> = {
  runs: [RUN],
  spaces: [SPACE],
  selectedSpaceId: SPACE.spaceId,
  surface: "work",
  connectionState: "connected",
  capabilities: CAPABILITIES,
  terminalEnabled: true,
  terminalAvailable: true,
  activeSessionId: RUN.sessionId,
  pinnedSessionIds: new Set(),
  query: "",
  searchScope: "space",
  includeArchived: false,
  searchResults: [],
  searchBusy: false,
  searchError: "",
  searchHasMore: false,
  spaceThreadPreviews: new Map(),
  spaceThreadPreviewBusyIds: new Set(),
  spaceThreadPreviewErrors: new Map(),
  busy: false,
  error: "",
  hasMore: false,
  disabled: false,
  spaceStartup: null,
  threadLifecycleBusySessionId: null,
  sidebarWidth: null,
  drawerOpen: false,
  onQueryChange: vi.fn(),
  onSearchScopeChange: vi.fn(),
  onIncludeArchivedChange: vi.fn(),
  onNewWork: vi.fn(),
  onSelect: vi.fn(),
  onSelectSearchResult: vi.fn(),
  onLoadMore: vi.fn(),
  onLoadMoreSearch: vi.fn(),
  onLoadSpaceThreadPreview: vi.fn(),
  onSelectSpace: vi.fn(),
  onCreateSpace: vi.fn(),
  onRenameSpace: vi.fn(),
  onArchiveSpace: vi.fn(),
  onRestoreSpace: vi.fn(),
  onArchiveThread: vi.fn(),
  onToggleThreadPinned: vi.fn(),
  onRestoreThread: vi.fn(),
  onSelectSurface: vi.fn(),
  onOpenTerminal: vi.fn(),
  onOpenShell: vi.fn(),
  onSidebarWidthPreview: vi.fn(),
  onSidebarWidthCommit: vi.fn(),
  onSidebarWidthReset: vi.fn(),
  onDrawerOpen: vi.fn(),
  onDrawerClose: vi.fn(),
};

function renderSidebar(
  overrides: Partial<ComponentProps<typeof WorkSidebar>> = {},
): string {
  return renderToStaticMarkup(
    createElement(WorkSidebar, { ...BASE_PROPS, ...overrides }),
  );
}

describe("WorkSidebar", () => {
  it("shows workspace context once and uses the durable run title", () => {
    const markup = renderSidebar();

    expect(markup).toContain('id="spaces-heading">Spaces</span>');
    expect(markup).toContain("Colossus");
    expect(markup).toContain("Improve the Work sidebar");
    expect(markup).toContain('aria-label="New thread in Colossus"');
    expect(markup).not.toContain('class="button primary new-work"');
    expect(markup).toContain("Capabilities");
    expect(markup).toContain("Connections");
    expect(markup).toContain('aria-label="Resize Space sidebar"');
    expect(markup).toContain('aria-valuemin="260"');
    expect(markup).toContain('aria-valuemax="480"');
    expect(markup).not.toContain("<strong>Primary</strong>");
  });

  it("keeps catalog destinations stable when the selected Space reports no entries", () => {
    const markup = renderSidebar({
      capabilities: {
        ...CAPABILITIES,
        delegation: false,
        skills: false,
        artifacts: false,
        agentWorkflows: false,
      },
    });

    expect(markup).toContain("Capabilities");
    expect(markup).toContain("Library");
  });

  it("shows Space startup without disabling search or navigation", () => {
    const nextSpace: SpaceSummary = {
      ...SPACE,
      spaceId: "space-next",
      targetId: "space-next",
      displayName: "Next Workspace",
      displayPath: "~/tools/next",
      selected: false,
      state: "sleeping",
    };
    const markup = renderSidebar({
      spaces: [SPACE, nextSpace],
      disabled: false,
      spaceStartup: {
        spaceId: nextSpace.spaceId,
        displayName: nextSpace.displayName,
      },
    });

    expect(markup).toContain("Next Workspace");
    expect(markup).toContain('class="space-shelf is-active is-starting"');
    expect(markup).toContain(">Starting</span>");
    expect(markup).toContain(
      "Starting the local agent for Next Workspace. Search and navigation remain available.",
    );
    expect(markup).toContain('aria-busy="true"');
    expect(openingButtonTag(markup, "New thread in Next Workspace")).toContain(
      "disabled",
    );
    expect(openingInputTag(markup, "Search threads")).not.toContain("disabled");
    expect(openingButtonTag(markup, "Capabilities")).not.toContain("disabled");
  });

  it("keeps Space activation separate from Finder-style thread disclosure", () => {
    const researchSpace: SpaceSummary = {
      ...SPACE,
      spaceId: "space-research",
      targetId: "space-research",
      displayName: "Research Lab",
      displayPath: "~/tools/research",
      selected: false,
      state: "sleeping",
      attentionCount: 2,
    };
    const preview: SpaceSearchResult = {
      spaceId: researchSpace.spaceId,
      spaceName: researchSpace.displayName,
      targetId: researchSpace.targetId,
      runId: "run-source-review",
      sessionId: "session-source-review",
      title: "Review source provenance",
      mode: "research",
      status: "waiting",
      updatedAt: "2026-07-24T18:02:00Z",
      archived: false,
      threadArchived: false,
      attention: true,
    };
    const markup = renderSidebar({
      spaces: [SPACE, researchSpace],
      spaceThreadPreviews: new Map([[researchSpace.spaceId, [preview]]]),
    });

    expect(openingButtonTag(markup, "Research Lab")).toContain(
      'title="~/tools/research"',
    );
    expect(markup).toContain('aria-label="Expand Research Lab threads"');
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).not.toContain('aria-label="Switch to Research Lab"');
    expect(markup).toContain("Review source provenance");
    expect(markup).toContain(
      'aria-label="Open Review source provenance in Research Lab"',
    );
  });

  it("groups attention ahead of active and recent threads", () => {
    const markup = renderSidebar({
      runs: [
        { ...RUN, runId: "waiting", sessionId: "waiting", status: "waiting" },
        RUN,
        {
          ...RUN,
          runId: "completed",
          sessionId: "completed",
          status: "completed",
        },
      ],
    });

    expect(markup.indexOf("Needs attention")).toBeLessThan(
      markup.indexOf(">Active<"),
    );
    expect(markup.indexOf(">Active<")).toBeLessThan(markup.indexOf(">Recent<"));
  });

  it("offers persistent pin controls and groups pinned threads first", () => {
    const markup = renderSidebar({
      pinnedSessionIds: new Set([RUN.sessionId]),
      runs: [
        RUN,
        {
          ...RUN,
          runId: "waiting",
          sessionId: "waiting",
          title: "Needs attention",
          status: "waiting",
        },
      ],
    });

    expect(markup.indexOf(">Pinned<")).toBeLessThan(
      markup.indexOf("Needs attention"),
    );
    expect(markup).toContain('aria-label="Unpin Improve the Work sidebar"');
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).toContain('aria-label="Pin Needs attention"');
    expect(markup).toContain(
      'aria-label="Thread actions for Improve the Work sidebar"',
    );
  });

  it("offers archiving only after a thread reaches a terminal state", () => {
    const running = renderSidebar();
    expect(
      openingButtonTag(running, "Archive Improve the Work sidebar"),
    ).toContain("disabled");

    const completed = renderSidebar({
      runs: [
        {
          ...RUN,
          status: "completed",
          finishedAt: RUN.updatedAt,
        },
      ],
    });
    expect(
      openingButtonTag(completed, "Archive Improve the Work sidebar"),
    ).not.toContain("disabled");
  });

  it("shows archived threads in global search with an explicit restore action", () => {
    const markup = renderSidebar({
      query: "sidebar",
      searchScope: "all",
      includeArchived: true,
      searchResults: [
        {
          spaceId: SPACE.spaceId,
          spaceName: SPACE.displayName,
          targetId: SPACE.targetId,
          runId: RUN.runId,
          sessionId: RUN.sessionId,
          title: RUN.title,
          mode: RUN.mode,
          status: "completed",
          updatedAt: RUN.updatedAt,
          archived: false,
          threadArchived: true,
          attention: false,
        },
      ],
    });

    expect(markup).toContain("Include archived Spaces and threads");
    expect(markup).toContain("· Archived");
    expect(markup).toContain('aria-label="Restore Improve the Work sidebar"');
  });

  it("shows cross-Space search metadata and archived management without raw paths", () => {
    const archived: SpaceSummary = {
      ...SPACE,
      spaceId: "space-archive",
      targetId: "space-archive",
      displayName: "Proposal Archive",
      displayPath: "/private/sensitive/proposals",
      archived: true,
      selected: false,
      state: "archived",
    };
    const markup = renderSidebar({
      spaces: [SPACE, archived],
      query: "proposal",
      searchScope: "all",
      includeArchived: true,
      searchResults: [
        {
          spaceId: archived.spaceId,
          spaceName: archived.displayName,
          targetId: archived.targetId,
          runId: "run-proposal",
          sessionId: "session-proposal",
          title: "Review proposal package",
          mode: "plan",
          status: "waiting",
          updatedAt: "2026-07-24T18:02:00Z",
          archived: true,
          threadArchived: false,
          attention: true,
        },
      ],
    });

    expect(markup).toContain("All Spaces");
    expect(markup).toContain('aria-label="Search scope: All Spaces"');
    expect(openingInputTag(markup, "Search threads")).not.toContain("disabled");
    expect(markup).not.toContain('class="search-scope"');
    expect(markup).toContain("Include archived");
    expect(markup).toContain("Review proposal package");
    expect(markup).toContain("Proposal Archive");
    expect(markup).toContain("Restore");
    expect(markup).not.toContain("/private/sensitive/proposals");
  });
});

function openingButtonTag(markup: string, label: string): string {
  const labelIndex = markup.indexOf(label);
  expect(labelIndex).toBeGreaterThan(-1);
  const buttonIndex = markup.lastIndexOf("<button", labelIndex);
  expect(buttonIndex).toBeGreaterThan(-1);
  return markup.slice(buttonIndex, markup.indexOf(">", buttonIndex) + 1);
}

function openingInputTag(markup: string, placeholder: string): string {
  const placeholderIndex = markup.indexOf(`placeholder="${placeholder}"`);
  expect(placeholderIndex).toBeGreaterThan(-1);
  const inputIndex = markup.lastIndexOf("<input", placeholderIndex);
  expect(inputIndex).toBeGreaterThan(-1);
  return markup.slice(inputIndex, markup.indexOf(">", inputIndex) + 1);
}
