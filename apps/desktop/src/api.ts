import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  CancelRunRequest,
  CommandError,
  ConnectionStatus,
  CreateRunRequest,
  GetRunRequest,
  Interaction,
  ListRunsRequest,
  RespondInteractionRequest,
  Run,
  RunDetails,
  RunPage,
  WatchEvent,
  WatchRunRequest,
} from "./types";

const FALLBACK_ERROR: CommandError = {
  code: "desktop_request_failed",
  message: "The desktop request failed. Retry after checking the connection.",
  retryable: true,
  outcomeUnknown: false,
  violations: [],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function normalizeViolations(value: unknown): CommandError["violations"] {
  if (!Array.isArray(value)) {
    return [];
  }

  return value.flatMap((entry) => {
    if (!isRecord(entry)) {
      return [];
    }
    const { field, description } = entry;
    return typeof field === "string" && typeof description === "string"
      ? [{ field, description }]
      : [];
  });
}

export function normalizeCommandError(value: unknown): CommandError {
  if (!isRecord(value)) {
    return FALLBACK_ERROR;
  }

  const { code, message, retryable, outcomeUnknown, violations } = value;
  if (
    typeof code !== "string" ||
    typeof message !== "string" ||
    typeof retryable !== "boolean" ||
    typeof outcomeUnknown !== "boolean"
  ) {
    return FALLBACK_ERROR;
  }

  return {
    code,
    message,
    retryable,
    outcomeUnknown,
    violations: normalizeViolations(violations),
  };
}

export class CommandFailure extends Error {
  readonly detail: CommandError;

  constructor(detail: CommandError) {
    super(detail.message);
    this.name = "CommandFailure";
    this.detail = detail;
  }
}

async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    throw new CommandFailure(normalizeCommandError(error));
  }
}

export function connectColossus(): Promise<ConnectionStatus> {
  return call("connect_colossus");
}

export function connectionStatus(): Promise<ConnectionStatus> {
  return call("connection_status");
}

export function createRun(request: CreateRunRequest): Promise<Run> {
  return call("create_run", { request });
}

export function getRun(request: GetRunRequest): Promise<RunDetails> {
  return call("get_run", { request });
}

export function listRuns(request: ListRunsRequest): Promise<RunPage> {
  return call("list_runs", { request });
}

export async function watchRun(
  request: WatchRunRequest,
  handleEvent: (event: WatchEvent) => void,
): Promise<void> {
  const onEvent = new Channel<WatchEvent>();
  onEvent.onmessage = handleEvent;
  await call<void>("watch_run", { request, onEvent });
}

export function cancelRun(request: CancelRunRequest): Promise<Run> {
  return call("cancel_run", { request });
}

export function respondInteraction(
  request: RespondInteractionRequest,
): Promise<Interaction> {
  return call("respond_interaction", { request });
}
