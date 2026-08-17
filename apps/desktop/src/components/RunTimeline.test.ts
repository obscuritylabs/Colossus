import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { RunView, StreamState } from "../state";
import type { Run, RunStatus, RunUpdate } from "../types";
import type { ActivityPresentation } from "./RunTimeline";
import { RunTimeline } from "./RunTimeline";

function renderOutput(
  output: string,
  status: RunStatus,
  streamState: StreamState,
  updates: RunUpdate[] = [],
  runOverrides: Partial<Run> = {},
  planContinuationAvailable = true,
  activityPresentation: ActivityPresentation = "thread",
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
    archived: false,
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
      activityPresentation,
      activityComparison: true,
      planContinuationAvailable,
      planWorkflowAvailable: true,
      onInspectPlan: vi.fn(),
      onOpenPlanWorkflow: vi.fn(),
      onRevisePlan: vi.fn(),
      onExecutePlan: vi.fn(async () => undefined),
    }),
  );
}

describe("RunTimeline assistant output", () => {
  it("offers copy actions for user messages and the Colossus response", () => {
    const markup = renderOutput("Copy this response", "completed", "complete", [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "message",
          message: {
            sessionId: "session-markdown-test",
            runId: "run-markdown-test",
            sequence: 1,
            role: "user",
            content: [{ type: "text", text: "Copy this prompt" }],
            createdAt: "2026-07-21T12:00:00Z",
          },
        },
      },
    ]);

    expect(markup).toContain('aria-label="Copy message"');
    expect(markup).toContain('aria-label="Copy Colossus response"');
    expect(markup.match(/message-copy-button/g)).toHaveLength(2);
  });

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
    expect(markup).toContain("Read plan");
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

  it("turns a Research report source list into drawer launch controls", () => {
    const markup = renderOutput(
      "# Finding\n\nEvidence.\n\n## Sources\n\n- [R1] Runtime docs — repo://docs/runtime.md\n- [R2] Web docs — https://example.test/docs",
      "completed",
      "complete",
      [],
      { mode: "research" },
    );

    expect(markup).toContain('class="inline-research-sources"');
    expect(markup).toContain("View evidence");
    expect(markup).toContain("Runtime docs");
    expect(markup).toContain("Web docs");
    expect(markup).not.toContain("repo://docs/runtime.md");
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
    expect(markup).toContain("Retry after 1000 ms");
    expect(markup).toContain("<dd>Recoverable</dd>");
    expect(markup).toContain('class="failure-title"');
    expect(markup).toContain('aria-label="Failure details"');
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
            toolName: "repo.map",
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
            toolName: "repo.map",
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
            toolName: "repo.map",
            state: "completed",
            summary: "tool execution completed at turn 1",
            preview:
              '{"root":".","files":[{"path":"src/main.rs","bytes":42}],"file_count":1}',
          },
        },
      },
    ];

    const markup = renderOutput("Done", "completed", "complete", updates);

    expect(
      markup.match(
        /<details class="compact-tool-activity activity-tool-thread activity-state-completed">/g,
      ),
    ).toHaveLength(1);
    expect(markup).toContain("repo.map");
    expect(markup).toContain("requested");
    expect(markup).toContain("started");
    expect(markup).toContain("completed");
    expect(markup).toContain("tool execution completed at turn 1");
    expect(markup).toContain('class="tool-activity-preview"');
    expect(markup).toContain("<pre>");
    expect(markup).toContain("src/main.rs");
    expect(markup).toContain("file_count");
    expect(markup).not.toContain(
      "The tool completed, but this activity feed does not include an output preview.",
    );
  });

  it("interleaves released reasoning summaries with tool actions in the working thread", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "reasoning_summary",
          summary: "Checking the workspace boundary",
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 2,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-map",
            toolName: "repo.map",
            state: "completed",
            summary: "Mapped repository structure",
          },
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 3,
        createdAt: "2026-07-21T12:00:02Z",
        update: {
          type: "reasoning_summary",
          summary: "The package is nearly complete",
        },
      },
    ];

    const markup = renderOutput(
      "Done",
      "completed",
      "complete",
      updates,
      {},
      true,
      "thread",
    );

    expect(markup).toContain("run-activity-thread");
    expect(markup).toContain("run-state-completed");
    expect(markup).toContain("activity-state-completed");
    expect(markup).toContain("2 notes");
    expect(markup.indexOf("Checking the workspace boundary")).toBeLessThan(
      markup.indexOf("Mapped repository structure"),
    );
    expect(markup.indexOf("Mapped repository structure")).toBeLessThan(
      markup.indexOf("The package is nearly complete"),
    );
  });

  it("exposes active run and tool states for timeline color treatment", () => {
    const markup = renderOutput(
      "",
      "running",
      "watching",
      [
        {
          runId: "run-markdown-test",
          sequence: 1,
          createdAt: "2026-07-21T12:00:00Z",
          update: {
            type: "tool_activity",
            activity: {
              callId: "call-map",
              toolName: "repo.map",
              state: "started",
              summary: "Mapping repository structure",
            },
          },
        },
      ],
      {},
      true,
      "thread",
    );

    expect(markup).toContain("run-state-running");
    expect(markup).toContain("activity-state-started");
  });

  it("renders the same canonical activity as a single run capsule", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "reasoning_summary",
          summary: "Inspecting released context",
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 2,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-shell",
            toolName: "shell.run",
            state: "failed",
            summary: "Command denied by workspace policy",
            input: '{"command":"ps"}',
          },
        },
      },
    ];

    const markup = renderOutput(
      "Done",
      "completed",
      "complete",
      updates,
      {},
      true,
      "capsule",
    );

    expect(markup).toContain("run-activity-capsule");
    expect(markup).toContain("1 action");
    expect(markup).toContain("1 note");
    expect(markup).toContain("Command denied by workspace policy");
    expect(markup).toContain("ps");
    expect(markup).toContain("run-activity-exceptions");
  });

  it("does not create thinking rows when the runtime released none", () => {
    const markup = renderOutput("Done", "completed", "complete", [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-map",
            toolName: "repo.map",
            state: "completed",
            summary: "Mapped repository structure",
          },
        },
      },
    ]);

    expect(markup).not.toContain("activity-thought");
    expect(markup).not.toContain("note</small>");
  });

  it("labels an unstarted tool as cancelled instead of failed", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-cancelled",
            toolName: "filesystem.list",
            state: "cancelled",
            summary: "tool execution was cancelled before start at turn 1",
          },
        },
      },
    ];

    const markup = renderOutput("", "failed", "complete", updates);

    expect(markup).toContain("filesystem.list");
    expect(markup).toContain("tool-state-cancelled");
    expect(markup).toContain(">cancelled<");
    expect(markup).not.toContain("tool-state-failed");
    expect(markup).toContain('class="tool-activity-preview"');
    expect(markup).toContain("Preview");
    expect(markup).toContain(
      "No preview was generated because the tool was cancelled before it started.",
    );
  });

  it("shows the validated shell command while execution is running", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-shell-input",
        sequence: 1,
        createdAt: "2026-07-21T12:00:00Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-shell",
            toolName: "shell.run",
            state: "requested",
            summary: "validated tool call requested",
          },
        },
      },
      {
        runId: "run-shell-input",
        sequence: 2,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-shell",
            toolName: "shell.run",
            state: "started",
            summary: "tool execution started at turn 1",
            input: '{"command":"git status --short","cwd":"."}',
          },
        },
      },
    ];

    const markup = renderOutput("", "running", "watching", updates);

    expect(markup).toContain('aria-label="Tool input"');
    expect(markup).toContain("Input");
    expect(markup).toContain("git status --short");
    expect(markup).toContain(
      "No preview is available while the tool is still running.",
    );
  });

  it("counts Research progress as steps and uses its canonical duration", () => {
    const updates: RunUpdate[] = [
      {
        runId: "run-markdown-test",
        sequence: 1,
        createdAt: "2026-07-21T12:00:01Z",
        update: {
          type: "notice",
          reason: "research.planning.completed",
          message: "Accepted model-generated research queries.",
        },
      },
      {
        runId: "run-markdown-test",
        sequence: 2,
        createdAt: "2026-07-21T12:00:40Z",
        update: {
          type: "notice",
          reason: "research.synthesis.completed",
          message: "Accepted model-synthesized cited report.",
        },
      },
    ];
    const markup = renderOutput("Report", "completed", "complete", updates, {
      mode: "research",
      terminal: {
        type: "result",
        result: {
          output: "Report",
          profile: "research",
          modelProfile: "research",
          providerProfile: "research",
          model: "research",
          elapsedSeconds: 42.25,
        },
      },
    });

    expect(markup).toContain("2 steps");
    expect(markup).toContain("42s");
    expect(markup).not.toContain("0 actions");
    expect(markup).not.toContain("<small>0 steps · 0s</small>");
  });
});
