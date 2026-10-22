import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { buildSessionMapFixture } from "../dev/operations-studio-fixture";
import type { RunView } from "../state";
import type { Run } from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { SessionMapDetailsPanel } from "./SessionMapDetailsPanel";
import { SessionSourcesView as SharedSessionSourcesView } from "@colossus/ui/session/SessionWorkspace";
import { SessionPresentationProvider } from "@colossus/ui/session/links";
import {
  SessionPlansView,
  SessionResourcesView,
  SessionSnapshotsView,
  SessionTopology,
  SessionWorkspaceTabs,
} from "./SessionWorkspace";

function classTokens(markup: string): string[][] {
  return [...markup.matchAll(/class="([^"]*)"/g)].map((match) =>
    match[1]!.split(/\s+/),
  );
}

function runView(runId: string, title: string, minute: number): RunView {
  const run: Run = {
    runId,
    sessionId: "session-one",
    title,
    role: "primary",
    mode: "execute",
    status: "completed",
    createdAt: `2026-08-16T12:${String(minute).padStart(2, "0")}:00Z`,
    updatedAt: `2026-08-16T12:${String(minute).padStart(2, "0")}:10Z`,
    startedAt: `2026-08-16T12:${String(minute).padStart(2, "0")}:00Z`,
    finishedAt: `2026-08-16T12:${String(minute).padStart(2, "0")}:10Z`,
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal: null,
    etag: `etag-${runId}`,
    archived: false,
  };
  return {
    run,
    localPrompt: null,
    output: "",
    updates: [],
    seenSequences: new Set(),
    lastSequence: 0,
    pendingInteractions: [],
    usage: null,
    streamState: "complete",
    streamError: null,
  };
}

describe("SessionWorkspace", () => {
  it("exposes the persistent session records as peer views", () => {
    const markup = renderToStaticMarkup(
      createElement(SessionWorkspaceTabs, {
        active: "topology",
        onChange: vi.fn(),
      }),
    );

    expect(markup).toContain('aria-label="Session views"');
    expect(markup).toContain('aria-current="page">Topology');
    expect(markup).toContain("Conversation");
    expect(markup).toContain("Plans");
    expect(markup).toContain("Snapshots");
    expect(markup).toContain("Activity");
    expect(markup).toContain("Sources");
    expect(markup).toContain("Resources");
  });

  it("renders the lazy interactive graph shell and session map controls", () => {
    const first = runView("run-one", "Initial review", 0);
    const second = runView("run-two", "Follow-up review", 1);
    const sessionMap = buildSessionMapFixture();
    first.run.sessionId = sessionMap.sessionId;
    second.run.sessionId = sessionMap.sessionId;
    const participants: AgentParticipant[] = [
      {
        id: "session-one",
        name: "Primary",
        role: "Primary session",
        state: "completed",
        icon: "lead",
        kind: "primary",
      },
      {
        id: "delegate-one",
        name: "Security reviewer",
        role: "Review the security boundary",
        state: "completed",
        icon: "security",
        kind: "delegate",
        parentRunId: first.run.runId,
      },
      {
        id: "delegate-two",
        name: "Implementation reviewer",
        role: "Review the follow-up",
        state: "working",
        icon: "builder",
        kind: "delegate",
        parentRunId: second.run.runId,
      },
    ];

    const markup = renderToStaticMarkup(
      createElement(SessionTopology, {
        views: [first, second],
        participants,
        sessionMap,
        loading: false,
        error: "",
        artifacts: [],
        onSelectResource: vi.fn(),
        onSelectArtifact: vi.fn(),
      }),
    );

    expect(markup).toContain("Session map");
    expect(markup).toContain(
      "Primary session · canonical resources and agent lineage",
    );
    expect(markup).toContain("Loading interactive session map…");
    expect(markup).toContain("Collapse all");
    expect(markup).toContain("Layers");
    expect(markup).toContain("Show parent run IDs");
    expect(markup).not.toContain("Entire session");
  });

  it.each([false, true])(
    "keeps unavailable topology free of graph controls while loading=%s",
    (loading) => {
      const markup = renderToStaticMarkup(
        createElement(SessionTopology, {
          views: [],
          participants: [],
          sessionMap: null,
          loading,
          error: "This connection has not released its session map.",
          artifacts: [],
          onSelectResource: vi.fn(),
          onSelectArtifact: vi.fn(),
        }),
      );
      expect(markup).toContain(
        loading ? "Loading session resources" : "Session map unavailable",
      );
      for (const control of [
        "Collapse all",
        "Layers",
        "Show parent run IDs",
        "Entire session",
      ]) {
        expect(markup).not.toContain(control);
      }
      expect(markup).not.toContain("Loading interactive session map");
      expect(markup).not.toMatch(/<button\b/);
    },
  );

  it("shows loading instead of controls for a retained map of the selected session", () => {
    const sessionMap = buildSessionMapFixture();
    const opening = runView("loading-run", "Selected session", 0);
    opening.run.sessionId = sessionMap.sessionId;
    const markup = renderToStaticMarkup(
      createElement(SessionTopology, {
        views: [opening],
        participants: [
          {
            id: sessionMap.sessionId,
            name: "Primary",
            role: "primary",
            state: "working",
            kind: "primary",
          },
        ],
        sessionMap,
        loading: true,
        error: "",
        artifacts: [],
        onSelectResource: vi.fn(),
        onSelectArtifact: vi.fn(),
      }),
    );
    expect(markup).toContain("Loading session resources");
    expect(markup).not.toContain("Loading interactive session map");
    expect(markup).not.toMatch(/<button\b/);
  });

  it.each([false, true])(
    "does not expose retained topology from a previous session while loading=%s",
    (loading) => {
      const opening = runView("new-run", "New session", 0);
      opening.run.sessionId = "new-session";
      const markup = renderToStaticMarkup(
        createElement(SessionTopology, {
          views: [opening],
          participants: [
            {
              id: "new-session",
              name: "New primary",
              role: "primary",
              state: "working",
              kind: "primary",
            },
          ],
          sessionMap: buildSessionMapFixture(),
          loading,
          error: "",
          artifacts: [],
          onSelectResource: vi.fn(),
          onSelectArtifact: vi.fn(),
        }),
      );
      expect(markup).toContain(
        loading ? "Loading session resources" : "Session map unavailable",
      );
      expect(markup).not.toContain("Loading interactive session map");
      expect(markup).not.toContain("Show parent run IDs");
      expect(markup).not.toMatch(/<button\b/);
    },
  );

  it("makes plan titles readable actions and renders a compact Markdown preview", () => {
    const plan = runView("run-plan", "make me a simple plan", 2);
    plan.run.mode = "plan";
    plan.run.terminal = {
      type: "result",
      result: {
        output: "Plan saved.",
        planId: "plan-simple",
        planRevision: 1,
        planStatus: "draft",
        profile: "desktop",
        modelProfile: "primary",
        providerProfile: "primary-provider",
        model: "fixture",
        elapsedSeconds: 2,
      },
    };
    plan.output = `### Simple repository orientation plan

1. **Review project foundations**
2. Map the application`;

    const markup = renderToStaticMarkup(
      createElement(SessionPlansView, {
        views: [plan],
        continuationAvailable: true,
        workflowAvailable: true,
        onInspectPlan: vi.fn(),
        onOpenPlanWorkflow: vi.fn(),
        onRevisePlan: vi.fn(),
      }),
    );

    expect(markup).toMatch(
      /<button\b[^>]*class="[^"]*\bsession-plan-title\b[^"]*"[^>]*>make me a simple plan<\/button>/,
    );
    expect(classTokens(markup)).toContainEqual(
      expect.arrayContaining(["markdown-content", "session-plan-preview"]),
    );
    expect(markup).toContain("<h4>Simple repository orientation plan</h4>");
    expect(markup).toContain("<strong>Review project foundations</strong>");
    expect(markup).not.toContain("### Simple repository orientation plan");
  });

  it("does not label a plan as Draft without released plan status", () => {
    const plan = runView(
      "unknown-plan",
      "Saved plan with incomplete metadata",
      2,
    );
    plan.run.mode = "plan";
    plan.run.terminal = {
      type: "result",
      result: { output: "Saved.", planId: "plan-unknown", planRevision: 3 },
    } as Run["terminal"];
    const markup = renderToStaticMarkup(
      createElement(SessionPlansView, {
        views: [plan],
        continuationAvailable: true,
        workflowAvailable: false,
        onInspectPlan: vi.fn(),
        onOpenPlanWorkflow: vi.fn(),
        onRevisePlan: vi.fn(),
      }),
    );
    expect(markup).toContain("Status not reported");
    expect(markup).not.toContain(">Draft<");
    expect(markup).toMatch(/<button\b[^>]*disabled=""[^>]*>Revise<\/button>/);
  });

  it("renders a compact snapshot preview and the complete record as Markdown", () => {
    const sessionMap = buildSessionMapFixture();
    const snapshot = sessionMap.contextSnapshots[0]!;
    const markup = renderToStaticMarkup(
      createElement(SessionSnapshotsView, {
        sessionMap,
        loading: false,
        error: "",
        onSelectResource: vi.fn(),
      }),
    );

    expect(markup).toContain("Context snapshots");
    expect(markup).toContain(
      `Messages ${snapshot.sourceStartSequence}–${snapshot.sourceEndSequence}`,
    );
    expect(classTokens(markup)).toContainEqual(
      expect.arrayContaining(["markdown-content", "session-snapshot-preview"]),
    );
    expect(markup).toContain("<h4>Session context</h4>");
    expect(markup).toContain(
      "<strong>hardening Desktop runtime recovery</strong>",
    );
    expect(markup).not.toContain("## Session context");
    expect(markup).toContain("View snapshot");

    const detail = renderToStaticMarkup(
      createElement(SessionMapDetailsPanel, {
        resource: { family: "snapshots", value: snapshot },
        spaceName: "Colossus",
        onBack: vi.fn(),
      }),
    );
    expect(detail).toContain("Context snapshot");
    expect(detail).toContain(
      `<h3 id="session-map-detail-title">Messages ${snapshot.sourceStartSequence}–${snapshot.sourceEndSequence}</h3>`,
    );
    expect(classTokens(detail)).toContainEqual(
      expect.arrayContaining(["markdown-content", "session-snapshot-summary"]),
    );
    expect(detail).toContain("<h4>Session context</h4>");
    expect(detail).not.toContain("## Session context");
    expect(detail).toContain("Pinned facts");
    expect(detail).toContain("Open tasks");
    expect(detail).toContain("Files touched");
    expect(detail).toContain("Notable tool results");
    expect(detail).toContain(snapshot.pinnedFacts[0]!);
  });

  it("distinguishes an unavailable snapshot count from a known empty map", () => {
    const props = { loading: false, error: "", onSelectResource: vi.fn() };
    const unavailable = renderToStaticMarkup(
      createElement(SessionSnapshotsView, { ...props, sessionMap: null }),
    );
    expect(unavailable).toContain('aria-label="Snapshot count unavailable"');
    expect(unavailable).toContain("Context snapshots are unavailable");
    expect(unavailable).not.toContain('aria-label="0 snapshots"');
    expect(unavailable).not.toContain("No context snapshots");
    const knownEmpty = renderToStaticMarkup(
      createElement(SessionSnapshotsView, {
        ...props,
        sessionMap: { ...buildSessionMapFixture(), contextSnapshots: [] },
      }),
    );
    expect(knownEmpty).toContain('aria-label="0 snapshots"');
    expect(knownEmpty).toContain("No context snapshots");
  });

  it("retains released snapshots and resources alongside their refresh error", () => {
    const sessionMap = buildSessionMapFixture();
    const error = "The runtime could not refresh these records.";
    const snapshots = renderToStaticMarkup(
      createElement(SessionSnapshotsView, {
        sessionMap,
        error,
        loading: false,
        onSelectResource: vi.fn(),
      }),
    );
    expect(snapshots).toContain("View snapshot");
    expect(snapshots).toContain(error);
    expect(snapshots).toContain('role="status"');
    const resources = renderToStaticMarkup(
      createElement(SessionResourcesView, {
        sessionMap,
        error,
        loading: false,
        views: [],
        artifacts: [],
        onChangeView: vi.fn(),
        onSelectArtifact: vi.fn(),
        onSelectResource: vi.fn(),
      }),
    );
    expect(resources).toContain("Keep execution boundary unchanged");
    expect(resources).toContain("Showing the last loaded records.");
    expect(resources).toContain(error);
    expect(resources).not.toContain("Durable records are unavailable");
  });

  it("leaves source URLs inert by default and delegates navigation to a supplied host", () => {
    const research = runView("research", "Released research", 0);
    research.output =
      "## Sources\n\n- [R1] Official documentation — https://example.test/docs\n- [R2] Workspace note — repo://docs/runtime.md";
    const inert = renderToStaticMarkup(
      createElement(SharedSessionSourcesView, {
        views: [research],
        onOpenWorkspaceFile: vi.fn(),
      }),
    );
    expect(inert).toContain("https://example.test/docs");
    expect(inert).not.toContain("href=");
    expect(inert).toContain("Open file");
    const linked = renderToStaticMarkup(
      createElement(SessionPresentationProvider, {
        linkComponent: ({ href, children }) =>
          createElement("button", { "data-host-url": href }, children),
        children: createElement(SharedSessionSourcesView, {
          views: [research],
        }),
      }),
    );
    expect(linked).toContain('data-host-url="https://example.test/docs"');
    expect(linked).not.toContain("href=");
    expect(linked).not.toContain("Open file");
  });

  it("lists every durable session-map family in Resources", () => {
    const sessionMap = buildSessionMapFixture();
    const markup = renderToStaticMarkup(
      createElement(SessionResourcesView, {
        views: [],
        artifacts: [],
        sessionMap,
        loading: false,
        error: "",
        onChangeView: vi.fn(),
        onSelectArtifact: vi.fn(),
        onSelectResource: vi.fn(),
      }),
    );

    for (const label of [
      "Plans",
      "Sources",
      "Context snapshots",
      "Artifacts",
      "Delegated agents",
      "Goals",
      "Tasks",
      "Key decisions",
      "Memories",
      "Research",
    ]) {
      expect(markup).toContain(label);
    }
    expect(markup).not.toContain("No released decisions");
    expect(markup).toContain("Keep execution boundary unchanged");
    expect(markup).toContain("Fail closed on identity drift");
    expect(markup).toContain(
      "Active and historical decisions across this Workspace",
    );
    const detail = renderToStaticMarkup(
      createElement(SessionMapDetailsPanel, {
        resource: { family: "decisions", value: sessionMap.decisions[1]! },
        spaceName: "Colossus",
        onBack: vi.fn(),
      }),
    );
    expect(detail).toContain("Origin session");
    expect(detail).toContain("fixture-session-other");
  });
});
