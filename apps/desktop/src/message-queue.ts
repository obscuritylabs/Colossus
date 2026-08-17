import type {
  ArtifactReference,
  CommandError,
  ResearchDepth,
  ResearchSourceKind,
  RunMode,
} from "./types";

export const MAX_QUEUED_MESSAGES_PER_THREAD = 8;
export const MAX_QUEUED_MESSAGES_TOTAL = 32;

export type QueuedMessageState = "pending" | "sending" | "failed";

export interface QueuedMessage {
  id: string;
  idempotencyKey: string;
  targetId: string;
  sessionId: string;
  prompt: string;
  role: string;
  mode: RunMode;
  researchDepth: ResearchDepth;
  researchSources: readonly ResearchSourceKind[];
  maxTurns: number;
  attachments: readonly ArtifactReference[];
  createdAt: string;
  state: QueuedMessageState;
  error: CommandError | null;
}

export type QueuePlacement = "next" | "last";

export type EnqueueResult =
  | { accepted: true; messages: readonly QueuedMessage[] }
  | { accepted: false; reason: "thread_full" | "queue_full" };

export function messagesForThread(
  messages: readonly QueuedMessage[],
  targetId: string,
  sessionId: string,
): QueuedMessage[] {
  return messages.filter(
    (message) =>
      message.targetId === targetId && message.sessionId === sessionId,
  );
}

export function enqueueMessage(
  messages: readonly QueuedMessage[],
  message: QueuedMessage,
  placement: QueuePlacement,
): EnqueueResult {
  if (
    messagesForThread(messages, message.targetId, message.sessionId).length >=
    MAX_QUEUED_MESSAGES_PER_THREAD
  ) {
    return { accepted: false, reason: "thread_full" };
  }
  if (messages.length >= MAX_QUEUED_MESSAGES_TOTAL) {
    return { accepted: false, reason: "queue_full" };
  }

  if (placement === "last") {
    return { accepted: true, messages: [...messages, message] };
  }

  const firstThreadIndex = messages.findIndex(
    (candidate) =>
      candidate.targetId === message.targetId &&
      candidate.sessionId === message.sessionId,
  );
  if (firstThreadIndex < 0) {
    return { accepted: true, messages: [...messages, message] };
  }
  return {
    accepted: true,
    messages: [
      ...messages.slice(0, firstThreadIndex),
      message,
      ...messages.slice(firstThreadIndex),
    ],
  };
}

export function updateQueuedMessage(
  messages: readonly QueuedMessage[],
  messageId: string,
  update: (message: QueuedMessage) => QueuedMessage,
): QueuedMessage[] {
  return messages.map((message) =>
    message.id === messageId ? update(message) : message,
  );
}

export function removeQueuedMessage(
  messages: readonly QueuedMessage[],
  messageId: string,
): QueuedMessage[] {
  return messages.filter((message) => message.id !== messageId);
}

export function nextPendingMessage(
  messages: readonly QueuedMessage[],
  targetId: string,
  sessionId: string,
): QueuedMessage | undefined {
  return messages.find(
    (message) =>
      message.targetId === targetId &&
      message.sessionId === sessionId &&
      message.state === "pending",
  );
}
