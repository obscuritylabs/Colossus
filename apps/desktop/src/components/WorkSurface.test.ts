import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { RunView } from "../state";
import type { Run } from "../types";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { WorkSurface } from "./WorkSurface";

function renderSurface(
  artifacts: readonly ArtifactViewItem[],
  capabilities = { files: true, artifacts: true },
  activityComparisonEnabled = false,
  withRun = activityComparisonEnabled,
  runMode: Run["mode"] = "execute",
): string {
  const comparisonRun: Run = {
    runId: "comparison-run",
    sessionId: "comparison-session",
    title: "Compare activity layouts",
    role: "primary",
    mode: runMode,
    status: "completed",
    createdAt: "2026-08-15T12:00:00Z",
    updatedAt: "2026-08-15T12:00:01Z",
    startedAt: "2026-08-15T12:00:00Z",
    finishedAt: "2026-08-15T12:00:01Z",
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal:
      runMode === "research"
        ? {
            type: "result",
            result: {
              output: "",
              profile: "research",
              modelProfile: "research",
              providerProfile: "research",
              model: "research",
              elapsedSeconds: 1,
            },
          }
        : null,
    etag: "comparison-etag",
    selectedSkills: [],
    archived: false,
  };
  const comparisonView: RunView = {
    run: comparisonRun,
    localPrompt: null,
    output:
      runMode === "research"
        ? "# Report\n\n## Sources\n\n- [R1] Runtime docs — repo://docs/runtime.md"
        : "",
    updates: [],
    seenSequences: new Set(),
    lastSequence: 0,
    pendingInteractions: [],
    usage: null,
    streamState: "complete",
    streamError: null,
  };
  const renderedView = withRun ? comparisonView : undefined;
  vi.stubGlobal("window", {
    matchMedia: () => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
  try {
    return renderToStaticMarkup(
      createElement(WorkSurface, {
        title: "Primary",
        view: renderedView,
        conversationViews: renderedView === undefined ? [] : [renderedView],
        connection: {
          state: "connected",
          message: "Connected securely.",
          targetId: "managed-local",
        },
        connecting: false,
        cancelling: false,
        runLoadError: "",
        actionError: null,
        participants: [],
        selectedParticipantId: null,
        delegateView: undefined,
        delegateInspection: null,
        delegateLoading: false,
        delegateError: "",
        sessionMap: null,
        sessionMapLoading: false,
        sessionMapError: "",
        artifacts,
        selectedSpaceName: "Colossus",
        threadPinned: true,
        followRequestSequence: 0,
        composer: createElement("div"),
        filesPanel: createElement("div", null, "Workspace file explorer"),
        filesAvailable: capabilities.files,
        onOpenWorkspaceFile: vi.fn(),
        artifactsAvailable: capabilities.artifacts,
        asideView: undefined,
        asideConversationViews: [],
        asideHistory: [],
        asideBusy: false,
        asideError: null,
        asideReadOnly: false,
        planContinuationAvailable: false,
        planWorkflowAvailable: false,
        activityComparisonEnabled,
        workNavigationOpen: false,
        onConnect: vi.fn(),
        onCancel: vi.fn(),
        onRespond: vi.fn(async () => undefined),
        onResume: vi.fn(),
        onSuggestion: vi.fn(),
        onSelectParticipant: vi.fn(),
        onBackToThreadDetails: vi.fn(),
        onSelectArtifact: vi.fn(),
        onOpenPlanWorkflow: vi.fn(),
        onRevisePlan: vi.fn(),
        onExecutePlan: vi.fn(),
        onOpenWorkNavigation: vi.fn(),
        onCloseWorkNavigation: vi.fn(),
        onLoadAsides: vi.fn(async () => undefined),
        onCreateAside: vi.fn(async () => true),
        onContinueAside: vi.fn(async () => true),
        onOpenAside: vi.fn(async () => undefined),
        onNewAside: vi.fn(),
        onRespondAside: vi.fn(async () => undefined),
        onCloseAside: vi.fn(async () => true),
      }),
    );
  } finally {
    vi.unstubAllGlobals();
  }
}

describe("WorkSurface side panels", () => {
  it("offers released Research citations in the resizable side panel", () => {
    const markup = renderSurface([], undefined, false, true, "research");

    expect(markup).toContain('aria-label="Open Research sources"');
    expect(markup).toContain('aria-label="Research sources"');
    expect(markup).toContain("Runtime docs");
    expect(markup).toContain("Research mode");
  });

  it("keeps new work in the flexible conversation row when agent flow is absent", () => {
    const markup = renderSurface([]);

    expect(markup).toContain('<main class="work-surface is-new-work"');
    expect(markup).toContain('<section class="work-welcome">');
    expect(markup).toContain("Orient yourself in this repo");
    expect(markup).not.toContain(
      "Review this workspace and identify the safest high-impact next task",
    );
    expect(markup).not.toContain('class="agent-flow"');
  });

  it("keeps an empty artifact panel collapsed behind a count-bearing toggle", () => {
    const markup = renderSurface([]);

    expect(markup).toContain('aria-controls="work-side-drawer"');
    expect(markup.match(/aria-controls="work-side-drawer"/g)).toHaveLength(2);
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain("Open files panel");
    expect(markup).toContain("Open artifacts panel, 0 artifacts");
    expect(markup).toContain('<span class="artifact-count"');
    expect(markup).toContain(">0</span>");
    expect(markup).toContain('<div class="work-layout">');
    expect(markup).not.toContain("is-work-drawer-open");
  });

  it("shows the released artifact count without forcing the panel open", () => {
    const markup = renderSurface([
      {
        id: "artifact-1",
        fileName: "report.md",
        mediaType: "text/markdown",
        sizeLabel: "1 KB",
        stateLabel: "Available",
        createdLabel: "Recent",
      },
    ]);

    expect(markup).toContain("Open artifacts panel, 1 artifact");
    expect(markup).toContain(">1</span>");
    expect(markup).not.toContain("is-work-drawer-open");
  });

  it("does not imply drawers the runtime has not advertised", () => {
    const markup = renderSurface([], { files: false, artifacts: false });

    expect(markup).not.toContain("Open files panel");
    expect(markup).not.toContain("Open artifacts panel");
    expect(markup).not.toContain('id="work-side-drawer"');
  });

  it("renders live activity directly in the working timeline", () => {
    const liveThreadMarkup = renderSurface(
      [],
      { files: true, artifacts: true },
      false,
      true,
    );
    expect(liveThreadMarkup).toContain('aria-label="Session views"');
    expect(liveThreadMarkup).toContain("Topology");
    expect(liveThreadMarkup).toContain("Plans");
    expect(liveThreadMarkup).toContain("Sources");
    expect(liveThreadMarkup).toContain("Resources");
    expect(liveThreadMarkup).not.toContain('aria-label="Timeline view"');
    expect(liveThreadMarkup).not.toContain("Capsule");
    expect(liveThreadMarkup).not.toContain("Working thread");
    expect(liveThreadMarkup).toContain(
      'data-aside-source-run-id="comparison-run"',
    );
    expect(liveThreadMarkup).not.toContain("data-aside-message-count");
  });

  it("offers grounded thread details without inventing file paths", () => {
    const markup = renderSurface(
      [
        {
          id: "bootstrap",
          fileName: "bootstrap.rs",
          mediaType: "text/x-rust",
          sizeLabel: "18 KB",
          stateLabel: "Available",
          createdLabel: "Jul 20, 10:30 AM",
        },
      ],
      { files: true, artifacts: true },
      false,
      true,
    );

    expect(markup).toContain('aria-label="Open thread details"');
    expect(markup).toContain('id="thread-details-title"');
    expect(markup).toContain("Colossus");
    expect(markup).toContain("bootstrap.rs");
    expect(markup).not.toContain("src/bootstrap.rs");
  });
});
