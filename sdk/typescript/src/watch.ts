export interface RunFeedItem<Value> {
  readonly runId: string;
  readonly sequence: bigint;
  readonly value: Value;
}

export type OpenRunWatch<Value> = (
  runId: string,
  afterSequence: bigint,
  signal: AbortSignal | undefined,
) => AsyncIterable<RunFeedItem<Value>>;

export interface RunWatchReconciliation {
  readonly runId: string;
  readonly lastSequence: bigint;
  readonly terminal: boolean;
}

export interface RunWatchOptions<Value> {
  readonly runId: string;
  readonly afterSequence?: bigint;
  readonly signal?: AbortSignal;
  readonly open: OpenRunWatch<Value>;
  readonly reconcile: (
    runId: string,
    lastSequence: bigint,
    signal: AbortSignal | undefined,
  ) => Promise<RunWatchReconciliation>;
  readonly isTerminal: (value: Value) => boolean;
  readonly isRetryable?: (error: unknown) => boolean;
  readonly initialBackoffMs?: number;
  readonly maximumBackoffMs?: number;
  readonly sleep?: (milliseconds: number, signal: AbortSignal | undefined) => Promise<void>;
}

export type RunUpdateCase =
  | "state"
  | "outputDelta"
  | "reasoningSummary"
  | "toolActivity"
  | "usage"
  | "interaction"
  | "message"
  | "notice"
  | "result"
  | "failure"
  | "cancellation";

export interface RunUpdateOneof {
  readonly update?:
    | {
        readonly $case?: RunUpdateCase;
      }
    | undefined;
}

export class RunFeedProtocolError extends Error {
  public constructor(message: string) {
    super(message);
    this.name = "RunFeedProtocolError";
  }
}

/**
 * Returns true only for the three terminal variants in the v1alpha1 RunUpdate
 * oneof. Lifecycle state notifications are historical updates, not completion
 * markers.
 */
export function isTerminalRunUpdate(update: RunUpdateOneof): boolean {
  const updateCase = update.update?.$case;
  return (
    updateCase === "result" ||
    updateCase === "failure" ||
    updateCase === "cancellation"
  );
}

function grpcCode(error: unknown): number | undefined {
  if (error === null || typeof error !== "object" || !("code" in error)) {
    return undefined;
  }
  const code = (error as { code?: unknown }).code;
  return typeof code === "number" ? code : undefined;
}

function defaultRetryable(error: unknown): boolean {
  // UNAVAILABLE is safe to retry for this read-only cursor stream.
  return grpcCode(error) === 14;
}

function defaultSleep(
  milliseconds: number,
  signal: AbortSignal | undefined,
): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted === true) {
      reject(signal.reason);
      return;
    }
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(signal?.reason);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function isAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

/**
 * Replays and tails one durable run feed.
 *
 * Only the read-only watch call is retried. Exact at-least-once replays are dropped,
 * while a gap fails closed so an application never silently presents incomplete output.
 */
export async function* watchRun<Value>(
  options: RunWatchOptions<Value>,
): AsyncGenerator<RunFeedItem<Value>, void, undefined> {
  if (options.runId.length === 0) {
    throw new TypeError("runId must not be empty");
  }
  if (typeof options.reconcile !== "function") {
    throw new TypeError("watch terminal reconciliation is required");
  }

  let cursor = options.afterSequence ?? 0n;
  if (cursor < 0n) {
    throw new TypeError("afterSequence must be non-negative");
  }

  const isRetryable = options.isRetryable ?? defaultRetryable;
  const sleep = options.sleep ?? defaultSleep;
  const initialBackoff = options.initialBackoffMs ?? 250;
  const maximumBackoff = options.maximumBackoffMs ?? 5_000;
  if (
    !Number.isSafeInteger(initialBackoff) ||
    !Number.isSafeInteger(maximumBackoff) ||
    initialBackoff <= 0 ||
    maximumBackoff > 2_147_483_647 ||
    maximumBackoff < initialBackoff
  ) {
    throw new TypeError("watch backoff bounds are invalid");
  }
  let backoff = initialBackoff;

  while (!isAborted(options.signal)) {
    try {
      const stream = options.open(options.runId, cursor, options.signal);
      for await (const item of stream) {
        if (item.runId !== options.runId) {
          throw new RunFeedProtocolError("watch stream returned a different run_id");
        }
        if (item.sequence <= cursor) {
          continue;
        }
        if (item.sequence !== cursor + 1n) {
          throw new RunFeedProtocolError("watch stream contains a sequence gap");
        }

        cursor = item.sequence;
        backoff = initialBackoff;
        yield item;
        if (options.isTerminal(item.value)) {
          return;
        }
      }

      const reconciled = await options.reconcile(
        options.runId,
        cursor,
        options.signal,
      );
      if (
        reconciled.runId !== options.runId ||
        reconciled.lastSequence !== cursor ||
        !reconciled.terminal
      ) {
        throw new RunFeedProtocolError(
          "clean watch EOF was not terminal at the exact cursor",
        );
      }
      return;
    } catch (error) {
      if (isAborted(options.signal)) {
        return;
      }
      if (error instanceof RunFeedProtocolError || !isRetryable(error)) {
        throw error;
      }
      await sleep(backoff, options.signal);
      backoff = Math.min(maximumBackoff, Math.max(1, backoff * 2));
    }
  }
}
