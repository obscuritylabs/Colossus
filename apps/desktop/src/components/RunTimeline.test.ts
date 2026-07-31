import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { RunView, StreamState } from "../state";
import type { Run, RunStatus, RunUpdate } from "../types";
import { RunTimeline } from "./RunTimeline";

function renderOutput(
  output: string,
  status: RunStatus,
  streamState: StreamState,
  updates: RunUpdate[] = [],
  runOverrides: Partial<Run> = {},
  planContinuationAvailable = true,
): string {
  const run: Run = {
    runId: "run-markdown-test",
    sessionId: "session-markdown-test",
    title: "Render markdown output",
    role: "primary",
    mode: "execute",
    status,
    createdAt: "2026-07-21T12:00:00Z",
    updatedAt: "2026-07-21T12:00:01Z",
    startedAt: "2026-07-21T12:00:00Z",
    finishedAt: status === "completed" ? "2026-07-21T12:00:01Z" : null,
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal: null,
    etag: "etag-markdown-test",
    selectedSkills: [],
    ...runOverrides,
  };
  const view: RunView = {
    run,
    localPrompt: null,
    output,
    updates,
    seenSequences: new Set(),
    lastSequence: 0,
    pendingInteractions: [],
    usage: null,
    streamState,
    streamError: null,
  };

  return renderToStaticMarkup(
    createElement(RunTimeline, {
      view,
      planContinuationAvailable,
      planWorkflowAvailable: true,
      onOpenPlanWorkflow: vi.fn(),
      onRevisePlan: vi.fn(),
      onExecutePlan: vi.fn(async () => undefined),
    }),
  );
}

describe("RunTimeline assistant output", () => {
  it("surfaces the canonical draft and its authenticated Plan workflow handoff", () => {
    const markup = renderOutput("", "completed", "complete", [], {
      mode: "plan",
      terminal: {
        type: "result",
        result: {
          output: "Draft saved.",
          planId: "plan-1",
          planRevision: 3,
          planStatus: "draft",
          profile: "primary",
          modelProfile: "primary",
          providerProfile: "provider",
          model: "model",
          elapsedSeconds: 0.5,
        },
      },
    });

    expect(markup).toContain("Plan ready for your decision");
    expect(markup).toContain("Revision 3");
    expect(markup).toContain("Revise in chat");
    expect(markup).toContain("Run once");
    expect(markup).toContain("Run as Goal");
    expect(markup).toContain("<dd>plan-1</dd>");
    expect(markup).toContain("Advanced workflow");
  });

  it("shows completed Goal lineage without offering stale draft actions", () => {
    const markup = renderOutput("", "completed", "complete", [], {
      terminal: {
        type: "result",
        result: {
          output: "Goal started.",
          planId: "plan-1",
          planRevision: 5,
          planStatus: "executed",
          goalId: "goal-1",
          profile: "goal",
          modelProfile: "goal",
          providerProfile: "goal",
          model: "goal",
          elapsedSeconds: 1,
        },
      },
    });

    expect(markup).toContain("Plan started as a Goal");
    expect(markup).toContain("<dd>goal-1</dd>");
    expect(markup).not.toContain("Revise in chat");
    expect(markup).not.toContain("Run once");
  });

  it("keeps Plan actions unavailable when the runtime did not advertise continuation", () => {
    const markup = renderOutput(
      "",
      "completed",
      "complete",
      [],
      {
        mode: "plan",
        terminal: {
          type: "result",
          result: {
            output: "Draft saved.",
            planId: "plan-1",
            planRevision: 3,
            planStatus: "draft",
            profile: "primary",
            modelProfile: "primary",
            providerProfile: "provider",
            model: "model",
            elapsedSeconds: 0.5,
          },
        },
      },
      false,
    );

    expect(markup).not.toContain("Revise in chat");
    expect(markup).not.toContain("Run once");
    expect(markup).not.toContain("Run as Goal");
    expect(markup).toContain("Open the advanced Plan workflow");
  });

  it("renders completed assistant responses as Markdown", () => {
    const markup = renderOutput(
      "# Ready\n\n- **security** checked",
      "completed",
      "complete",
    );

    expect(markup).toContain('class="markdown-content"');
    expect(markup).toContain('<h3 class="feed-entry-title">Colossus</h3>');
    expect(markup).toContain("<h4>Ready</h4>");
    expect(markup).toContain("<strong>security</strong>");
  });

  it("keeps active streamed output plain until the response is complete", () => {
    const markup = renderOutput("# Still streaming", "running", "watching");

    expect(markup).toContain("# Still streaming");
    expect(markup).not.toContain('class="markdown-content"');
    expect(markup).not.toContain("<h3>Still streaming</h3>");
    expect(markup).toContain('class="stream-caret"');
  });

  it("shows one live phase status instead of raw lifecycle notices", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "notice",
          reason: "run.phase.preparing",
          message: "run phase changed to preparing at turn 1",
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 2,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "notice",
          reason: "run.phase.responding",
          message: "run phase changed to responding at turn 1",
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 3,
        createdAt: "2026-07-21T12:00:02Z",
        update: {
          type: "notice",
          reason: "model.final_output",
          message: "the final visible output is available in the run result",
        },
      },
    ];

    const markup = renderOutput("", "running", "watching", updates);

    expect(markup).toContain("Responding…");
    expect(
      markup.match(/<div class="feed-entry live-run-status">/g),
    ).toHaveLength(1);
    expect(markup).not.toContain("run phase changed");
    expect(markup).not.toContain("final visible output");
    expect(markup).not.toContain("message-assistant");
  });

  it("replaces the live status with the assistant message when output starts", () => {
    const markup = renderOutput(
      "What would you like to build?",
      "running",
      "watching",
      [
        {
          runId: "run-markdown-test",
          sequence: 1,
          createdAt: "2026-07-21T12:00:00Z",
          update: {
            type: "notice",
            reason: "run.phase.responding",
            message: "run phase changed to responding at turn 1",
          },
        },
      ],
    );

    expect(markup).not.toContain("Responding…");
    expect(markup).toContain("What would you like to build?");
    expect(markup).toContain("message-assistant");
  });

  it("labels failed output as partial and shows safe response metadata", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "failure",
          status: "failed",
          failure: {
            reason: "provider.temporarily_unavailable",
            message: "The provider endpoint is not ready.",
            outcomeCertainty: "known",
            recoverable: true,
            httpStatus: 503,
            retryAfterMs: 1_000,
          },
        },
      },
    ];

    const markup = renderOutput(
      "The response began before the provider failed.",
      "failed",
      "complete",
      updates,
    );

    expect(markup).toContain("Partial response");
    expect(markup).toContain("provider.temporarily_unavailable");
    expect(markup).toContain("HTTP 503");
    expect(markup).toContain("1000 ms");
    expect(markup).toContain("<dd>Yes</dd>");
  });

  it("coalesces tool transitions into one expandable compact row", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-1",
            toolName: "shell.run",
            state: "requested",
            summary: "validated tool call requested",
          },
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 2,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-1",
            toolName: "shell.run",
            state: "started",
            summary: "tool execution started at turn 1",
          },
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 3,
        createdAt: "2026-07-21T12:00:02Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-1",
            toolName: "shell.run",
            state: "completed",
            summary: "tool execution completed at turn 1",
          },
        },
      },
    ];

    const markup = renderOutput("Done", "completed", "complete", updates);

    expect(
      markup.match(/<details class="compact-tool-activity">/g),
    ).toHaveLength(1);
    expect(markup).toContain("shell.run");
    expect(markup).toContain("requested");
    expect(markup).toContain("started");
    expect(markup).toContain("completed");
    expect(markup).toContain("tool execution completed at turn 1");
  });
});
