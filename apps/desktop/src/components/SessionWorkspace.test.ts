import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { buildSessionMapFixture } from "../dev/operations-studio-fixture";
import type { RunView } from "../state";
import type { Run } from "../types";
import type { AgentParticipant } from "./AgentFlow";
import {
  SessionPlansView,
  SessionTopology,
  SessionWorkspaceTabs,
} from "./SessionWorkspace";

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
    selectedSkills: [],
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
    expect(markup).toContain("Sources");
    expect(markup).toContain("Resources");
  });

  it("renders the lazy interactive graph shell and session map controls", () => {
    const first = runView("run-one", "Initial review", 0);
    const second = runView("run-two", "Follow-up review", 1);
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
        sessionMap: buildSessionMapFixture(),
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
    expect(markup.match(/role="switch"/g)).toHaveLength(6);
    expect(markup).toContain("Show run lineage");
  });

  it("makes plan titles readable actions and renders a compact Markdown preview", () => {
    const plan = runView("run-plan", "make me a simple plan", 2);
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
        workflowAvailable: true,
        onInspectPlan: vi.fn(),
        onOpenPlanWorkflow: vi.fn(),
        onRevisePlan: vi.fn(),
      }),
    );

    expect(markup).toContain(
      '<button class="session-plan-title" type="button">make me a simple plan</button>',
    );
    expect(markup).toContain('class="markdown-content session-plan-preview"');
    expect(markup).toContain("<h4>Simple repository orientation plan</h4>");
    expect(markup).toContain("<strong>Review project foundations</strong>");
    expect(markup).not.toContain("### Simple repository orientation plan");
  });
});
