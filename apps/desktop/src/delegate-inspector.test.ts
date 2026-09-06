import { describe, expect, it } from "vitest";

import {
  selectDelegateActivities,
  selectInspectedDelegateActivities,
} from "./delegate-inspector";
import type { RunView } from "./state";

function viewFixture(): RunView {
  return {
    run: {
      runId: "run-child",
      sessionId: "session-child",
      title: "Review security boundary",
      role: "security_reviewer",
      mode: "execute",
      status: "completed",
      createdAt: "2026-08-16T14:00:00Z",
      updatedAt: "2026-08-16T14:00:05Z",
      startedAt: "2026-08-16T14:00:00Z",
      finishedAt: "2026-08-16T14:00:05Z",
      lastSequence: 3,
      pendingInteractionCount: 0,
      terminal: null,
      etag: "etag-child",
      archived: false,
    },
    localPrompt: null,
    output: "No cross-space path found.",
    updates: [
      {
        runId: "run-child",
        sequence: 1,
        createdAt: "2026-08-16T14:00:01Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-read",
            toolName: "filesystem.read",
            state: "started",
            summary: "tool execution started",
          },
        },
      },
      {
        runId: "run-child",
        sequence: 2,
        createdAt: "2026-08-16T14:00:02.100Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-read",
            toolName: "filesystem.read",
            state: "completed",
            summary: "tool execution completed",
          },
        },
      },
      {
        runId: "run-child",
        sequence: 3,
        createdAt: "2026-08-16T14:00:03Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-search",
            toolName: "repo.search",
            state: "completed",
            summary: "tool execution completed",
          },
        },
      },
    ],
    seenSequences: new Set([1, 2, 3]),
    lastSequence: 3,
    pendingInteractions: [],
    usage: null,
    streamState: "complete",
    streamError: null,
  };
}

describe("selectDelegateActivities", () => {
  it("collapses tool lifecycles into released, readable actions", () => {
    expect(selectDelegateActivities(viewFixture())).toEqual([
      expect.objectContaining({
        callId: "call-read",
        title: "Read workspace files",
        toolName: "filesystem.read",
        state: "completed",
        durationLabel: "1.1s",
      }),
      expect.objectContaining({
        callId: "call-search",
        title: "Searched repository",
        state: "completed",
      }),
    ]);
  });

  it("presents bounded native child activity without a public Run", () => {
    expect(
      selectInspectedDelegateActivities({
        jobId: "agent-child",
        parentRunId: "run-parent",
        childSessionId: "session-child",
        childRunId: "run-child",
        task: "Review the boundary",
        role: "subagent_default",
        status: "completed",
        finalOutput: "Done",
        error: "",
        createdAt: "2026-08-16T14:00:00Z",
        updatedAt: "2026-08-16T14:00:03Z",
        startedAt: "2026-08-16T14:00:00Z",
        completedAt: "2026-08-16T14:00:03Z",
        activities: [
          {
            callId: "call-shell",
            toolName: "shell.run",
            state: "completed",
            summary: "tool execution completed",
            input: '{"command":"pwd"}',
            preview: "/workspace",
            startedAt: "2026-08-16T14:00:01Z",
            completedAt: "2026-08-16T14:00:02.250Z",
          },
        ],
      }),
    ).toEqual([
      expect.objectContaining({
        callId: "call-shell",
        title: "Ran a shell command",
        durationLabel: "1.3s",
        input: '{"command":"pwd"}',
        preview: "/workspace",
      }),
    ]);
  });
});
