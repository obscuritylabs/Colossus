import { describe, expect, it, vi } from "vitest";

import {
  MAX_WATCH_RECOVERY_ATTEMPTS,
  TargetRouteRegistry,
  WATCH_RECOVERY_DEADLINE_MS,
  selectedTargetRouteChanged,
  watchDurableRun,
} from "./target-routing";
import type {
  CommandError,
  DesktopStatus,
  Run,
  RunDetails,
  WatchEvent,
} from "./types";

const UNAVAILABLE: CommandError = {
  code: "unavailable",
  message: "Managed Local is restarting.",
  retryable: true,
  outcomeUnknown: false,
  violations: [],
};

function run(
  status: Run["status"],
  lastSequence: number,
  terminal: Run["terminal"] = null,
): Run {
  return {
    runId: "run-1",
    sessionId: "session-1",
    role: "primary",
    mode: "execute",
    status,
    createdAt: "2026-07-21T12:00:00Z",
    updatedAt: "2026-07-21T12:00:01Z",
    startedAt: "2026-07-21T12:00:00Z",
    finishedAt: terminal === null ? null : "2026-07-21T12:00:01Z",
    lastSequence,
    pendingInteractionCount: 0,
    terminal,
    etag: `etag-${lastSequence}`,
    selectedSkills: [],
  };
}

function details(value: Run): RunDetails {
  return { run: value, pendingInteractions: [] };
}

function normalizeError(error: unknown): CommandError {
  return error as CommandError;
}

function status(): DesktopStatus {
  return {
    connection: {
      state: "connected",
      message: "Connected.",
      targetId: "managed-local",
    },
    targets: [
      {
        targetId: "managed-local",
        kind: "managed_local",
        label: "Managed Local",
        state: "ready",
        message: "Ready.",
        selected: true,
        terminalAvailable: true,
        workspace: null,
        failureCode: null,
      },
      {
        targetId: "external-1",
        kind: "external_daemon",
        label: "Fleet node",
        state: "disconnected",
        message: "Unavailable.",
        selected: false,
        terminalAvailable: false,
        workspace: null,
        failureCode: "transport",
      },
    ],
    selectedTargetId: "managed-local",
    managedState: "ready",
    workspace: null,
    provider: {
      configured: true,
      kind: "openai_responses",
      model: "gpt",
    },
    accessProfile: "development",
    terminalEnabled: false,
  };
}

describe("TargetRouteRegistry", () => {
  it("invalidates run bindings and old generations across target switches", () => {
    const routes = new TargetRouteRegistry();
    const local = routes.activate("managed-local", "managed_local");
    expect(routes.bindRun("run-1", local)).toBe(true);
    expect(routes.routeForRun("run-1")).toBe(local);

    const external = routes.activate("external-7", "external_daemon");
    expect(routes.isCurrent(local)).toBe(false);
    expect(routes.isCurrent(external)).toBe(true);
    expect(routes.routeForRun("run-1")).toBeNull();
    expect(routes.bindRun("late-run", local)).toBe(false);
  });

  it("invalidates the same target when its connection generation changes", () => {
    const routes = new TargetRouteRegistry();
    const first = routes.activate("managed-local", "managed_local");
    const firstGeneration = routes.captureGeneration();
    const reconnected = routes.activate("managed-local", "managed_local");

    expect(routes.isCurrent(first)).toBe(false);
    expect(routes.isGenerationCurrent(firstGeneration)).toBe(false);
    expect(routes.isCurrent(reconnected)).toBe(true);
    expect(reconnected.generation).toBeGreaterThan(first.generation);
  });

  it("bounds renderer run bindings", () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    for (let index = 0; index < 520; index += 1) {
      routes.bindRun(`run-${index}`, route);
    }

    expect(routes.routeForRun("run-0")).toBeNull();
    expect(routes.routeForRun("run-519")).toBe(route);
  });
});

describe("selectedTargetRouteChanged", () => {
  it("does not invalidate Work when only another fleet target health changes", () => {
    const previous = status();
    const next = {
      ...previous,
      targets: previous.targets.map((target) =>
        target.targetId === "external-1"
          ? { ...target, state: "ready" as const, failureCode: null }
          : target,
      ),
    };

    expect(selectedTargetRouteChanged(previous, next)).toBe(false);
  });

  it("detects selected target identity and connection-state transitions", () => {
    const previous = status();
    expect(
      selectedTargetRouteChanged(previous, {
        ...previous,
        connection: { ...previous.connection, state: "restarting" },
        targets: previous.targets.map((target) =>
          target.selected
            ? { ...target, state: "restarting" as const }
            : target,
        ),
      }),
    ).toBe(true);
  });
});

describe("watchDurableRun", () => {
  it("refetches durable state and resumes from the persisted cursor", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    const calls: string[] = [];
    const updates: number[] = [];
    const hydrates: number[] = [];
    let watchAttempt = 0;

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 4,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch: async (targetId, runId, cursor, onEvent) => {
        calls.push(`watch:${targetId}:${runId}:${cursor}`);
        watchAttempt += 1;
        if (watchAttempt === 1) {
          onEvent({
            type: "update",
            update: {
              runId,
              sequence: 5,
              createdAt: "2026-07-21T12:00:01Z",
              update: { type: "state", status: "running" },
            },
          });
          throw UNAVAILABLE;
        }
        onEvent({
          type: "update",
          update: {
            runId,
            sequence: 6,
            createdAt: "2026-07-21T12:00:02Z",
            update: {
              type: "result",
              result: {
                output: "done",
                profile: "development",
                model: "test",
                elapsedSeconds: 1,
              },
            },
          },
        });
        onEvent({ type: "complete", runId });
      },
      getRun: async (targetId, runId) => {
        calls.push(`get:${targetId}:${runId}`);
        return details(run("running", 5));
      },
      normalizeError,
      canRecover: (error, candidate) =>
        candidate.kind === "managed_local" &&
        error.code === "unavailable" &&
        error.retryable,
      onUpdate: (update) => updates.push(update.sequence),
      onHydrate: (snapshot) => hydrates.push(snapshot.run.lastSequence),
      delay: async () => undefined,
    });

    expect(result).toEqual({ type: "complete", cursor: 6 });
    expect(calls).toEqual([
      "watch:managed-local:run-1:4",
      "get:managed-local:run-1",
      "watch:managed-local:run-1:5",
    ]);
    expect(updates).toEqual([5, 6]);
    expect(hydrates).toEqual([5]);
  });

  it("discards an in-flight watch after a target generation changes", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    let finishWatch: (() => void) | undefined;
    const onUpdate = vi.fn();
    const pending = watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 0,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch: (_targetId, _runId, _cursor, onEvent) =>
        new Promise<void>((resolve) => {
          finishWatch = () => {
            const event: WatchEvent = {
              type: "update",
              update: {
                runId: "run-1",
                sequence: 1,
                createdAt: "2026-07-21T12:00:01Z",
                update: { type: "state", status: "running" },
              },
            };
            onEvent(event);
            resolve();
          };
        }),
      getRun: async () => details(run("running", 0)),
      normalizeError,
      canRecover: () => true,
      onUpdate,
      onHydrate: vi.fn(),
      delay: async () => undefined,
    });

    routes.activate("external-1", "external_daemon");
    finishWatch?.();

    await expect(pending).resolves.toEqual({ type: "stale", cursor: 0 });
    expect(onUpdate).not.toHaveBeenCalled();
  });

  it("bounds recovery without replaying any effect request", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    const watch = vi.fn(async (_targetId: string) => {
      throw UNAVAILABLE;
    });
    const getRun = vi.fn(async (_targetId: string) =>
      details(run("running", 0)),
    );
    const delays: number[] = [];

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 0,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch,
      getRun,
      normalizeError,
      canRecover: (error) => error.retryable,
      onUpdate: vi.fn(),
      onHydrate: vi.fn(),
      delay: async (milliseconds) => {
        delays.push(milliseconds);
      },
      recoveryDeadlineMs: 750,
      maxRecoveryAttempts: MAX_WATCH_RECOVERY_ATTEMPTS,
    });

    expect(result).toEqual({ type: "error", cursor: 0, error: UNAVAILABLE });
    expect(watch).toHaveBeenCalledTimes(3);
    expect(getRun).toHaveBeenCalledTimes(2);
    expect(delays).toEqual([250, 250, 250]);
    expect(
      [...watch.mock.calls, ...getRun.mock.calls].every(
        (call) => call[0] === "managed-local",
      ),
    ).toBe(true);
  });

  it("does not let duplicate replay reset the bounded recovery window", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    let clock = 0;
    let attempts = 0;
    const fatal: CommandError = {
      ...UNAVAILABLE,
      code: "fatal",
      retryable: false,
    };
    const watch = vi.fn(
      async (
        _targetId: string,
        runId: string,
        _cursor: number,
        onEvent: (event: WatchEvent) => void,
      ) => {
        attempts += 1;
        onEvent({
          type: "update",
          update: {
            runId,
            sequence: 1,
            createdAt: "2026-07-21T12:00:01Z",
            update: { type: "state", status: "running" },
          },
        });
        throw attempts >= 5 ? fatal : UNAVAILABLE;
      },
    );

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 1,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch,
      getRun: async () => details(run("running", 1)),
      normalizeError,
      canRecover: (error) => error.retryable,
      onUpdate: vi.fn(),
      onHydrate: vi.fn(),
      delay: async (milliseconds) => {
        clock += milliseconds;
      },
      now: () => clock,
      recoveryDeadlineMs: 750,
    });

    expect(result).toEqual({ type: "error", cursor: 1, error: UNAVAILABLE });
    expect(watch).toHaveBeenCalledTimes(3);
  });

  it("keeps a cursor-preserving read-only recovery window open for a native restart", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    let clock = 0;
    let watchAttempt = 0;
    const updates: number[] = [];
    const getRun = vi.fn(async () => {
      if (clock < 30_000) {
        throw UNAVAILABLE;
      }
      return details(run("running", 7));
    });

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 7,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch: async (_targetId, runId, cursor, onEvent) => {
        watchAttempt += 1;
        if (watchAttempt === 1) {
          throw UNAVAILABLE;
        }
        expect(cursor).toBe(7);
        onEvent({
          type: "update",
          update: {
            runId,
            sequence: 8,
            createdAt: "2026-07-21T12:00:08Z",
            update: { type: "state", status: "running" },
          },
        });
        onEvent({ type: "complete", runId });
      },
      getRun,
      normalizeError,
      canRecover: (error, candidate) =>
        candidate.kind === "managed_local" && error.retryable,
      onUpdate: (update) => updates.push(update.sequence),
      onHydrate: vi.fn(),
      delay: async (milliseconds) => {
        clock += milliseconds;
      },
      now: () => clock,
      recoveryDeadlineMs: WATCH_RECOVERY_DEADLINE_MS,
    });

    expect(result).toEqual({ type: "complete", cursor: 8 });
    expect(clock).toBeGreaterThanOrEqual(30_000);
    expect(clock).toBeLessThan(WATCH_RECOVERY_DEADLINE_MS);
    expect(getRun.mock.calls.length).toBeGreaterThan(3);
    expect(updates).toEqual([8]);
  });

  it("cancels recovery before another read when the route generation changes", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("managed-local", "managed_local");
    const getRun = vi.fn(async () => details(run("running", 0)));

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 0,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch: async () => {
        throw UNAVAILABLE;
      },
      getRun,
      normalizeError,
      canRecover: () => true,
      onUpdate: vi.fn(),
      onHydrate: vi.fn(),
      delay: async () => {
        routes.activate("external-1", "external_daemon");
      },
    });

    expect(result).toEqual({ type: "stale", cursor: 0 });
    expect(getRun).not.toHaveBeenCalled();
  });

  it("does not recover external targets or non-retryable failures", async () => {
    const routes = new TargetRouteRegistry();
    const route = routes.activate("external-1", "external_daemon");
    const getRun = vi.fn(async () => details(run("running", 0)));

    const result = await watchDurableRun({
      route,
      runId: "run-1",
      afterSequence: 0,
      isCurrent: (candidate) => routes.isCurrent(candidate),
      watch: async () => {
        throw UNAVAILABLE;
      },
      getRun,
      normalizeError,
      canRecover: (_error, candidate) => candidate.kind === "managed_local",
      onUpdate: vi.fn(),
      onHydrate: vi.fn(),
      delay: async () => undefined,
    });

    expect(result).toEqual({ type: "error", cursor: 0, error: UNAVAILABLE });
    expect(getRun).not.toHaveBeenCalled();
  });
});
