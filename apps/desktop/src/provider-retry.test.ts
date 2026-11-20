import { describe, expect, it } from "vitest";

import {
  ProviderRetryRequests,
  selectedProviderRetryRun,
} from "./provider-retry";
import type { ComposerModelSelection } from "./composer-model";
import {
  chatReducer,
  initialChatState,
  MAX_CACHED_IDEMPOTENCY_ATTEMPTS,
} from "./state";
import type { CommandError, Run, RunFailure } from "./types";

const failure: RunFailure = {
  reason: "provider.temporarily_unavailable",
  message: "Provider returned HTTP 500.",
  outcomeCertainty: "known",
  recoverable: true,
  httpStatus: 500,
};

const failedRun: Run = {
  runId: "failed-run",
  sessionId: "saved-session",
  title: "Failed response",
  role: "primary",
  mode: "execute",
  status: "failed",
  createdAt: "2026-11-20T17:00:00Z",
  updatedAt: "2026-11-20T17:01:00Z",
  startedAt: "2026-11-20T17:00:00Z",
  finishedAt: "2026-11-20T17:01:00Z",
  lastSequence: 1,
  pendingInteractionCount: 0,
  terminal: { type: "failure", failure },
  etag: "failed-etag",
  archived: false,
};

const modelRejected: CommandError = {
  code: "invalid_argument",
  message: "The requested model preferences are unavailable.",
  outcomeUnknown: false,
  retryable: false,
  violations: [{ field: "model_profile", description: "Unknown profile." }],
};

function selected(run: Run = failedRun) {
  return chatReducer(
    chatReducer(initialChatState, { type: "upsert_run", run }),
    { type: "select_run", runId: run.runId },
  );
}

function selection(
  modelSelection: ComposerModelSelection = {},
  pluginSkillIds: readonly string[] = [],
) {
  return { modelSelection, pluginSkillIds };
}

describe("Desktop provider continuation", () => {
  it.each([500, 502, 503, 504])(
    "accepts a confirmed HTTP %i Execute failure",
    (httpStatus) => {
      const run: Run = {
        ...failedRun,
        terminal: { type: "failure", failure: { ...failure, httpStatus } },
      };
      expect(selectedProviderRetryRun(selected(run))?.runId).toBe(run.runId);
    },
  );

  it.each([
    { outcomeCertainty: "unknown" },
    { recoverable: false },
    { httpStatus: 501 },
    { reason: "effect.http_status" },
  ] satisfies Partial<RunFailure>[])(
    "rejects an ineligible failure %o",
    (changes) => {
      const run: Run = {
        ...failedRun,
        terminal: { type: "failure", failure: { ...failure, ...changes } },
      };
      expect(selectedProviderRetryRun(selected(run))).toBeNull();
    },
  );

  it.each([
    { mode: "plan" },
    { mode: "goal" },
    { mode: "research" },
    { status: "running" },
    { archived: true },
    { continuable: false },
  ] satisfies Partial<Run>[])("rejects an ineligible run %o", (changes) => {
    expect(
      selectedProviderRetryRun(selected({ ...failedRun, ...changes })),
    ).toBeNull();
  });

  it("uses the selected run and honors later runs and running siblings", () => {
    const chat = selected();
    const newer: Run = {
      ...failedRun,
      runId: "newer-run",
      createdAt: "2026-11-20T17:02:00Z",
      status: "completed",
    };
    expect(
      selectedProviderRetryRun({
        ...chat,
        recentRuns: [...chat.recentRuns, newer],
      }),
    ).toBeNull();
    expect(
      selectedProviderRetryRun(
        chatReducer(chat, {
          type: "upsert_run",
          run: { ...newer, sessionId: "another-session", status: "running" },
        }),
      )?.runId,
    ).toBe(failedRun.runId);
    expect(
      selectedProviderRetryRun(
        chatReducer(chat, {
          type: "upsert_run",
          run: {
            ...newer,
            createdAt: "2026-11-20T16:59:00Z",
            status: "running",
          },
        }),
      ),
    ).toBeNull();
    expect(selectedProviderRetryRun({ ...chat, activeRunId: null })).toBeNull();
  });

  it("replays the admitted model settings and exact explicit skills", () => {
    const requests = new ProviderRetryRequests();
    const admittedSkills = ["review/inline", "tools/testing"];
    requests.rememberAccepted(
      "target",
      failedRun.runId,
      selection(
        { modelProfile: "continuation-model", reasoningEffort: "high" },
        admittedSkills,
      ),
    );
    admittedSkills.push("tools/added-later");
    expect(requests.description("target", failedRun.runId)).toContain(
      "failed run’s model selection",
    );
    const request = requests.prepare(
      "target",
      failedRun,
      selection({ modelProfile: "different-model", reasoningEffort: "low" }, [
        "tools/current",
      ]),
    )!;
    expect(request.modelSelection).toEqual({
      modelProfile: "continuation-model",
      reasoningEffort: "high",
    });
    expect(request.pluginSkillIds).toEqual(["review/inline", "tools/testing"]);
    expect(Object.isFrozen(request.pluginSkillIds)).toBe(true);

    requests.settle("target", failedRun.runId, {
      type: "stale",
      dispatched: true,
    });
    expect(
      requests.prepare(
        "target",
        failedRun,
        selection({ modelProfile: "third-model" }, ["tools/changed"]),
      ),
    ).toBe(request);

    requests.settle("target", failedRun.runId, { type: "accepted" });
    const nextRun = { ...failedRun, runId: "accepted-continuation" };
    requests.rememberAccepted("target", nextRun.runId, request);
    const nextRequest = requests.prepare("target", nextRun, selection())!;
    expect(nextRequest.modelSelection).toEqual(request.modelSelection);
    expect(nextRequest.pluginSkillIds).toEqual(request.pluginSkillIds);
    expect(nextRequest.idempotencyKey).not.toBe(request.idempotencyKey);
  });

  it("preserves an explicitly empty admitted skill selection", () => {
    const requests = new ProviderRetryRequests();
    requests.rememberAccepted("target", failedRun.runId, selection());
    expect(
      requests.prepare("target", failedRun, selection({}, ["tools/current"]))
        ?.pluginSkillIds,
    ).toEqual([]);
  });

  it("freezes the historical fallback selection and request identity after a lost reply", () => {
    const requests = new ProviderRetryRequests();
    const selected: ComposerModelSelection = {
      modelProfile: "selected-profile",
      reasoningEffort: "high",
    };
    const selectedSkills = ["tools/selected"];
    expect(requests.description("target", failedRun.runId)).toContain(
      "selected conversation model",
    );
    const request = requests.prepare(
      "target",
      failedRun,
      selection(selected, selectedSkills),
    );
    requests.settle("target", failedRun.runId, {
      type: "stale",
      dispatched: true,
    });
    selected.modelProfile = "different-profile";
    selected.reasoningEffort = "low";
    selectedSkills.push("tools/changed");
    expect(
      requests.prepare(
        "target",
        failedRun,
        selection(selected, selectedSkills),
      ),
    ).toBe(request);
    expect(requests.description("target", failedRun.runId)).toBe(
      "Send the same continuation request again",
    );
    expect(request).toEqual({
      role: "primary",
      sessionId: "saved-session",
      idempotencyKey: "desktop-provider-retry:failed-run",
      modelSelection: {
        modelProfile: "selected-profile",
        reasoningEffort: "high",
      },
      pluginSkillIds: ["tools/selected"],
    });
    expect(
      requests.prepare(
        "another-target",
        failedRun,
        selection(selected, selectedSkills),
      ),
    ).toMatchObject(selection(selected, selectedSkills));
  });

  it("releases a new attempt after rejection or a selection change before dispatch", () => {
    const requests = new ProviderRetryRequests();
    const first = requests.prepare(
      "target",
      failedRun,
      selection({ modelProfile: "old-profile" }),
    );
    requests.settle("target", failedRun.runId, {
      type: "failed",
      error: modelRejected,
    });
    const second = requests.prepare(
      "target",
      failedRun,
      selection({ modelProfile: "new-profile" }),
    );
    expect(second?.modelSelection).toEqual({ modelProfile: "new-profile" });
    expect(second?.idempotencyKey).toBe(first?.idempotencyKey);
    requests.settle("target", failedRun.runId, {
      type: "stale",
      dispatched: false,
    });
    expect(
      requests.prepare("target", failedRun, selection())?.modelSelection,
    ).toEqual({});
  });

  it.each([
    { ...modelRejected, outcomeUnknown: true },
    {
      ...modelRejected,
      violations: [{ field: "setupPackage", description: "Invalid settings." }],
    },
    { ...modelRejected, code: "desktop_request_failed", violations: [] },
    { ...modelRejected, code: "unavailable", violations: [] },
  ])(
    "retains a possibly admitted request after a later definite rejection",
    (error) => {
      const requests = new ProviderRetryRequests();
      const first = requests.prepare(
        "target",
        failedRun,
        selection({ modelProfile: "first-profile" }),
      );
      requests.settle("target", failedRun.runId, { type: "failed", error });
      requests.settle("target", failedRun.runId, {
        type: "failed",
        error: modelRejected,
      });
      expect(requests.prepare("target", failedRun, selection())).toBe(first);
    },
  );

  it("bounds unresolved admissions and allows exact replays at capacity", () => {
    const requests = new ProviderRetryRequests();
    const first = requests.prepare(
      "target",
      failedRun,
      selection({ reasoningEffort: "high" }),
    );
    for (let index = 1; index < MAX_CACHED_IDEMPOTENCY_ATTEMPTS; index += 1) {
      const source = { ...failedRun, runId: `unresolved-${index}` };
      expect(requests.prepare("target", source, selection())).not.toBeNull();
      requests.settle("target", source.runId, {
        type: "stale",
        dispatched: true,
      });
    }
    const next = { ...failedRun, runId: "next-run" };
    expect(requests.prepare("target", next, selection())).toBeNull();
    expect(requests.prepare("target", failedRun, selection())).toBe(first);

    requests.settle("target", failedRun.runId, { type: "accepted" });
    expect(requests.prepare("target", next, selection())).not.toBeNull();
  });
});
