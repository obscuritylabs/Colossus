import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { buildSessionMapFixture } from "../dev/operations-studio-fixture";
import type { RunView } from "../state";
import type { Run } from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { SessionTopology, SessionWorkspaceTabs } from "./SessionWorkspace";

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

  it("renders released session resources as expandable graph families", () => {
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
    expect(markup).toContain("Delegated agents <b>3</b>");
    expect(markup).toContain("Goals <b>2</b>");
    expect(markup).toContain("Key decisions <b>2</b>");
    expect(markup).toContain("Memories <b>3</b>");
    expect(markup).toContain(
      "Use Rust 1.96 and edition 2024 for implementation work.",
    );
    expect(markup).toContain("Show run lineage");
  });
});
