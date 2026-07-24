import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  CancelRunRequest,
  CommandError,
  ApplyManagedModelConfigurationRequest,
  ConfigureManagedRuntimeRequest,
  ConnectionStatus,
  CreateRunRequest,
  DesktopStatus,
  DesktopReleaseChannel,
  GetRunRequest,
  Interaction,
  ListRunsRequest,
  RespondInteractionRequest,
  Run,
  RunDetails,
  RunPage,
  TerminalContext,
  TerminalEvent,
  TerminalKind,
  OpenTerminalResponse,
  TerminalSignal,
  WatchEvent,
  WatchRunRequest,
  WorkspaceSummary,
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

export function initializeDesktop(): Promise<DesktopStatus> {
  return call("initialize_desktop");
}

export function desktopReleaseChannel(): Promise<DesktopReleaseChannel> {
  return call("desktop_release_channel");
}

export function desktopStatus(): Promise<DesktopStatus> {
  return call("desktop_status");
}

export function addExternalTarget(): Promise<DesktopStatus | null> {
  return call("add_external_target");
}

export function removeExternalTarget(targetId: string): Promise<DesktopStatus> {
  return call("remove_external_target", { targetId });
}

export function connectColossus(targetId?: string): Promise<ConnectionStatus> {
  return call("connect_colossus", { targetId: targetId ?? null });
}

export function connectionStatus(): Promise<ConnectionStatus> {
  return call("connection_status");
}

export function chooseWorkspace(): Promise<WorkspaceSummary | null> {
  return call("choose_workspace");
}

export function configureManagedRuntime(
  request: ConfigureManagedRuntimeRequest,
): Promise<DesktopStatus> {
  return call("configure_managed_runtime", { request });
}

export function applyManagedModelConfiguration(
  request: ApplyManagedModelConfigurationRequest,
): Promise<DesktopStatus> {
  return call("apply_managed_model_configuration", { request });
}

export function restartManagedRuntime(): Promise<DesktopStatus> {
  return call("restart_managed_runtime");
}

export function runManagedSelfTest(): Promise<void> {
  return call("run_managed_self_test");
}

export function selectTarget(targetId: string): Promise<DesktopStatus> {
  return call("select_target", { targetId });
}

export function setTerminalEnabled(enabled: boolean): Promise<DesktopStatus> {
  return call("set_terminal_enabled", { enabled });
}

export function showTerminalWindow(kind: TerminalKind): Promise<void> {
  return call("show_terminal_window", { request: { kind } });
}

export function terminalContext(): Promise<TerminalContext> {
  return call("terminal_context");
}

export function createRun(
  targetId: string,
  request: CreateRunRequest,
): Promise<Run> {
  return call("create_run", { targetId, request });
}

export function getRun(
  targetId: string,
  request: GetRunRequest,
): Promise<RunDetails> {
  return call("get_run", { targetId, request });
}

export function listRuns(
  targetId: string,
  request: ListRunsRequest,
): Promise<RunPage> {
  return call("list_runs", { targetId, request });
}

export async function watchRun(
  targetId: string,
  request: WatchRunRequest,
  handleEvent: (event: WatchEvent) => void,
): Promise<void> {
  const onEvent = new Channel<WatchEvent>();
  onEvent.onmessage = handleEvent;
  await call<void>("watch_run", { targetId, request, onEvent });
}

export function cancelRun(
  targetId: string,
  request: CancelRunRequest,
): Promise<Run> {
  return call("cancel_run", { targetId, request });
}

export function respondInteraction(
  targetId: string,
  request: RespondInteractionRequest,
): Promise<Interaction> {
  return call("respond_interaction", { targetId, request });
}

export async function openTerminal(
  workspaceId: string,
  contextGeneration: number,
  kind: TerminalKind,
  rows: number,
  cols: number,
  handleEvent: (event: TerminalEvent) => void,
): Promise<string> {
  const onEvent = new Channel<TerminalEvent>();
  onEvent.onmessage = handleEvent;
  const response = await call<OpenTerminalResponse>("open_terminal", {
    request: { workspaceId, contextGeneration, kind, rows, cols },
    onEvent,
  });
  return response.sessionId;
}

export function writeTerminal(
  sessionId: string,
  dataBase64: string,
): Promise<void> {
  return call("write_terminal", {
    request: { sessionId, dataBase64 },
  });
}

export function resizeTerminal(
  sessionId: string,
  rows: number,
  cols: number,
): Promise<void> {
  return call("resize_terminal", { request: { sessionId, rows, cols } });
}

export function signalTerminal(
  sessionId: string,
  signal: TerminalSignal,
): Promise<void> {
  return call("signal_terminal", { request: { sessionId, signal } });
}

export function closeTerminal(sessionId: string): Promise<void> {
  return call("close_terminal", { request: { sessionId } });
}
