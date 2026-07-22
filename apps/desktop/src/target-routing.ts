import type {
  CommandError,
  DesktopStatus,
  RunDetails,
  RunUpdate,
  RuntimeTargetKind,
  WatchEvent,
} from "./types";

export const MAX_WATCH_RECOVERY_ATTEMPTS = 16;
export const WATCH_RECOVERY_DEADLINE_MS = 45_000;

const WATCH_RECOVERY_DELAYS_MS = [
  250, 500, 1_000, 2_000, 4_000, 5_000,
] as const;
const WATCH_RECOVERY_ROUTE_CHECK_MS = 250;
const WATCH_RECOVERY_STABILITY_MS = 5_000;
const MAX_ROUTED_RUNS = 512;

export interface TargetRoute {
  readonly targetId: string;
  readonly kind: RuntimeTargetKind;
  readonly generation: number;
}

/**
 * Renderer-local routing authority. Native target IDs remain opaque; the
 * generation prevents an async result from being projected into a later
 * connection that happens to select the same target again.
 */
export class TargetRouteRegistry {
  private generation = 0;
  private current: TargetRoute | null = null;
  private readonly runs = new Map<string, TargetRoute>();

  activate(targetId: string, kind: RuntimeTargetKind): TargetRoute {
    this.generation += 1;
    const route = Object.freeze({
      targetId,
      kind,
      generation: this.generation,
    });
    this.current = route;
    this.runs.clear();
    return route;
  }

  invalidate(): void {
    this.generation += 1;
    this.current = null;
    this.runs.clear();
  }

  capture(): TargetRoute | null {
    return this.current;
  }

  captureGeneration(): number {
    return this.generation;
  }

  isGenerationCurrent(generation: number): boolean {
    return this.generation === generation;
  }

  isCurrent(route: TargetRoute): boolean {
    return (
      this.current?.generation === route.generation &&
      this.current.targetId === route.targetId
    );
  }

  bindRun(runId: string, route: TargetRoute): boolean {
    if (!this.isCurrent(route)) {
      return false;
    }
    this.runs.delete(runId);
    this.runs.set(runId, route);
    while (this.runs.size > MAX_ROUTED_RUNS) {
      const oldest = this.runs.keys().next();
      if (oldest.done) {
        break;
      }
      this.runs.delete(oldest.value);
    }
    return true;
  }

  bindRuns(runIds: Iterable<string>, route: TargetRoute): boolean {
    if (!this.isCurrent(route)) {
      return false;
    }
    for (const runId of runIds) {
      this.bindRun(runId, route);
    }
    return true;
  }

  routeForRun(runId: string): TargetRoute | null {
    return this.runs.get(runId) ?? null;
  }
}

interface SelectedTargetRouteState {
  selectedTargetId: string | null;
  connectionTargetId: string | null;
  connectionState: DesktopStatus["connection"]["state"];
  targetKind: RuntimeTargetKind | null;
  targetState: DesktopStatus["targets"][number]["state"] | null;
}

function selectedTargetRouteState(
  status: DesktopStatus,
): SelectedTargetRouteState {
  const selected = status.targets.find(
    (target) => target.targetId === status.selectedTargetId,
  );
  return {
    selectedTargetId: status.selectedTargetId,
    connectionTargetId: status.connection.targetId,
    connectionState: status.connection.state,
    targetKind: selected?.kind ?? null,
    targetState: selected?.state ?? null,
  };
}

/** Fleet-only health changes must not invalidate the selected work route. */
export function selectedTargetRouteChanged(
  previous: DesktopStatus,
  next: DesktopStatus,
): boolean {
  const left = selectedTargetRouteState(previous);
  const right = selectedTargetRouteState(next);
  return (
    left.selectedTargetId !== right.selectedTargetId ||
    left.connectionTargetId !== right.connectionTargetId ||
    left.connectionState !== right.connectionState ||
    left.targetKind !== right.targetKind ||
    left.targetState !== right.targetState
  );
}

export type DurableWatchResult =
  | { type: "complete"; cursor: number }
  | { type: "stale"; cursor: number }
  | { type: "error"; cursor: number; error: CommandError };

export interface DurableWatchOptions {
  route: TargetRoute;
  runId: string;
  afterSequence: number;
  isCurrent: (route: TargetRoute) => boolean;
  watch: (
    targetId: string,
    runId: string,
    afterSequence: number,
    onEvent: (event: WatchEvent) => void,
  ) => Promise<void>;
  getRun: (targetId: string, runId: string) => Promise<RunDetails>;
  normalizeError: (error: unknown) => CommandError;
  canRecover: (error: CommandError, route: TargetRoute) => boolean;
  onUpdate: (update: RunUpdate) => void;
  onHydrate: (details: RunDetails) => void;
  delay?: (milliseconds: number) => Promise<void>;
  now?: () => number;
  recoveryDeadlineMs?: number;
  maxRecoveryAttempts?: number;
}

interface RecoveryWindow {
  readonly startedAt: number;
  scheduledDelayMs: number;
  attempts: number;
}

function protocolError(message: string): CommandError {
  return {
    code: "desktop_watch_protocol",
    message,
    retryable: false,
    outcomeUnknown: false,
    violations: [],
  };
}

function defaultDelay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function defaultNow(): number {
  return globalThis.performance?.now() ?? Date.now();
}

function nonNegativeInteger(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : fallback;
}

function elapsedSince(startedAt: number, now: () => number): number {
  const elapsed = now() - startedAt;
  return Number.isFinite(elapsed) ? Math.max(0, elapsed) : 0;
}

function recoveryElapsed(window: RecoveryWindow, now: () => number): number {
  return Math.max(window.scheduledDelayMs, elapsedSince(window.startedAt, now));
}

function recoveryDelay(attempt: number): number {
  return (
    WATCH_RECOVERY_DELAYS_MS[
      Math.min(attempt, WATCH_RECOVERY_DELAYS_MS.length) - 1
    ] ??
    WATCH_RECOVERY_DELAYS_MS.at(-1) ??
    1_000
  );
}

/**
 * Re-establishes only read operations after a managed transport interruption.
 * Create, cancel, and interaction responses are deliberately outside this loop.
 */
export async function watchDurableRun(
  options: DurableWatchOptions,
): Promise<DurableWatchResult> {
  const maxRecoveryAttempts = nonNegativeInteger(
    options.maxRecoveryAttempts ?? MAX_WATCH_RECOVERY_ATTEMPTS,
    MAX_WATCH_RECOVERY_ATTEMPTS,
  );
  const recoveryDeadlineMs = nonNegativeInteger(
    options.recoveryDeadlineMs ?? WATCH_RECOVERY_DEADLINE_MS,
    WATCH_RECOVERY_DEADLINE_MS,
  );
  const delay = options.delay ?? defaultDelay;
  const now = options.now ?? defaultNow;
  let cursor = Math.max(0, Math.trunc(options.afterSequence));
  let recoveryWindow: RecoveryWindow | null = null;

  while (options.isCurrent(options.route)) {
    let eventFailure: CommandError | null = null;
    let watchActivity = false;
    const watchStartedAt = now();
    try {
      await options.watch(
        options.route.targetId,
        options.runId,
        cursor,
        (event) => {
          if (!options.isCurrent(options.route) || eventFailure !== null) {
            return;
          }
          if (event.type === "error") {
            eventFailure = event.error;
            return;
          }
          if (event.type === "complete") {
            if (event.runId !== options.runId) {
              eventFailure = protocolError(
                "The run watch completed for a different run.",
              );
            } else {
              watchActivity = true;
            }
            return;
          }
          const update = event.update;
          if (update.runId !== options.runId) {
            eventFailure = protocolError(
              "The run watch returned an update for a different run.",
            );
            return;
          }
          if (update.sequence <= cursor) {
            return;
          }
          if (update.sequence !== cursor + 1) {
            eventFailure = protocolError(
              "The run watch returned a non-contiguous update.",
            );
            return;
          }
          cursor = update.sequence;
          watchActivity = true;
          options.onUpdate(update);
        },
      );
    } catch (error: unknown) {
      eventFailure ??= options.normalizeError(error);
    }

    if (!options.isCurrent(options.route)) {
      return { type: "stale", cursor };
    }
    if (
      recoveryWindow !== null &&
      (watchActivity ||
        elapsedSince(watchStartedAt, now) >= WATCH_RECOVERY_STABILITY_MS)
    ) {
      recoveryWindow = null;
    }
    if (eventFailure === null) {
      return { type: "complete", cursor };
    }

    let failure = eventFailure;
    let reconciled = false;
    if (options.canRecover(failure, options.route)) {
      recoveryWindow ??= {
        startedAt: now(),
        scheduledDelayMs: 0,
        attempts: 0,
      };
    }
    while (options.canRecover(failure, options.route)) {
      const window = recoveryWindow;
      if (
        window === null ||
        window.attempts >= maxRecoveryAttempts ||
        recoveryElapsed(window, now) >= recoveryDeadlineMs
      ) {
        break;
      }

      window.attempts += 1;
      let remainingDelay = Math.min(
        recoveryDelay(window.attempts),
        recoveryDeadlineMs - recoveryElapsed(window, now),
      );
      while (remainingDelay > 0) {
        const slice = Math.min(remainingDelay, WATCH_RECOVERY_ROUTE_CHECK_MS);
        await delay(slice);
        window.scheduledDelayMs += slice;
        remainingDelay -= slice;
        if (!options.isCurrent(options.route)) {
          return { type: "stale", cursor };
        }
      }
      if (
        recoveryElapsed(window, now) > recoveryDeadlineMs ||
        !options.isCurrent(options.route)
      ) {
        break;
      }

      try {
        const details = await options.getRun(
          options.route.targetId,
          options.runId,
        );
        if (!options.isCurrent(options.route)) {
          return { type: "stale", cursor };
        }
        if (details.run.runId !== options.runId) {
          return {
            type: "error",
            cursor,
            error: protocolError(
              "The run summary returned a different run during watch recovery.",
            ),
          };
        }
        if (details.run.lastSequence < cursor) {
          return {
            type: "error",
            cursor,
            error: protocolError(
              "The run summary regressed behind the durable watch cursor.",
            ),
          };
        }

        options.onHydrate(details);
        if (!options.isCurrent(options.route)) {
          return { type: "stale", cursor };
        }
        if (
          details.run.terminal !== null &&
          details.run.lastSequence === cursor
        ) {
          return { type: "complete", cursor };
        }
        reconciled = true;
        break;
      } catch (error: unknown) {
        failure = options.normalizeError(error);
      }
    }

    if (!reconciled) {
      return { type: "error", cursor, error: failure };
    }
  }

  return { type: "stale", cursor };
}
