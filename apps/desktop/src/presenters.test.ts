import { describe, expect, it } from "vitest";

import {
  MAX_PRESENTED_ACTIVITY_ITEMS,
  MAX_PRESENTED_ARTIFACTS,
  MAX_PRESENTED_WORK_ITEMS,
  agentRoleLabel,
  presentRunStatus,
  safeDisplayLabel,
  selectOperationalActivity,
  selectRecentWork,
  selectReleasedArtifacts,
} from "./presenters";
import type { RunView } from "./state";
import type { ArtifactReference, Run, RunUpdate, RunUpdateKind } from "./types";

const BASE_RUN: Run = {
  runId: "run-1",
  sessionId: "opaque-session-1",
  role: "primary",
  mode: "execute",
  status: "running",
  createdAt: "2026-07-20T12:00:00Z",
  updatedAt: "2026-07-20T12:00:00Z",
  startedAt: "2026-07-20T12:00:00Z",
  finishedAt: null,
  lastSequence: 0,
  pendingInteractionCount: 0,
  terminal: null,
  etag: "private-etag",
  selectedSkills: [],
};

function runFixture(overrides: Partial<Run>): Run {
  return { ...BASE_RUN, ...overrides };
}

function update(
  sequence: number,
  kind: RunUpdateKind,
  createdAt = `2026-07-20T12:00:${String(sequence % 60).padStart(2, "0")}Z`,
): RunUpdate {
  return {
    runId: BASE_RUN.runId,
    sequence,
    createdAt,
    update: kind,
  };
}

function viewFixture(updates: RunUpdate[], run = BASE_RUN): RunView {
  return {
    run,
    localPrompt: null,
    output: "",
    updates,
    seenSequences: new Set(updates.map(({ sequence }) => sequence)),
    lastSequence: updates.at(-1)?.sequence ?? 0,
    pendingInteractions: [],
    usage: null,
    streamState: "idle",
    streamError: null,
  };
}

function artifactFixture(
  artifactId: string,
  overrides: Partial<ArtifactReference> = {},
): ArtifactReference {
  return {
    artifactId,
    fileName: "report.pdf",
    mediaType: "application/pdf",
    sizeBytes: 2048,
    sha256: "a".repeat(64),
    purpose: "run_output",
    state: "available",
    createdAt: "2026-07-20T12:00:00Z",
    ...overrides,
  };
}

function artifactMessage(
  sequence: number,
  artifact: ArtifactReference,
): RunUpdate {
  return update(sequence, {
    type: "message",
    message: {
      sessionId: "opaque-session-1",
      runId: BASE_RUN.runId,
      sequence,
      role: "tool",
      content: [{ type: "artifact", artifact }],
      createdAt: `2026-07-20T12:00:${String(sequence).padStart(2, "0")}Z`,
    },
  });
}

describe("safe presentation copy", () => {
  it("uses explicit, concise status semantics", () => {
    expect(presentRunStatus("waiting")).toEqual({
      label: "Needs input",
      copy: "Waiting for your input",
      tone: "attention",
    });
    expect(presentRunStatus("outcome_unknown").copy).toMatch(
      /verify.*before retrying/i,
    );
  });

  it("removes control and bidi formatting characters and bounds labels", () => {
    expect(safeDisplayLabel("  alpha\n\u202Ebeta  ", "Fallback")).toBe(
      "alpha beta",
    );
    expect(safeDisplayLabel("x".repeat(100), "Fallback", 12)).toBe(
      `${"x".repeat(11)}…`,
    );
    expect(agentRoleLabel("document-review")).toBe("Document review");
    expect(agentRoleLabel("\n\u202E")).toBe("Default agent");
  });
});

describe("selectRecentWork", () => {
  const now = new Date(2026, 6, 20, 18, 0, 0);
  const today = new Date(2026, 6, 20, 12, 0, 0).toISOString();
  const yesterday = new Date(2026, 6, 19, 12, 0, 0).toISOString();
  const earlier = new Date(2026, 6, 15, 12, 0, 0).toISOString();

  it("groups runs as work without manufacturing session labels", () => {
    const groups = selectRecentWork(
      [
        runFixture({
          runId: "old-active",
          sessionId: "session-do-not-display",
          role: "fleet-primary",
          status: "running",
          updatedAt: earlier,
        }),
        runFixture({
          runId: "today",
          role: "document-editor",
          status: "completed",
          updatedAt: today,
        }),
        runFixture({
          runId: "yesterday",
          role: "code-review",
          mode: "plan",
          status: "completed",
          updatedAt: yesterday,
        }),
        runFixture({
          runId: "earlier",
          role: "research",
          status: "failed",
          updatedAt: earlier,
        }),
      ],
      { now },
    );

    expect(groups.map(({ key }) => key)).toEqual([
      "active",
      "today",
      "yesterday",
      "earlier",
    ]);
    expect(groups[0]?.items[0]?.title).toBe("Fleet primary");
    expect(JSON.stringify(groups)).not.toContain("session-do-not-display");
    expect(groups.flatMap(({ items }) => items)).toHaveLength(4);
  });

  it("searches only safe display fields and preserves the renderer bound", () => {
    const runs = Array.from(
      { length: MAX_PRESENTED_WORK_ITEMS + 25 },
      (_, index) =>
        runFixture({
          runId: `run-${index}`,
          sessionId: `secret-session-${index}`,
          role: index === 3 ? "planner" : "primary",
          mode: index === 3 ? "plan" : "execute",
          status: "completed",
          updatedAt: new Date(today).toISOString(),
        }),
    );

    expect(
      selectRecentWork(runs, { now }).flatMap(({ items }) => items),
    ).toHaveLength(MAX_PRESENTED_WORK_ITEMS);
    expect(
      selectRecentWork(runs, { now, query: "planner" }).flatMap(
        ({ items }) => items,
      ),
    ).toHaveLength(1);
    expect(selectRecentWork(runs, { now, query: "secret-session-3" })).toEqual(
      [],
    );
  });
});

describe("selectReleasedArtifacts", () => {
  it("deduplicates released references, exposes safe metadata, and gates opening", () => {
    const first = artifactFixture("artifact-1", {
      state: "quarantined",
      sha256: "private-old-digest",
    });
    const released = artifactFixture("artifact-1", {
      fileName: "final\nreport.pdf",
      state: "available",
      sha256: "private-new-digest",
      createdAt: "2026-07-20T12:00:02Z",
    });
    const pending = artifactFixture("artifact-2", {
      fileName: "source.docx",
      mediaType:
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      state: "quarantined",
      createdAt: "2026-07-20T12:00:03Z",
    });
    const artifacts = selectReleasedArtifacts(
      viewFixture([
        artifactMessage(1, first),
        artifactMessage(2, released),
        artifactMessage(3, pending),
      ]),
    );

    expect(artifacts).toHaveLength(2);
    expect(artifacts[0]).toMatchObject({
      artifactId: "artifact-2",
      stateLabel: "Pending review",
      canOpen: false,
    });
    expect(artifacts[1]).toMatchObject({
      artifactId: "artifact-1",
      fileName: "final report.pdf",
      stateLabel: "Available",
      canOpen: true,
      sizeLabel: "2.0 KB",
    });
    expect(artifacts[1]).not.toHaveProperty("sha256");
    expect(JSON.stringify(artifacts)).not.toContain("private-new-digest");
  });

  it("bounds artifacts even when messages contain more references", () => {
    const updates = Array.from(
      { length: MAX_PRESENTED_ARTIFACTS + 10 },
      (_, index) =>
        artifactMessage(
          index + 1,
          artifactFixture(`artifact-${index}`, {
            createdAt: `2026-07-20T12:${String(index % 60).padStart(2, "0")}:00Z`,
          }),
        ),
    );
    expect(selectReleasedArtifacts(viewFixture(updates))).toHaveLength(
      MAX_PRESENTED_ARTIFACTS,
    );
  });
});

describe("selectOperationalActivity", () => {
  it("flattens released updates without projecting output or approval secrets", () => {
    const updates: RunUpdate[] = [
      update(1, { type: "output_delta", delta: "private streamed output" }),
      update(2, {
        type: "tool_activity",
        activity: {
          callId: "private-call-id",
          toolName: "docs.fetch",
          state: "waiting_approval",
          summary: "Review\nrequested",
        },
      }),
      update(3, {
        type: "interaction",
        interaction: {
          interactionId: "interaction-1",
          runId: BASE_RUN.runId,
          kind: "approval",
          status: "pending",
          createdAt: "2026-07-20T12:00:03Z",
          expiresAt: "2026-07-20T12:05:03Z",
          respondableByCaller: true,
          etag: "private-interaction-etag",
          content: {
            type: "approval",
            reason: "Publish the document",
            action: "private-action",
            resource: "private-resource",
            risk: "medium",
            requestHash: "private-request-hash",
          },
        },
      }),
      update(4, {
        type: "result",
        result: {
          output: "private terminal output",
          profile: "private-profile",
          modelProfile: "private-profile",
          providerProfile: "private-provider",
          model: "private-model",
          elapsedSeconds: 1.25,
        },
      }),
    ];

    const activity = selectOperationalActivity(viewFixture(updates));
    const serialized = JSON.stringify(activity);
    expect(activity.map(({ kind }) => kind)).toEqual([
      "result",
      "interaction",
      "tool",
    ]);
    expect(activity[1]).toMatchObject({
      title: "Approval requested",
      detail: "Publish the document",
      stateLabel: "Needs attention",
    });
    expect(activity[2]).toMatchObject({
      title: "docs.fetch",
      detail: "Review requested",
      stateLabel: "Needs approval",
    });
    expect(serialized).not.toContain("private streamed output");
    expect(serialized).not.toContain("private terminal output");
    expect(serialized).not.toContain("private-request-hash");
    expect(serialized).not.toContain("private-call-id");
  });

  it("merges cached views newest-first and keeps the activity bound", () => {
    const firstView = viewFixture([
      update(1, {
        type: "notice",
        reason: "checkpoint",
        message: "First view",
      }),
    ]);
    const secondRun = runFixture({ runId: "run-2", role: "research" });
    const secondView = viewFixture(
      Array.from({ length: MAX_PRESENTED_ACTIVITY_ITEMS + 10 }, (_, index) => ({
        ...update(index + 1, {
          type: "notice",
          reason: "checkpoint",
          message: `Checkpoint ${index}`,
        }),
        runId: secondRun.runId,
        createdAt: `2026-07-21T12:${String(index % 60).padStart(2, "0")}:00Z`,
      })),
      secondRun,
    );

    const activity = selectOperationalActivity([firstView, secondView]);
    expect(activity).toHaveLength(MAX_PRESENTED_ACTIVITY_ITEMS);
    expect(activity[0]?.runId).toBe(secondRun.runId);
    expect(activity[0]?.agentLabel).toBe("Research");
  });
});
