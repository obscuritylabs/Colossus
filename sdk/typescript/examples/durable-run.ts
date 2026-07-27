/**
 * Application core for one durable Colossus run.
 *
 * Compose this with createSecureGrpcClient() in trusted native/server code. The
 * bearer credential must come from application-protected storage and must never be
 * placed in argv, environment variables, descriptors, logs, or renderer memory.
 */

import { randomUUID } from "node:crypto";

import type { ServiceError } from "@grpc/grpc-js";

import {
  decodeColossusRpcError,
  isTerminalRunUpdate,
  watchRun,
} from "../src/index.js";
import {
  type AgentRunServiceClient,
  type CreateRunResponse,
  type GetRunResponse,
  type Interaction,
  type RespondInteractionRequest,
  type RespondInteractionResponse,
  RunMode,
  type RunUpdate,
  type WatchRunResponse,
} from "../src/gen/colossus/api/v1alpha1/agent_run.js";

export interface DurableRunResult {
  readonly runId: string;
  readonly output: string;
  readonly toolNames: readonly string[];
}

export type InteractionHandler = (
  interaction: Interaction,
) => Promise<RespondInteractionRequest | undefined>;

export class DurableRunFailed extends Error {
  public readonly reason: string;
  public readonly recoverable: boolean;
  public readonly outcomeCertainty: number;
  public readonly httpStatus: number | undefined;
  public readonly retryAfterMs: bigint | undefined;

  public constructor(failure: NonNullable<RunUpdate["update"]> & {
    readonly $case: "failure";
  }) {
    const detail = failure.value.failure;
    super(detail?.message ?? "run failed without released failure detail");
    this.name = "DurableRunFailed";
    this.reason = detail?.reason ?? "run.failure_detail_missing";
    this.recoverable = detail?.recoverable ?? false;
    this.outcomeCertainty = detail?.outcomeCertainty ?? 0;
    this.httpStatus = detail?.httpStatus;
    this.retryAfterMs = detail?.retryAfterMs;
  }
}

export async function runPrompt(
  client: AgentRunServiceClient,
  prompt: string,
  options: {
    readonly mode?: RunMode;
    readonly maxTurns?: number;
    readonly signal?: AbortSignal;
    readonly handleInteraction?: InteractionHandler;
  } = {},
): Promise<DurableRunResult> {
  let created: CreateRunResponse;
  try {
    created = await unary<CreateRunResponse>((callback) =>
      client.createRun(
        {
          input: [{ content: { $case: "text", value: { text: prompt } } }],
          role: "primary",
          mode: options.mode ?? RunMode.RUN_MODE_EXECUTE,
          selectedSkills: [],
          maxTurns: options.maxTurns ?? 12,
          idempotencyKey: `sdk-example-create-${randomUUID()}`,
        },
        callback,
      ),
    );
  } catch (error) {
    const detail =
      error !== null &&
      typeof error === "object" &&
      "code" in error &&
      "metadata" in error
        ? decodeColossusRpcError(
            error as Pick<ServiceError, "code" | "metadata">,
          )
        : undefined;
    if (detail !== undefined) {
      throw new Error(
        `CreateRun failed: ${detail.reason}; retryable=${String(detail.retryable)}; ` +
          `outcome=${detail.outcomeCertainty}`,
        { cause: error },
      );
    }
    throw error;
  }
  const runId = created.run?.runId;
  if (runId === undefined || runId.length === 0) {
    throw new Error("CreateRun returned no durable run identity");
  }

  const toolNames = new Set<string>();
  const feed = watchRun<RunUpdate>({
    runId,
    ...(options.signal === undefined ? {} : { signal: options.signal }),
    open: async function* (watchedRunId, afterSequence, signal) {
      const stream = client.watchRun({
        runId: watchedRunId,
        afterSequence,
      });
      const cancel = (): void => stream.cancel();
      signal?.addEventListener("abort", cancel, { once: true });
      try {
        for await (const response of stream as AsyncIterable<WatchRunResponse>) {
          if (response.update === undefined) {
            throw new Error("WatchRun returned an empty response");
          }
          yield {
            runId: response.update.runId,
            sequence: response.update.sequence,
            value: response.update,
          };
        }
      } finally {
        signal?.removeEventListener("abort", cancel);
      }
    },
    reconcile: async (watchedRunId) => {
      const response = await unary<GetRunResponse>((callback) =>
        client.getRun({ runId: watchedRunId }, callback),
      );
      if (response.run === undefined) {
        throw new Error("GetRun returned no run during watch reconciliation");
      }
      return {
        runId: response.run.runId,
        lastSequence: response.run.lastSequence,
        terminal: response.run.terminal !== undefined,
      };
    },
    isTerminal: isTerminalRunUpdate,
  });

  for await (const item of feed) {
    const update = item.value.update;
    switch (update?.$case) {
      case "toolActivity":
        toolNames.add(update.value.toolName);
        break;
      case "interaction": {
        if (!update.value.respondableByCaller) {
          break;
        }
        const request = await options.handleInteraction?.(update.value);
        if (request === undefined) {
          throw new Error(
            "run is waiting for an interaction; supply handleInteraction and resume " +
              "from the last durable cursor",
          );
        }
        await unary<RespondInteractionResponse>((callback) =>
          client.respondInteraction(request, callback),
        );
        break;
      }
      case "result":
        return {
          runId,
          output: update.value.output,
          toolNames: [...toolNames].sort(),
        };
      case "failure":
        throw new DurableRunFailed(update);
      case "cancellation":
        throw new Error(`run cancelled: ${update.value.message}`);
      default:
        break;
    }
  }
  throw new Error("run watch ended without an exact terminal update");
}

export async function denyApproval(
  interaction: Interaction,
): Promise<RespondInteractionRequest | undefined> {
  if (interaction.content?.$case !== "approval") {
    return undefined;
  }
  return {
    runId: interaction.runId,
    interactionId: interaction.interactionId,
    etag: interaction.etag,
    idempotencyKey: `sdk-example-interaction-${randomUUID()}`,
    response: {
      $case: "approvalAnswer",
      value: {
        approved: false,
        requestHash: interaction.content.value.requestHash,
      },
    },
  };
}

function unary<Response>(
  invoke: (callback: (error: ServiceError | null, response: Response) => void) => unknown,
): Promise<Response> {
  return new Promise((resolve, reject) => {
    invoke((error, response) => {
      if (error !== null) {
        reject(error);
      } else {
        resolve(response);
      }
    });
  });
}
