import { describe, expect, it } from "vitest";

import {
  selectAgentParticipants,
  selectSessionParticipants,
} from "./participants";
import type { RunView } from "./state";
import type { Run, RunUpdate } from "./types";

const RUN: Run = {
  runId: "run-parent",
  sessionId: "session-parent",
  title: "Coordinate specialist review",
  role: "primary",
  mode: "execute",
  status: "completed",
  createdAt: "2026-08-16T12:00:00Z",
  updatedAt: "2026-08-16T12:00:14Z",
  startedAt: "2026-08-16T12:00:00Z",
  finishedAt: "2026-08-16T12:00:14Z",
  lastSequence: 0,
  pendingInteractionCount: 0,
  terminal: null,
  etag: "etag-parent",
  selectedSkills: [],
  archived: false,
};

function toolUpdate(
  sequence: number,
  toolName: "agent.delegate" | "agent.result",
  preview: unknown,
): RunUpdate {
  return {
    runId: RUN.runId,
    sequence,
    createdAt: `2026-08-16T12:00:${String(sequence).padStart(2, "0")}Z`,
    update: {
      type: "tool_activity",
      activity: {
        callId: `call-${sequence}`,
        toolName,
        state: "completed",
        summary: "tool execution completed",
        preview: JSON.stringify(preview),
      },
    },
  };
}

function viewFixture(updates: RunUpdate[]): RunView {
  return {
    run: { ...RUN, lastSequence: updates.at(-1)?.sequence ?? 0 },
    localPrompt: null,
    output: "",
    updates,
    seenSequences: new Set(updates.map(({ sequence }) => sequence)),
    lastSequence: updates.at(-1)?.sequence ?? 0,
    pendingInteractions: [],
    usage: null,
    streamState: "complete",
    streamError: null,
  };
}

describe("selectAgentParticipants", () => {
  it("shows a delegated agent and applies its latest released status", () => {
    const job = {
      id: "agent-018f",
      parent_run_id: RUN.runId,
      role: "subagent_default",
      task: "Review the permission boundary and report findings",
      child_session_id: "session-child",
      child_run_id: "run-child",
      final_output: "The permission boundary is correctly scoped.",
      error: "",
      created_at: "2026-08-16T12:00:00Z",
      updated_at: "2026-08-16T12:00:14Z",
      started_at: "2026-08-16T12:00:01Z",
      completed_at: "2026-08-16T12:00:14Z",
    };
    const participants = selectAgentParticipants(
      viewFixture([
        toolUpdate(1, "agent.delegate", { ...job, status: "queued" }),
        toolUpdate(2, "agent.result", { ...job, status: "completed" }),
      ]),
    );

    expect(participants).toEqual([
      expect.objectContaining({
        id: RUN.runId,
        name: "Primary",
        role: "Primary run",
        state: "completed",
      }),
      expect.objectContaining({
        id: job.id,
        name: "Delegated agent",
        role: job.task,
        state: "completed",
        childSessionId: job.child_session_id,
        childRunId: job.child_run_id,
        finalOutput: job.final_output,
        startedAt: job.started_at,
        completedAt: job.completed_at,
      }),
    ]);
  });

  it("ignores malformed and cross-run subagent previews", () => {
    const valid = toolUpdate(3, "agent.delegate", {
      id: "agent-valid",
      parent_run_id: RUN.runId,
      role: "security_reviewer",
      status: "running",
    });
    const malformed = toolUpdate(1, "agent.delegate", {
      id: "agent-malformed",
      status: "unknown",
    });
    const crossRun = toolUpdate(2, "agent.result", {
      id: "agent-cross-run",
      parent_run_id: "run-other",
      role: "subagent_default",
      status: "completed",
    });

    expect(
      selectAgentParticipants(viewFixture([malformed, crossRun, valid])).slice(
        1,
      ),
    ).toEqual([
      expect.objectContaining({
        id: "agent-valid",
        role: "Security reviewer",
        state: "working",
      }),
    ]);
  });

  it("returns no participants without a selected run", () => {
    expect(selectAgentParticipants(undefined)).toEqual([]);
  });

  it("retains delegates from every run in the selected session", () => {
    const first = viewFixture([
      toolUpdate(1, "agent.delegate", {
        id: "agent-first",
        parent_run_id: RUN.runId,
        role: "security_reviewer",
        task: "Review the security boundary",
        status: "completed",
      }),
    ]);
    const secondRun = {
      ...RUN,
      runId: "run-follow-up",
      title: "Follow up on the review",
      createdAt: "2026-08-16T12:01:00Z",
      updatedAt: "2026-08-16T12:01:10Z",
    };
    const secondUpdate = {
      ...toolUpdate(2, "agent.result", {
        id: "agent-second",
        parent_run_id: secondRun.runId,
        role: "subagent_default",
        task: "Check the follow-up implementation",
        status: "completed",
      }),
      runId: secondRun.runId,
    };
    const second = {
      ...viewFixture([secondUpdate]),
      run: secondRun,
    };

    expect(selectSessionParticipants([first, second])).toEqual([
      expect.objectContaining({
        id: RUN.sessionId,
        role: "Primary session",
      }),
      expect.objectContaining({
        id: "agent-first",
        parentRunId: RUN.runId,
        parentRunIndex: 1,
        parentRunTitle: RUN.title,
      }),
      expect.objectContaining({
        id: "agent-second",
        parentRunId: secondRun.runId,
        parentRunIndex: 2,
        parentRunTitle: secondRun.title,
      }),
    ]);
  });
});
