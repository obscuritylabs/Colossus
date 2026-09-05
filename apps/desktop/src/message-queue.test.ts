import { describe, expect, it } from "vitest";

import type { QueuedMessage } from "./message-queue";
import {
  MAX_QUEUED_MESSAGES_PER_THREAD,
  enqueueMessage,
  messagesForThread,
  nextPendingMessage,
  removeQueuedMessage,
  updateQueuedMessage,
} from "./message-queue";

function message(
  id: string,
  sessionId = "session-1",
  state: QueuedMessage["state"] = "pending",
): QueuedMessage {
  return {
    id,
    idempotencyKey: `key-${id}`,
    targetId: "target-1",
    sessionId,
    prompt: `Prompt ${id}`,
    role: "primary",
    mode: "execute",
    researchDepth: "standard",
    researchSources: ["repo", "web", "mcp"],
    maxTurns: 0,
    attachments: [],
    createdAt: "2026-08-14T12:00:00Z",
    state,
    error: null,
  };
}

describe("message queue", () => {
  it("retains captured per-message IDs and original display text through delivery and retry", () => {
    const queued = {
      ...message("selected"),
      prompt: "@colossus/coding work",
      executionPrompt: "work",
      pluginSkillIds: ["colossus/coding", "colossus/security-review"],
      conversationSkillIds: ["colossus/security-review"],
    };
    const sending = updateQueuedMessage([queued], queued.id, (item) => ({
      ...item,
      state: "sending",
    }));
    const retry = updateQueuedMessage(sending, queued.id, (item) => ({
      ...item,
      state: "pending",
    }));
    expect(nextPendingMessage(retry, "target-1", "session-1")).toEqual(queued);
  });
  it("keeps ordinary follow-ups FIFO and places redirects first in their thread", () => {
    const first = enqueueMessage([], message("one"), "last");
    expect(first.accepted).toBe(true);
    if (!first.accepted) return;
    const second = enqueueMessage(first.messages, message("two"), "last");
    expect(second.accepted).toBe(true);
    if (!second.accepted) return;
    const redirect = enqueueMessage(
      second.messages,
      message("redirect"),
      "next",
    );
    expect(redirect.accepted).toBe(true);
    if (!redirect.accepted) return;

    expect(
      messagesForThread(redirect.messages, "target-1", "session-1").map(
        (item) => item.id,
      ),
    ).toEqual(["redirect", "one", "two"]);
  });

  it("isolates thread selection and skips failed or sending entries", () => {
    const messages = [
      message("failed", "session-1", "failed"),
      message("other", "session-2"),
      message("sending", "session-1", "sending"),
      message("ready", "session-1"),
    ];

    expect(nextPendingMessage(messages, "target-1", "session-1")?.id).toBe(
      "ready",
    );
    expect(messagesForThread(messages, "target-1", "session-2")).toHaveLength(
      1,
    );
  });

  it("updates and removes one entry without disturbing queue order", () => {
    const messages = [message("one"), message("two")];
    const updated = updateQueuedMessage(messages, "one", (item) => ({
      ...item,
      prompt: "Edited",
    }));

    expect(updated.map((item) => item.prompt)).toEqual([
      "Edited",
      "Prompt two",
    ]);
    expect(removeQueuedMessage(updated, "one").map((item) => item.id)).toEqual([
      "two",
    ]);
  });

  it("bounds each thread queue", () => {
    const full = Array.from(
      { length: MAX_QUEUED_MESSAGES_PER_THREAD },
      (_, i) => message(`item-${i}`),
    );

    expect(enqueueMessage(full, message("extra"), "last")).toEqual({
      accepted: false,
      reason: "thread_full",
    });
  });
});
