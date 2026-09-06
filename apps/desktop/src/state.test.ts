import { describe, expect, it } from "vitest";

import {
  MAX_CACHED_IDEMPOTENCY_ATTEMPTS,
  MAX_CACHED_RUN_VIEWS,
  MAX_CONVERSATION_RUNS,
  MAX_FEED_ITEMS,
  MAX_FEED_TEXT_CHARACTERS,
  MAX_OUTPUT_CHARACTERS,
  MAX_PROMPT_BYTES,
  MAX_RECENT_RUNS,
  chatReducer,
  clampMaxTurns,
  connectionStateForError,
  initialChatState,
  isConnectionError,
  isPromptWithinByteLimit,
  operationFingerprint,
  selectConversationViews,
  stableIdempotentAttempt,
  utf8ByteLength,
  withBoundedEntry,
  withoutEntry,
} from "./state";
import type { Interaction, Run, RunUpdate } from "./types";
import { USE_CONFIGURED_MAX_TURNS } from "./types";

const baseRun: Run = {
  runId: "run-1",
  sessionId: "session-1",
  title: "Test work",
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
  etag: "run-etag",
  archived: false,
};

function withRun() {
  return chatReducer(initialChatState, { type: "upsert_run", run: baseRun });
}

function update(sequence: number, updateKind: RunUpdate["update"]): RunUpdate {
  return {
    runId: baseRun.runId,
    sequence,
    createdAt: `2026-07-20T12:00:0${sequence}Z`,
    update: updateKind,
  };
}

function runFixture(
  runId: string,
  status: Run["status"] = "completed",
  lastSequence = 1,
): Run {
  const terminal =
    status === "completed"
      ? {
          type: "result" as const,
          result: {
            output: `Output for ${runId}`,
            profile: "default",
            modelProfile: "default",
            providerProfile: "default-provider",
            model: "test-model",
            elapsedSeconds: 1,
          },
        }
      : null;
  return {
    ...baseRun,
    runId,
    sessionId: `session-${runId}`,
    status,
    createdAt: `2026-07-${String((lastSequence % 28) + 1).padStart(2, "0")}T12:00:00Z`,
    updatedAt: `2026-07-20T12:00:${String(lastSequence % 60).padStart(2, "0")}Z`,
    lastSequence,
    finishedAt: terminal === null ? null : "2026-07-20T12:01:00Z",
    terminal,
  };
}

const pendingInteraction: Interaction = {
  interactionId: "interaction-1",
  runId: baseRun.runId,
  kind: "user_prompt",
  status: "pending",
  createdAt: "2026-07-20T12:00:01Z",
  expiresAt: "2026-07-20T12:05:01Z",
  respondableByCaller: true,
  etag: "interaction-etag",
  content: {
    type: "user_prompt",
    question: "Choose a target",
    choices: [{ choiceId: "safe", label: "Safe target" }],
    allowFreeForm: false,
  },
};

describe("chatReducer", () => {
  it("deduplicates replayed sequences while accumulating output deltas", () => {
    const first = update(1, { type: "output_delta", delta: "Hello" });
    const second = update(2, { type: "output_delta", delta: " world" });

    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: first,
    });
    state = chatReducer(state, { type: "ingest_update", update: first });
    state = chatReducer(state, { type: "ingest_update", update: second });

    const view = state.views.get(baseRun.runId);
    expect(view?.output).toBe("Hello world");
    expect(view?.updates).toHaveLength(0);
    expect(view?.lastSequence).toBe(2);
  });

  it("projects result, failure, and cancellation updates into terminal run state", () => {
    const result = update(1, {
      type: "result",
      result: {
        output: "Done",
        profile: "default",
        modelProfile: "default",
        providerProfile: "default-provider",
        model: "test-model",
        elapsedSeconds: 1.25,
      },
    });
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: result,
    });
    expect(state.views.get(baseRun.runId)?.run.status).toBe("completed");
    expect(state.views.get(baseRun.runId)?.output).toBe("Done");

    state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "failure",
        status: "outcome_unknown",
        failure: {
          reason: "transport_lost",
          message: "The outcome could not be confirmed.",
          outcomeCertainty: "unknown",
        },
      }),
    });
    expect(state.views.get(baseRun.runId)?.run.status).toBe("outcome_unknown");
    expect(state.views.get(baseRun.runId)?.run.terminal?.type).toBe("failure");

    state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "cancellation",
        cancellation: { turn: 2, message: "Cancelled safely." },
      }),
    });
    expect(state.views.get(baseRun.runId)?.run.status).toBe("cancelled");
    expect(state.views.get(baseRun.runId)?.run.terminal?.type).toBe(
      "cancellation",
    );
  });

  it("adds pending interactions and removes them after a response", () => {
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "interaction",
        interaction: pendingInteraction,
      }),
    });
    expect(state.views.get(baseRun.runId)?.pendingInteractions).toEqual([
      pendingInteraction,
    ]);

    state = chatReducer(state, {
      type: "interaction_resolved",
      interaction: {
        ...pendingInteraction,
        status: "answered",
        respondableByCaller: false,
        etag: "",
      },
    });
    expect(state.views.get(baseRun.runId)?.pendingInteractions).toEqual([]);
    expect(state.views.get(baseRun.runId)?.run.pendingInteractionCount).toBe(0);
  });

  it("bounds renderer history and streamed output", () => {
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "output_delta",
        delta: "x".repeat(MAX_OUTPUT_CHARACTERS + 100),
      }),
    });
    for (let sequence = 2; sequence <= MAX_FEED_ITEMS + 5; sequence += 1) {
      state = chatReducer(state, {
        type: "ingest_update",
        update: update(sequence, {
          type: "notice",
          reason: "progress",
          message: `Checkpoint ${sequence}`,
        }),
      });
    }

    const view = state.views.get(baseRun.runId);
    expect(view?.output.length).toBe(MAX_OUTPUT_CHARACTERS);
    expect(view?.output).toMatch(/^\[Earlier output omitted/);
    expect(view?.updates).toHaveLength(MAX_FEED_ITEMS);
  });

  it("retains only bounded display projections and strips duplicate result output", () => {
    const oversized = "x".repeat(MAX_FEED_TEXT_CHARACTERS + 100);
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, { type: "reasoning_summary", summary: oversized }),
    });
    state = chatReducer(state, {
      type: "ingest_update",
      update: update(2, {
        type: "result",
        result: {
          output: oversized,
          profile: "default",
          modelProfile: "default",
          providerProfile: "default-provider",
          model: "test-model",
          elapsedSeconds: 1,
        },
      }),
    });

    const view = state.views.get(baseRun.runId);
    const retained = view?.updates[0];
    expect(view?.updates).toHaveLength(2);
    expect(retained?.update.type).toBe("reasoning_summary");
    if (retained?.update.type === "reasoning_summary") {
      expect(retained.update.summary).toHaveLength(MAX_FEED_TEXT_CHARACTERS);
      expect(retained.update.summary).toMatch(/Content truncated/);
    }
    expect(view?.output).toBe(oversized);
    expect(view?.run.terminal?.type).toBe("result");
    if (view?.run.terminal?.type === "result") {
      expect(view.run.terminal.result.output).toBe("");
    }
    const retainedResult = view?.updates[1];
    expect(retainedResult?.update.type).toBe("result");
    if (retainedResult?.update.type === "result") {
      expect(retainedResult.update.result.output).toBe("");
    }
  });

  it("clears pending interactions whenever a run becomes terminal", () => {
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "interaction",
        interaction: pendingInteraction,
      }),
    });
    state = chatReducer(state, {
      type: "ingest_update",
      update: update(2, { type: "state", status: "cancelled" }),
    });

    let view = state.views.get(baseRun.runId);
    expect(view?.pendingInteractions).toEqual([]);
    expect(view?.run.pendingInteractionCount).toBe(0);

    state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "interaction",
        interaction: pendingInteraction,
      }),
    });
    state = chatReducer(state, {
      type: "upsert_run",
      run: {
        ...baseRun,
        status: "completed",
        lastSequence: 2,
        pendingInteractionCount: 1,
        terminal: {
          type: "result",
          result: {
            output: "Done",
            profile: "default",
            modelProfile: "default",
            providerProfile: "default-provider",
            model: "test-model",
            elapsedSeconds: 1,
          },
        },
      },
    });
    view = state.views.get(baseRun.runId);
    expect(view?.pendingInteractions).toEqual([]);
    expect(view?.run.pendingInteractionCount).toBe(0);
  });

  it("does not let a stale snapshot regress feed state or pending interactions", () => {
    let state = chatReducer(withRun(), {
      type: "ingest_update",
      update: update(1, {
        type: "interaction",
        interaction: pendingInteraction,
      }),
    });
    state = chatReducer(state, {
      type: "ingest_update",
      update: update(2, { type: "state", status: "waiting" }),
    });

    const staleRun = {
      ...baseRun,
      status: "queued" as const,
      lastSequence: 1,
      pendingInteractionCount: 0,
    };
    state = chatReducer(state, {
      type: "hydrate_run",
      details: { run: staleRun, pendingInteractions: [] },
    });
    state = chatReducer(state, { type: "upsert_run", run: staleRun });

    const view = state.views.get(baseRun.runId);
    expect(view?.lastSequence).toBe(2);
    expect(view?.run.status).toBe("waiting");
    expect(view?.pendingInteractions).toEqual([pendingInteraction]);
    expect(view?.run.pendingInteractionCount).toBe(1);
  });

  it("keeps a submitted prompt local to its run across daemon hydration", () => {
    let state = withRun();
    state = chatReducer(state, {
      type: "record_local_prompt",
      runId: baseRun.runId,
      prompt: "Inspect the release safely",
    });
    state = chatReducer(state, {
      type: "hydrate_run",
      details: {
        run: { ...baseRun, lastSequence: 1 },
        pendingInteractions: [],
      },
    });

    expect(state.views.get(baseRun.runId)?.localPrompt).toBe(
      "Inspect the release safely",
    );
    expect(baseRun).not.toHaveProperty("localPrompt");
  });

  it("projects continuation runs as one ordered bounded conversation", () => {
    let state = initialChatState;
    for (let index = MAX_CONVERSATION_RUNS + 1; index >= 0; index -= 1) {
      state = chatReducer(state, {
        type: "upsert_run",
        run: {
          ...baseRun,
          runId: `continuation-${index}`,
          sessionId: baseRun.sessionId,
          createdAt: `2026-07-20T12:00:${String(index).padStart(2, "0")}Z`,
        },
      });
    }

    const views = selectConversationViews(state, baseRun.sessionId);
    expect(views).toHaveLength(MAX_CONVERSATION_RUNS);
    expect(views.map((view) => view.run.runId)).toEqual(
      Array.from(
        { length: MAX_CONVERSATION_RUNS },
        (_, index) => `continuation-${index + 2}`,
      ),
    );
    expect(selectConversationViews(state, null)).toEqual([]);
  });

  it("bounds caches while preserving the active and nonterminal run views", () => {
    const active = runFixture("active-terminal");
    const live = runFixture("background-live", "running");
    let state = chatReducer(initialChatState, {
      type: "upsert_run",
      run: active,
    });
    state = chatReducer(state, {
      type: "select_run",
      runId: active.runId,
    });
    state = chatReducer(state, { type: "upsert_run", run: live });

    for (let index = 0; index < MAX_CACHED_RUN_VIEWS + 8; index += 1) {
      state = chatReducer(state, {
        type: "upsert_run",
        run: runFixture(`terminal-${index}`, "completed", index + 2),
      });
    }

    expect(state.views.size).toBeLessThanOrEqual(MAX_CACHED_RUN_VIEWS);
    expect(state.views.has(active.runId)).toBe(true);
    expect(state.views.has(live.runId)).toBe(true);

    const summaries = Array.from({ length: MAX_RECENT_RUNS + 25 }, (_, index) =>
      runFixture(`summary-${index}`, "completed", index + 1),
    );
    state = chatReducer(state, {
      type: "replace_recent",
      runs: summaries,
      nextPageToken: "",
    });
    expect(state.recentRuns).toHaveLength(MAX_RECENT_RUNS);
    for (const summary of state.recentRuns) {
      if (summary.terminal?.type === "result") {
        expect(summary.terminal.result.output).toBe("");
      }
    }
  });

  it("removes every cached run in an archived session and clears its selection", () => {
    const first = { ...runFixture("archive-1"), sessionId: "session-archive" };
    const second = {
      ...runFixture("archive-2"),
      sessionId: "session-archive",
    };
    const retained = runFixture("retained");
    let state = chatReducer(initialChatState, {
      type: "upsert_run",
      run: first,
    });
    state = chatReducer(state, { type: "upsert_run", run: second });
    state = chatReducer(state, { type: "upsert_run", run: retained });
    state = chatReducer(state, { type: "select_run", runId: second.runId });

    state = chatReducer(state, {
      type: "remove_session",
      sessionId: "session-archive",
    });

    expect(state.activeRunId).toBeNull();
    expect([...state.views.keys()]).toEqual([retained.runId]);
    expect(state.recentRuns.map((run) => run.runId)).toEqual([retained.runId]);
  });
});

describe("stableIdempotentAttempt", () => {
  it("keeps the same key for retries and rotates it when the operation changes", () => {
    let generated = 0;
    const createKey = () => `key-${++generated}`;
    const fingerprint = operationFingerprint(["run-1", "cancel"]);

    const first = stableIdempotentAttempt(null, fingerprint, createKey);
    const retry = stableIdempotentAttempt(first, fingerprint, createKey);
    const changed = stableIdempotentAttempt(
      retry,
      operationFingerprint(["run-2", "cancel"]),
      createKey,
    );

    expect(retry.key).toBe(first.key);
    expect(changed.key).not.toBe(first.key);
    expect(generated).toBe(2);
  });

  it("bounds renderer attempt caches by evicting the oldest entry", () => {
    let attempts = new Map<number, string>();
    for (
      let index = 0;
      index < MAX_CACHED_IDEMPOTENCY_ATTEMPTS + 1;
      index += 1
    ) {
      attempts = withBoundedEntry(attempts, index, `attempt-${index}`);
    }

    expect(attempts.size).toBe(MAX_CACHED_IDEMPOTENCY_ATTEMPTS);
    expect(attempts.has(0)).toBe(false);
    expect(attempts.get(MAX_CACHED_IDEMPOTENCY_ATTEMPTS)).toBe(
      `attempt-${MAX_CACHED_IDEMPOTENCY_ATTEMPTS}`,
    );
  });

  it("rotates a lifecycle key after a completed operation is removed", () => {
    let generated = 0;
    const createKey = () => `key-${++generated}`;
    const fingerprint = operationFingerprint(["session-1", "archive"]);
    const first = stableIdempotentAttempt(null, fingerprint, createKey);
    const attempts = withoutEntry(
      withBoundedEntry(new Map(), "session-1:archive", first),
      "session-1:archive",
    );
    const next = stableIdempotentAttempt(
      attempts.get("session-1:archive") ?? null,
      fingerprint,
      createKey,
    );

    expect(next.key).not.toBe(first.key);
    expect(generated).toBe(2);
  });
});

describe("clampMaxTurns", () => {
  it("preserves the configured-default sentinel and bounds explicit overrides", () => {
    expect(clampMaxTurns(USE_CONFIGURED_MAX_TURNS)).toBe(
      USE_CONFIGURED_MAX_TURNS,
    );
    expect(clampMaxTurns(-1)).toBe(1);
    expect(clampMaxTurns(24)).toBe(24);
    expect(clampMaxTurns(8.7)).toBe(8);
    expect(clampMaxTurns(128)).toBe(100);
    expect(clampMaxTurns(Number.NaN)).toBe(1);
  });
});

describe("isConnectionError", () => {
  const error = (code: string) => ({
    code,
    message: "Safe message",
    retryable: true,
    outcomeUnknown: false,
    violations: [],
  });

  it("requires a reconnect for transport and identity failures only", () => {
    expect(isConnectionError(error("transport"))).toBe(true);
    expect(isConnectionError(error("unavailable"))).toBe(true);
    expect(isConnectionError(error("identity_mismatch"))).toBe(true);
    expect(isConnectionError(error("invalid_argument"))).toBe(false);
    expect(isConnectionError(error("outcome_unknown"))).toBe(false);
  });

  it("preserves the setup state for a missing native enrollment", () => {
    expect(connectionStateForError(error("not_configured"))).toBe(
      "not_configured",
    );
    expect(connectionStateForError(error("transport"))).toBe("disconnected");
  });
});

describe("UTF-8 prompt limits", () => {
  it("counts encoded bytes rather than JavaScript code units", () => {
    expect(utf8ByteLength("hello")).toBe(5);
    expect(utf8ByteLength("😀")).toBe(4);
    expect("😀").toHaveLength(2);
  });

  it("accepts the exact byte limit and rejects Unicode content over it", () => {
    expect(isPromptWithinByteLimit("a".repeat(MAX_PROMPT_BYTES))).toBe(true);
    expect(isPromptWithinByteLimit("😀".repeat(MAX_PROMPT_BYTES / 4))).toBe(
      true,
    );
    expect(
      isPromptWithinByteLimit("😀".repeat(MAX_PROMPT_BYTES / 4) + "a"),
    ).toBe(false);
  });
});
