import type {
  CommandError,
  ConnectionState,
  Interaction,
  Run,
  RunDetails,
  RunTerminal,
  RunUpdate,
  TokenUsage,
} from "./types";
import { isTerminalStatus } from "./types";

export const MAX_FEED_ITEMS = 200;
export const MAX_FEED_TEXT_CHARACTERS = 65_536;
export const MAX_OUTPUT_CHARACTERS = 2_000_000;
export const MAX_TURNS = 100;
export const MAX_PROMPT_BYTES = 65_536;
export const MAX_CACHED_RUN_VIEWS = 12;
export const MAX_RECENT_RUNS = 200;
export const MAX_CACHED_IDEMPOTENCY_ATTEMPTS = 256;
const MAX_TRACKED_SEQUENCES = 512;
const MAX_MESSAGE_CONTENT_PARTS = 64;
const OUTPUT_OMISSION = "[Earlier output omitted from this view]\n";
const TEXT_OMISSION = "\n[Content truncated in this desktop view]";

export type StreamState = "idle" | "watching" | "complete" | "error";

export interface RunView {
  run: Run;
  /** Volatile renderer-only create input; never hydrated from daemon state. */
  localPrompt: string | null;
  output: string;
  updates: RunUpdate[];
  seenSequences: ReadonlySet<number>;
  lastSequence: number;
  pendingInteractions: Interaction[];
  usage: TokenUsage | null;
  streamState: StreamState;
  streamError: CommandError | null;
}

export interface ChatState {
  activeRunId: string | null;
  views: ReadonlyMap<string, RunView>;
  recentRuns: Run[];
  nextPageToken: string;
}

export type ChatAction =
  | { type: "select_run"; runId: string | null }
  | { type: "upsert_run"; run: Run }
  | { type: "record_local_prompt"; runId: string; prompt: string }
  | { type: "hydrate_run"; details: RunDetails }
  | { type: "ingest_update"; update: RunUpdate }
  | { type: "watch_started"; runId: string }
  | { type: "watch_complete"; runId: string }
  | { type: "watch_error"; runId: string; error: CommandError }
  | { type: "interaction_resolved"; interaction: Interaction }
  | { type: "replace_recent"; runs: Run[]; nextPageToken: string }
  | { type: "append_recent"; runs: Run[]; nextPageToken: string };

export const initialChatState: ChatState = {
  activeRunId: null,
  views: new Map(),
  recentRuns: [],
  nextPageToken: "",
};

function terminalOutput(terminal: RunTerminal | null): string {
  return terminal?.type === "result"
    ? boundedOutput(terminal.result.output)
    : "";
}

function boundedFeedText(
  value: string,
  maxCharacters = MAX_FEED_TEXT_CHARACTERS,
): string {
  if (value.length <= maxCharacters) {
    return value;
  }
  if (maxCharacters <= TEXT_OMISSION.length) {
    return TEXT_OMISSION.slice(0, maxCharacters);
  }
  return value.slice(0, maxCharacters - TEXT_OMISSION.length) + TEXT_OMISSION;
}

function compactRun(run: Run): Run {
  const normalizedRun = isTerminalStatus(run.status)
    ? { ...run, pendingInteractionCount: 0 }
    : run;
  if (normalizedRun.terminal === null) {
    return normalizedRun;
  }
  switch (normalizedRun.terminal.type) {
    case "result":
      return {
        ...normalizedRun,
        terminal: {
          type: "result",
          result: {
            ...normalizedRun.terminal.result,
            output: "",
            profile: boundedFeedText(
              normalizedRun.terminal.result.profile,
              512,
            ),
            model: boundedFeedText(normalizedRun.terminal.result.model, 512),
          },
        },
      };
    case "failure":
      return {
        ...normalizedRun,
        terminal: {
          type: "failure",
          failure: {
            ...normalizedRun.terminal.failure,
            reason: boundedFeedText(normalizedRun.terminal.failure.reason, 512),
            message: boundedFeedText(normalizedRun.terminal.failure.message),
          },
        },
      };
    case "cancellation":
      return {
        ...normalizedRun,
        terminal: {
          type: "cancellation",
          cancellation: {
            ...normalizedRun.terminal.cancellation,
            message: boundedFeedText(
              normalizedRun.terminal.cancellation.message,
            ),
          },
        },
      };
  }
}

function boundedOutput(output: string): string {
  if (output.length <= MAX_OUTPUT_CHARACTERS) {
    return output;
  }
  return (
    OUTPUT_OMISSION +
    output.slice(-(MAX_OUTPUT_CHARACTERS - OUTPUT_OMISSION.length))
  );
}

function newView(run: Run, pendingInteractions: Interaction[] = []): RunView {
  const retainedInteractions = isTerminalStatus(run.status)
    ? []
    : pendingInteractions;
  return {
    run: compactRun(run),
    localPrompt: null,
    output: terminalOutput(run.terminal),
    updates: [],
    seenSequences: new Set(),
    lastSequence: 0,
    pendingInteractions: retainedInteractions,
    usage: null,
    streamState: "idle",
    streamError: null,
  };
}

function updateViewMap(
  views: ReadonlyMap<string, RunView>,
  runId: string,
  update: (view: RunView) => RunView,
): ReadonlyMap<string, RunView> {
  const current = views.get(runId);
  if (current === undefined) {
    return views;
  }
  const next = new Map(views);
  next.set(runId, update(current));
  return next;
}

function trimViewCache(
  views: ReadonlyMap<string, RunView>,
  activeRunId: string | null,
  protectedRunId?: string,
): ReadonlyMap<string, RunView> {
  if (views.size <= MAX_CACHED_RUN_VIEWS) {
    return views;
  }

  const next = new Map(views);
  for (const [runId, view] of next) {
    if (next.size <= MAX_CACHED_RUN_VIEWS) {
      break;
    }
    if (
      runId !== activeRunId &&
      runId !== protectedRunId &&
      isTerminalStatus(view.run.status)
    ) {
      next.delete(runId);
    }
  }
  return next;
}

function isStaleSnapshot(view: RunView, incoming: Run): boolean {
  return (
    incoming.lastSequence < Math.max(view.lastSequence, view.run.lastSequence)
  );
}

function upsertInteraction(
  interactions: Interaction[],
  interaction: Interaction,
): Interaction[] {
  const remaining = interactions.filter(
    (item) => item.interactionId !== interaction.interactionId,
  );
  return interaction.status === "pending"
    ? [...remaining, interaction]
    : remaining;
}

function compactInteraction(interaction: Interaction): Interaction {
  const common = {
    ...interaction,
    interactionId: boundedFeedText(interaction.interactionId, 128),
    runId: boundedFeedText(interaction.runId, 128),
    createdAt: boundedFeedText(interaction.createdAt, 128),
    expiresAt: boundedFeedText(interaction.expiresAt, 128),
    etag: boundedFeedText(interaction.etag, 256),
  };
  if (interaction.content.type === "approval") {
    return {
      ...common,
      content: {
        ...interaction.content,
        reason: boundedFeedText(interaction.content.reason),
        action: boundedFeedText(interaction.content.action, 1_024),
        resource: boundedFeedText(interaction.content.resource, 1_024),
        requestHash: boundedFeedText(interaction.content.requestHash, 256),
      },
    };
  }
  return {
    ...common,
    content: {
      ...interaction.content,
      question: boundedFeedText(interaction.content.question),
      choices: interaction.content.choices
        .slice(0, MAX_MESSAGE_CONTENT_PARTS)
        .map((choice) => ({
          choiceId: boundedFeedText(choice.choiceId, 128),
          label: boundedFeedText(choice.label, 512),
        })),
    },
  };
}

function insertOrdered(updates: RunUpdate[], update: RunUpdate): RunUpdate[] {
  if (
    updates.length === 0 ||
    (updates.at(-1)?.sequence ?? 0) < update.sequence
  ) {
    return [...updates, update].slice(-MAX_FEED_ITEMS);
  }
  const next = [...updates, update];
  next.sort((left, right) => left.sequence - right.sequence);
  return next.slice(-MAX_FEED_ITEMS);
}

function compactMessageContent(
  content: Extract<
    RunUpdate["update"],
    { type: "message" }
  >["message"]["content"],
): Extract<RunUpdate["update"], { type: "message" }>["message"]["content"] {
  const compacted: typeof content = [];
  let remaining = MAX_FEED_TEXT_CHARACTERS;
  for (const part of content.slice(0, MAX_MESSAGE_CONTENT_PARTS)) {
    if (remaining === 0) {
      break;
    }
    if (part.type === "text") {
      const text = boundedFeedText(part.text, remaining);
      compacted.push({ type: "text", text });
      remaining -= text.length;
      continue;
    }
    const fileName = boundedFeedText(
      part.artifact.fileName,
      Math.min(512, remaining),
    );
    compacted.push({
      type: "artifact",
      artifact: {
        ...part.artifact,
        artifactId: boundedFeedText(part.artifact.artifactId, 128),
        fileName,
        mediaType: boundedFeedText(part.artifact.mediaType, 128),
        sha256: boundedFeedText(part.artifact.sha256, 128),
        createdAt: boundedFeedText(part.artifact.createdAt, 128),
      },
    });
    remaining -= fileName.length;
  }
  return compacted;
}

function feedProjection(update: RunUpdate): RunUpdate | null {
  const base = {
    runId: boundedFeedText(update.runId, 128),
    sequence: update.sequence,
    createdAt: boundedFeedText(update.createdAt, 128),
  };
  switch (update.update.type) {
    case "message":
      return {
        ...base,
        update: {
          type: "message",
          message: {
            ...update.update.message,
            sessionId: boundedFeedText(update.update.message.sessionId, 128),
            runId: boundedFeedText(update.update.message.runId, 128),
            createdAt: boundedFeedText(update.update.message.createdAt, 128),
            content: compactMessageContent(update.update.message.content),
          },
        },
      };
    case "reasoning_summary":
      return {
        ...base,
        update: {
          type: "reasoning_summary",
          summary: boundedFeedText(update.update.summary),
        },
      };
    case "tool_activity":
      return {
        ...base,
        update: {
          type: "tool_activity",
          activity: {
            ...update.update.activity,
            callId: boundedFeedText(update.update.activity.callId, 128),
            toolName: boundedFeedText(update.update.activity.toolName, 512),
            summary: boundedFeedText(update.update.activity.summary),
          },
        },
      };
    case "notice":
      return {
        ...base,
        update: {
          type: "notice",
          reason: boundedFeedText(update.update.reason, 512),
          message: boundedFeedText(update.update.message),
        },
      };
    case "failure":
      return {
        ...base,
        update: {
          type: "failure",
          status: update.update.status,
          failure: {
            ...update.update.failure,
            reason: boundedFeedText(update.update.failure.reason, 512),
            message: boundedFeedText(update.update.failure.message),
          },
        },
      };
    case "cancellation":
      return {
        ...base,
        update: {
          type: "cancellation",
          cancellation: {
            ...update.update.cancellation,
            message: boundedFeedText(update.update.cancellation.message),
          },
        },
      };
    case "state":
      return {
        ...base,
        update: {
          type: "state",
          status: update.update.status,
        },
      };
    case "usage":
      return {
        ...base,
        update: {
          type: "usage",
          usage: { ...update.update.usage },
        },
      };
    case "interaction":
      return {
        ...base,
        update: {
          type: "interaction",
          interaction: compactInteraction(update.update.interaction),
        },
      };
    case "result":
      return {
        ...base,
        update: {
          type: "result",
          result: {
            ...update.update.result,
            output: "",
            profile: boundedFeedText(update.update.result.profile, 512),
            model: boundedFeedText(update.update.result.model, 512),
          },
        },
      };
    case "output_delta":
      return null;
  }
}

function applyUpdate(view: RunView, update: RunUpdate): RunView {
  if (view.seenSequences.has(update.sequence)) {
    return view;
  }

  const minimumTrackedSequence = Math.max(
    0,
    update.sequence - MAX_TRACKED_SEQUENCES,
  );
  const seenSequences = new Set(
    [...view.seenSequences].filter(
      (sequence) => sequence >= minimumTrackedSequence,
    ),
  );
  seenSequences.add(update.sequence);
  let run = {
    ...view.run,
    lastSequence: Math.max(view.run.lastSequence, update.sequence),
    updatedAt: update.createdAt,
  };
  let output = view.output;
  let pendingInteractions = view.pendingInteractions;
  let usage = view.usage;

  switch (update.update.type) {
    case "state":
      run = { ...run, status: update.update.status };
      break;
    case "output_delta":
      output = boundedOutput(output + update.update.delta);
      break;
    case "interaction":
      pendingInteractions = upsertInteraction(
        pendingInteractions,
        update.update.interaction,
      );
      run = {
        ...run,
        pendingInteractionCount: pendingInteractions.length,
      };
      break;
    case "usage":
      usage = update.update.usage;
      break;
    case "result":
      run = {
        ...run,
        status: "completed",
        terminal: {
          type: "result",
          result: {
            ...update.update.result,
            output: "",
            profile: boundedFeedText(update.update.result.profile, 512),
            model: boundedFeedText(update.update.result.model, 512),
          },
        },
      };
      if (output.length === 0) {
        output = boundedOutput(update.update.result.output);
      }
      break;
    case "failure":
      run = {
        ...run,
        status: update.update.status,
        terminal: {
          type: "failure",
          failure: {
            ...update.update.failure,
            reason: boundedFeedText(update.update.failure.reason, 512),
            message: boundedFeedText(update.update.failure.message),
          },
        },
      };
      break;
    case "cancellation":
      run = {
        ...run,
        status: "cancelled",
        terminal: {
          type: "cancellation",
          cancellation: {
            ...update.update.cancellation,
            message: boundedFeedText(update.update.cancellation.message),
          },
        },
      };
      break;
    case "reasoning_summary":
    case "tool_activity":
    case "message":
    case "notice":
      break;
  }

  if (isTerminalStatus(run.status)) {
    pendingInteractions = [];
    run = { ...run, pendingInteractionCount: 0 };
  }

  const feedUpdate = feedProjection(update);
  return {
    ...view,
    run,
    output,
    updates:
      feedUpdate === null
        ? view.updates
        : insertOrdered(view.updates, feedUpdate),
    seenSequences,
    lastSequence: Math.max(view.lastSequence, update.sequence),
    pendingInteractions,
    usage,
    streamError: null,
  };
}

function mergeRuns(existing: Run[], incoming: Run[]): Run[] {
  const byId = new Map(existing.map((run) => [run.runId, run]));
  for (const incomingRun of incoming) {
    const run = compactRun(incomingRun);
    const current = byId.get(run.runId);
    if (current === undefined || run.lastSequence >= current.lastSequence) {
      byId.set(run.runId, run);
    }
  }
  return [...byId.values()]
    .sort((left, right) => right.createdAt.localeCompare(left.createdAt))
    .slice(0, MAX_RECENT_RUNS);
}

function refreshRecentRuns(existing: Run[], incoming: Run[]): Run[] {
  const existingById = new Map(existing.map((run) => [run.runId, run]));
  return mergeRuns(
    [],
    incoming.map((run) => {
      const current = existingById.get(run.runId);
      return current !== undefined && current.lastSequence > run.lastSequence
        ? current
        : run;
    }),
  );
}

function updateRecentRun(recentRuns: Run[], run: Run): Run[] {
  return mergeRuns(recentRuns, [run]);
}

export function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case "select_run":
      return {
        ...state,
        activeRunId: action.runId,
        views: trimViewCache(state.views, action.runId),
      };
    case "upsert_run": {
      const current = state.views.get(action.run.runId);
      if (current !== undefined && isStaleSnapshot(current, action.run)) {
        return state;
      }
      const views = new Map(state.views);
      views.set(
        action.run.runId,
        current === undefined
          ? newView(action.run)
          : {
              ...current,
              run: compactRun(action.run),
              output: current.output || terminalOutput(action.run.terminal),
              pendingInteractions: isTerminalStatus(action.run.status)
                ? []
                : current.pendingInteractions,
            },
      );
      return {
        ...state,
        views: trimViewCache(views, state.activeRunId, action.run.runId),
        recentRuns: updateRecentRun(state.recentRuns, action.run),
      };
    }
    case "record_local_prompt":
      return {
        ...state,
        views: updateViewMap(state.views, action.runId, (view) => ({
          ...view,
          localPrompt: view.localPrompt ?? action.prompt,
        })),
      };
    case "hydrate_run": {
      const { run, pendingInteractions } = action.details;
      const retainedInteractions = isTerminalStatus(run.status)
        ? []
        : pendingInteractions;
      const current = state.views.get(run.runId);
      if (current !== undefined && isStaleSnapshot(current, run)) {
        return state;
      }
      const views = new Map(state.views);
      views.set(
        run.runId,
        current === undefined
          ? newView(run, retainedInteractions)
          : {
              ...current,
              run: compactRun(run),
              output: current.output || terminalOutput(run.terminal),
              pendingInteractions: retainedInteractions,
            },
      );
      return {
        ...state,
        views: trimViewCache(views, state.activeRunId, run.runId),
        recentRuns: updateRecentRun(state.recentRuns, run),
      };
    }
    case "ingest_update": {
      const current = state.views.get(action.update.runId);
      if (
        current === undefined ||
        current.seenSequences.has(action.update.sequence) ||
        action.update.sequence <= current.lastSequence
      ) {
        return state;
      }
      const nextView = applyUpdate(current, action.update);
      return {
        ...state,
        views: trimViewCache(
          updateViewMap(state.views, action.update.runId, () => nextView),
          state.activeRunId,
        ),
        recentRuns: updateRecentRun(state.recentRuns, nextView.run),
      };
    }
    case "watch_started":
      return {
        ...state,
        views: updateViewMap(state.views, action.runId, (view) => ({
          ...view,
          streamState: "watching",
          streamError: null,
        })),
      };
    case "watch_complete":
      return {
        ...state,
        views: updateViewMap(state.views, action.runId, (view) => ({
          ...view,
          streamState: "complete",
          streamError: null,
        })),
      };
    case "watch_error":
      return {
        ...state,
        views: updateViewMap(state.views, action.runId, (view) => ({
          ...view,
          streamState: "error",
          streamError: action.error,
        })),
      };
    case "interaction_resolved":
      return {
        ...state,
        views: updateViewMap(state.views, action.interaction.runId, (view) => {
          const pendingInteractions = upsertInteraction(
            view.pendingInteractions,
            action.interaction,
          );
          return {
            ...view,
            pendingInteractions,
            run: {
              ...view.run,
              pendingInteractionCount: pendingInteractions.length,
            },
          };
        }),
      };
    case "replace_recent":
      return {
        ...state,
        recentRuns: refreshRecentRuns(state.recentRuns, action.runs),
        nextPageToken: action.nextPageToken,
      };
    case "append_recent":
      return {
        ...state,
        recentRuns: mergeRuns(state.recentRuns, action.runs),
        nextPageToken: action.nextPageToken,
      };
  }
}

export interface IdempotentAttempt {
  fingerprint: string;
  key: string;
}

export function stableIdempotentAttempt(
  previous: IdempotentAttempt | null,
  fingerprint: string,
  createKey: () => string = () => crypto.randomUUID(),
): IdempotentAttempt {
  return previous?.fingerprint === fingerprint
    ? previous
    : { fingerprint, key: createKey() };
}

export function withBoundedEntry<Key, Value>(
  entries: ReadonlyMap<Key, Value>,
  key: Key,
  value: Value,
  maxEntries = MAX_CACHED_IDEMPOTENCY_ATTEMPTS,
): Map<Key, Value> {
  const next = new Map(entries);
  next.delete(key);
  next.set(key, value);
  const limit = Math.max(0, Math.trunc(maxEntries));
  while (next.size > limit) {
    const oldest = next.keys().next();
    if (oldest.done) {
      break;
    }
    next.delete(oldest.value);
  }
  return next;
}

export function operationFingerprint(parts: readonly unknown[]): string {
  return JSON.stringify(parts);
}

export function clampMaxTurns(value: number): number {
  const integer = Number.isFinite(value) ? Math.trunc(value) : 1;
  return Math.min(MAX_TURNS, Math.max(1, integer));
}

const UTF8_ENCODER = new TextEncoder();

export function utf8ByteLength(value: string): number {
  return UTF8_ENCODER.encode(value).byteLength;
}

export function isPromptWithinByteLimit(value: string): boolean {
  return utf8ByteLength(value) <= MAX_PROMPT_BYTES;
}

const CONNECTION_ERROR_CODES = new Set([
  "not_configured",
  "disconnected",
  "transport",
  "unavailable",
  "unauthenticated",
  "identity_mismatch",
  "version_mismatch",
]);

export function isConnectionError(error: CommandError): boolean {
  return CONNECTION_ERROR_CODES.has(error.code);
}

export function connectionStateForError(error: CommandError): ConnectionState {
  return error.code === "not_configured" ? "not_configured" : "disconnected";
}
