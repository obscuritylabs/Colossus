import assert from "node:assert/strict";
import { test } from "node:test";

import type { RunUpdate } from "../src/gen/colossus/api/v1alpha1/agent_run.js";
import {
  RunFeedProtocolError,
  isTerminalRunUpdate,
  type RunFeedItem,
  watchRun,
} from "../src/watch.js";

const generatedTerminalPredicate: (update: RunUpdate) => boolean =
  isTerminalRunUpdate;

interface Value {
  readonly terminal: boolean;
}

async function* items(
  values: ReadonlyArray<RunFeedItem<Value>>,
): AsyncGenerator<RunFeedItem<Value>> {
  yield* values;
}

async function* transientAfter(
  value: RunFeedItem<Value>,
): AsyncGenerator<RunFeedItem<Value>> {
  yield value;
  throw Object.assign(new Error("transient"), { code: 14 });
}

async function reconcileTerminal(runId: string, lastSequence: bigint) {
  return { runId, lastSequence, terminal: true };
}

test("only exact terminal RunUpdate variants stop a watch", () => {
  for (const updateCase of ["result", "failure", "cancellation"] as const) {
    assert.equal(
      isTerminalRunUpdate({ update: { $case: updateCase } }),
      true,
      updateCase,
    );
  }
  for (const updateCase of ["state", "notice", "message"] as const) {
    assert.equal(
      isTerminalRunUpdate({ update: { $case: updateCase } }),
      false,
      updateCase,
    );
  }
  assert.equal(isTerminalRunUpdate({}), false);
  assert.equal(
    generatedTerminalPredicate({
      runId: "run-1",
      sequence: 1n,
      createdAt: undefined,
      update: { $case: "state", value: { status: 0 } },
    }),
    false,
  );
});

test("watch reconnects from the last cursor and removes duplicate delivery", async () => {
  const openedAfter: bigint[] = [];
  let attempt = 0;
  const seen: bigint[] = [];

  for await (const item of watchRun<Value>({
    runId: "run-1",
    open(_runId, afterSequence) {
      openedAfter.push(afterSequence);
      attempt += 1;
      if (attempt === 1) {
        return transientAfter({
          runId: "run-1",
          sequence: 1n,
          value: { terminal: false },
        });
      }
      return items([
        { runId: "run-1", sequence: 1n, value: { terminal: false } },
        { runId: "run-1", sequence: 2n, value: { terminal: true } },
      ]);
    },
    reconcile: reconcileTerminal,
    isTerminal(value) {
      return value.terminal;
    },
    sleep: async () => {},
  })) {
    seen.push(item.sequence);
  }

  assert.deepEqual(openedAfter, [0n, 1n]);
  assert.deepEqual(seen, [1n, 2n]);
});

test("clean EOF at the terminal cursor completes without reconnecting", async () => {
  let attempts = 0;
  const observed: bigint[] = [];
  for await (const item of watchRun<Value>({
    runId: "run-1",
    afterSequence: 9n,
    open(_runId, afterSequence) {
      attempts += 1;
      assert.equal(afterSequence, 9n);
      return items([]);
    },
    async reconcile(runId, lastSequence) {
      assert.equal(runId, "run-1");
      assert.equal(lastSequence, 9n);
      return { runId, lastSequence, terminal: true };
    },
    isTerminal(value) {
      return value.terminal;
    },
    sleep: async () => {
      throw new Error("clean EOF must not sleep or reconnect");
    },
  })) {
    observed.push(item.sequence);
  }
  assert.equal(attempts, 1);
  assert.deepEqual(observed, []);
});

test("watch fails closed on a sequence gap", async () => {
  await assert.rejects(
    async () => {
      for await (const _item of watchRun<Value>({
        runId: "run-1",
        open() {
          return items([
            { runId: "run-1", sequence: 2n, value: { terminal: true } },
          ]);
        },
        reconcile: reconcileTerminal,
        isTerminal(value) {
          return value.terminal;
        },
      })) {
        // Consume the stream.
      }
    },
    RunFeedProtocolError,
  );
});

test("watch rejects zero or timer-overflowing backoff bounds", async () => {
  for (const bounds of [
    { initialBackoffMs: 0, maximumBackoffMs: 1 },
    { initialBackoffMs: 1, maximumBackoffMs: 2_147_483_648 },
  ]) {
    await assert.rejects(
      async () => {
        for await (const _item of watchRun<Value>({
          runId: "run-1",
          open() {
            return items([]);
          },
          reconcile: reconcileTerminal,
          isTerminal(value) {
            return value.terminal;
          },
          ...bounds,
        })) {
          // Consume the stream.
        }
      },
      /backoff/u,
    );
  }
});

test("watch retries only explicitly retryable failures", async () => {
  let attempt = 0;
  const observed: bigint[] = [];

  for await (const item of watchRun<Value>({
    runId: "run-1",
    open() {
      attempt += 1;
      if (attempt === 1) {
        throw Object.assign(new Error("transient"), { code: 14 });
      }
      return items([
        { runId: "run-1", sequence: 1n, value: { terminal: true } },
      ]);
    },
    reconcile: reconcileTerminal,
    isTerminal(value) {
      return value.terminal;
    },
    sleep: async () => {},
  })) {
    observed.push(item.sequence);
  }

  assert.equal(attempt, 2);
  assert.deepEqual(observed, [1n]);
});

test("clean EOF fails closed without exact terminal reconciliation", async () => {
  await assert.rejects(
    async () => {
      for await (const _item of watchRun<Value>({
        runId: "run-1",
        afterSequence: 9n,
        open() {
          return items([]);
        },
        async reconcile(runId) {
          return { runId, lastSequence: 10n, terminal: true };
        },
        isTerminal(value) {
          return value.terminal;
        },
      })) {
        // Consume the stream.
      }
    },
    /terminal at the exact cursor/u,
  );
});
