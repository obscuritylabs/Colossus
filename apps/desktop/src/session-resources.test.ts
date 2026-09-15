import { describe, expect, it } from "vitest";

import {
  canContinuePlanFromRun,
  selectPlanForAutomaticDetails,
  selectSessionPlans,
  selectSessionSources,
  sessionActionCount,
} from "./session-resources";
import type { RunView } from "./state";
import type { Run } from "./types";

function view(runId: string, output: string, planRevision?: number): RunView {
  const run: Run = {
    runId,
    sessionId: "session-1",
    title: `Run ${runId}`,
    role: "primary",
    mode: planRevision === undefined ? "research" : "plan",
    status: "completed",
    createdAt: `2026-08-16T12:0${planRevision ?? 0}:00Z`,
    updatedAt: `2026-08-16T12:0${planRevision ?? 0}:10Z`,
    startedAt: null,
    finishedAt: null,
    lastSequence: 1,
    pendingInteractionCount: 0,
    terminal:
      planRevision === undefined
        ? null
        : {
            type: "result",
            result: {
              output,
              planId: "plan-1",
              planRevision,
              planStatus: "draft",
              profile: "desktop",
              modelProfile: "primary",
              providerProfile: "provider",
              model: "model",
              elapsedSeconds: 1,
            },
          },
    etag: `etag-${runId}`,
    archived: false,
  };
  return {
    run,
    localPrompt: null,
    output,
    updates: [
      {
        runId,
        sequence: 1,
        createdAt: run.updatedAt,
        update: {
          type: "tool_activity",
          activity: {
            callId: "call-1",
            toolName: "repo.map",
            state: "completed",
            summary: "Mapped repository",
            preview: null,
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
}

describe("session resources", () => {
  it("selects the latest revision for each durable plan", () => {
    expect(
      selectSessionPlans([view("one", "draft", 1), view("two", "revised", 2)]),
    ).toEqual([
      expect.objectContaining({
        planId: "plan-1",
        revision: 2,
        sourceRunId: "two",
      }),
    ]);
  });

  it("selects each newly released plan revision for automatic details once", () => {
    const firstPlans = selectSessionPlans([view("one", "draft", 1)]);
    const first = selectPlanForAutomaticDetails(
      "session-1",
      firstPlans,
      new Set(),
    );
    expect(first.plan).toEqual(
      expect.objectContaining({ planId: "plan-1", revision: 1 }),
    );

    const repeated = selectPlanForAutomaticDetails(
      "session-1",
      firstPlans,
      first.observedKeys,
    );
    expect(repeated.plan).toBeNull();

    const revised = selectPlanForAutomaticDetails(
      "session-1",
      selectSessionPlans([view("one", "draft", 1), view("two", "revised", 2)]),
      repeated.observedKeys,
    );
    expect(revised.plan).toEqual(
      expect.objectContaining({ planId: "plan-1", revision: 2 }),
    );
  });

  it("does not automatically open a cancelled plan", () => {
    const plans = selectSessionPlans([view("one", "draft", 1)]).map((plan) => ({
      ...plan,
      cancelled: true,
    }));

    expect(
      selectPlanForAutomaticDetails("session-1", plans, new Set()).plan,
    ).toBeNull();
  });

  it("retires old draft actions when a plan is revised or consumed", () => {
    const first = view("one", "draft", 1);
    const revised = view("two", "revised", 2);
    const plans = selectSessionPlans([first, revised]);
    expect(canContinuePlanFromRun("one", plans, true)).toBe(false);
    expect(canContinuePlanFromRun("two", plans, true)).toBe(true);
    expect(canContinuePlanFromRun("two", plans, false)).toBe(false);

    const executed = view("three", "Implementation completed.", 4);
    executed.run.mode = "execute";
    if (executed.run.terminal?.type === "result") {
      executed.run.terminal.result.planStatus = "executed";
    }
    const consumed = selectSessionPlans([executed, first, revised]);
    expect(consumed[0]?.output).toBe("revised");
    for (const id of ["one", "two", "three"]) {
      expect(canContinuePlanFromRun(id, consumed, true)).toBe(false);
    }
    expect(
      selectPlanForAutomaticDetails("session-1", consumed, new Set()).plan,
    ).toBeNull();
  });

  it("does not present execution output as a missing planning response", () => {
    const executed = view("executed", "Implementation completed.", 3);
    executed.run.mode = "execute";
    if (executed.run.terminal?.type === "result") {
      executed.run.terminal.result.planStatus = "executed";
    }
    expect(selectSessionPlans([executed])[0]?.output).toBe("");
  });

  it("aggregates bounded released sources and tool actions across runs", () => {
    const research = view(
      "research",
      "Report\n\n## Sources\n- [Web] Example — https://example.com/source",
    );
    expect(selectSessionSources([research])).toEqual([
      expect.objectContaining({
        title: "Example",
        uri: "https://example.com/source",
        sourceRunId: "research",
      }),
    ]);
    expect(sessionActionCount([research, view("follow-up", "")])).toBe(2);
  });
});
