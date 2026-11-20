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

  it("continues with the known nondefault profile and effort of the admitted run", () => {
    const requests = new ProviderRetryRequests();
    requests.rememberAccepted("target", failedRun.runId, {
      modelProfile: "continuation-model",
      reasoningEffort: "high",
    });
    expect(requests.description("target", failedRun.runId)).toContain(
      "failed run’s model selection",
    );
    const request = requests.prepare("target", failedRun, {
      modelProfile: "different-model",
      reasoningEffort: "low",
    })!;
    expect(request.modelSelection).toEqual({
      modelProfile: "continuation-model",
      reasoningEffort: "high",
    });

    requests.settle("target", failedRun.runId, { type: "accepted" });
    const nextRun = { ...failedRun, runId: "accepted-continuation" };
    requests.rememberAccepted("target", nextRun.runId, request.modelSelection);
    const nextRequest = requests.prepare("target", nextRun, {})!;
    expect(nextRequest.modelSelection).toEqual(request.modelSelection);
    expect(nextRequest.idempotencyKey).not.toBe(request.idempotencyKey);
  });

  it("freezes the selected model and request identity when an admission reply is lost", () => {
    const requests = new ProviderRetryRequests();
    const selected: ComposerModelSelection = {
      modelProfile: "selected-profile",
      reasoningEffort: "high",
    };
    expect(requests.description("target", failedRun.runId)).toContain(
      "selected conversation model",
    );
    const request = requests.prepare("target", failedRun, selected);
    requests.settle("target", failedRun.runId, {
      type: "stale",
      dispatched: true,
    });
    selected.modelProfile = "different-profile";
    selected.reasoningEffort = "low";
    expect(requests.prepare("target", failedRun, selected)).toBe(request);
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
    });
    expect(
      requests.prepare("another-target", failedRun, selected)?.modelSelection,
    ).toEqual(selected);
  });

  it("releases a new attempt after rejection or a selection change before dispatch", () => {
    const requests = new ProviderRetryRequests();
    const first = requests.prepare("target", failedRun, {
      modelProfile: "old-profile",
    });
    requests.settle("target", failedRun.runId, {
      type: "failed",
      error: modelRejected,
    });
    const second = requests.prepare("target", failedRun, {
      modelProfile: "new-profile",
    });
    expect(second?.modelSelection).toEqual({ modelProfile: "new-profile" });
    expect(second?.idempotencyKey).toBe(first?.idempotencyKey);
    requests.settle("target", failedRun.runId, {
      type: "stale",
      dispatched: false,
    });
    expect(requests.prepare("target", failedRun, {})?.modelSelection).toEqual(
      {},
    );
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
      const first = requests.prepare("target", failedRun, {
        modelProfile: "first-profile",
      });
      requests.settle("target", failedRun.runId, { type: "failed", error });
      requests.settle("target", failedRun.runId, {
        type: "failed",
        error: modelRejected,
      });
      expect(requests.prepare("target", failedRun, {})).toBe(first);
    },
  );

  it("bounds unresolved admissions and allows exact replays at capacity", () => {
    const requests = new ProviderRetryRequests();
    const first = requests.prepare("target", failedRun, {
      reasoningEffort: "high",
    });
    for (let index = 1; index < MAX_CACHED_IDEMPOTENCY_ATTEMPTS; index += 1) {
      const source = { ...failedRun, runId: `unresolved-${index}` };
      expect(requests.prepare("target", source, {})).not.toBeNull();
      requests.settle("target", source.runId, {
        type: "stale",
        dispatched: true,
      });
    }
    const next = { ...failedRun, runId: "next-run" };
    expect(requests.prepare("target", next, {})).toBeNull();
    expect(requests.prepare("target", failedRun, {})).toBe(first);

    requests.settle("target", failedRun.runId, { type: "accepted" });
    expect(requests.prepare("target", next, {})).not.toBeNull();
  });
});
