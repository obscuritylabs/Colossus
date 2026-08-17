import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { RunView } from "../state";
import type { AgentParticipant } from "./AgentFlow";
import { ThreadDetailsPanel } from "./ThreadDetailsPanel";

const participant: AgentParticipant = {
  id: "delegate-security",
  name: "Delegated agent 2",
  role: "Conduct a read-only security review",
  state: "completed",
  icon: "security",
  kind: "delegate",
  parentRunId: "run-parent",
  childSessionId: "session-child",
  childRunId: "run-child",
  modelRole: "security_reviewer",
  task: "Conduct a read-only security review of process and session boundaries.",
};

const view: RunView = {
  run: {
    runId: "run-child",
    sessionId: "session-child",
    title: participant.task ?? "Security review",
    role: "security_reviewer",
    mode: "execute",
    status: "completed",
    createdAt: "2026-08-16T14:00:00Z",
    updatedAt: "2026-08-16T14:00:14Z",
    startedAt: "2026-08-16T14:00:00Z",
    finishedAt: "2026-08-16T14:00:14Z",
    lastSequence: 1,
    pendingInteractionCount: 0,
    terminal: {
      type: "result",
      result: {
        output: "Session ownership remains bound to the selected Space.",
        profile: "desktop",
        modelProfile: "delegated",
        providerProfile: "provider",
        model: "model",
        elapsedSeconds: 14,
      },
    },
    etag: "etag-child",
    selectedSkills: [],
    archived: false,
  },
  localPrompt: null,
  output: "Session ownership remains bound to the selected Space.",
  updates: [
    {
      runId: "run-child",
      sequence: 1,
      createdAt: "2026-08-16T14:00:02Z",
      update: {
        type: "tool_activity",
        activity: {
          callId: "call-search",
          toolName: "repo.search",
          state: "completed",
          summary: "tool execution completed",
          preview: "Matched the selected-Space guard.",
        },
      },
    },
  ],
  seenSequences: new Set([1]),
  lastSequence: 1,
  pendingInteractions: [],
  usage: null,
  streamState: "complete",
  streamError: null,
};

describe("ThreadDetailsPanel", () => {
  it("shows a selected delegate as a read-only released-run inspector", () => {
    const markup = renderToStaticMarkup(
      createElement(ThreadDetailsPanel, {
        run: view.run,
        spaceName: "Colossus",
        pinned: false,
        participants: [participant],
        files: [],
        selectedParticipantId: participant.id,
        delegateView: view,
        delegateInspection: null,
        delegateLoading: false,
        delegateError: "",
        sessionRunCount: 2,
        sessionPlanCount: 1,
        sessionSourceCount: 3,
        onSelectParticipant: vi.fn(),
        onBackToThread: vi.fn(),
        onOpenSessionView: vi.fn(),
      }),
    );

    expect(markup).toContain("Delegated agent 2");
    expect(markup).toContain("Read-only run");
    expect(markup).toContain("Released activity");
    expect(markup).toContain("Searched repository");
    expect(markup).toContain("View final response");
  });

  it("uses the released delegate record when the child id is not a public run", () => {
    const releasedParticipant: AgentParticipant = {
      ...participant,
      finalOutput: "Hi!",
      startedAt: "2026-08-16T14:00:00Z",
      completedAt: "2026-08-16T14:00:14Z",
    };
    const markup = renderToStaticMarkup(
      createElement(ThreadDetailsPanel, {
        run: view.run,
        spaceName: "Colossus",
        pinned: false,
        participants: [releasedParticipant],
        files: [],
        selectedParticipantId: releasedParticipant.id,
        delegateView: undefined,
        delegateInspection: null,
        delegateLoading: false,
        delegateError: "",
        sessionRunCount: 2,
        sessionPlanCount: 1,
        sessionSourceCount: 3,
        onSelectParticipant: vi.fn(),
        onBackToThread: vi.fn(),
        onOpenSessionView: vi.fn(),
      }),
    );

    expect(markup).toContain("Completed");
    expect(markup).toContain("14s");
    expect(markup).toContain("Detailed child actions were not released");
    expect(markup).toContain("Hi!");
    expect(markup).not.toContain("requested run was not found");
    expect(markup).not.toContain("has not released a final response yet");
  });
});
