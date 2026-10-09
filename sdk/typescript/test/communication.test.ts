import assert from "node:assert/strict";
import { test } from "node:test";
import type { AgentCommunicationServiceClient } from "../src/gen/colossus/api/v1alpha1/communication.js";
import { AgentMessage } from "../src/gen/colossus/api/v1alpha1/communication.js";
import {
  AgentCommunication,
  validateAgentMessage,
} from "../src/communication.js";

const message = () =>
  AgentMessage.fromPartial({
    id: "message-1",
    rootRunId: "run-1",
    recipientId: "participant-1",
    sender: { $case: "senderApplicationId", value: "app-1" },
    sequence: 1n,
    text: "Peer input",
    acceptedAt: "2026-10-09T12:00:00Z",
    receipt: { state: "accepted" },
  });

test("contradictory receipts and oversized UTF-8 peer input fail closed", () => {
  assert.equal(validateAgentMessage(message()).receipt?.state, "accepted");
  for (const changed of [
    { receipt: { state: "accepted", reason: "failed" } },
    {
      receipt: {
        state: "included_in_turn",
        runId: "run-1",
        turn: 1,
        requestHash: "not-a-hash",
      },
    },
    { receipt: { state: "not_delivered", reason: "invented" } },
    { text: "😀".repeat(4097) },
    { sender: undefined },
  ])
    assert.throws(
      () => validateAgentMessage({ ...message(), ...changed }),
      TypeError,
    );
});

test("inbox pages reject gaps and messages addressed to another attempt", async () => {
  for (const changed of [
    { sequence: 2n },
    { recipientId: "foreign-attempt" },
  ]) {
    const client = {
      listAgentMessages(...args: unknown[]) {
        const callback = args.at(-1) as (
          error: null,
          response: unknown,
        ) => void;
        callback(null, {
          messages: [{ ...message(), ...changed }],
          nextSequence: 1n,
          hasMore: false,
        });
      },
    } as unknown as AgentCommunicationServiceClient;
    await assert.rejects(
      new AgentCommunication(client).messages("participant-1"),
      TypeError,
    );
  }
});

test("a lost mutation response never retries the effect", async () => {
  let calls = 0;
  const error = Object.assign(new Error("Unknown send outcome"), { code: 14 });
  const client = {
    sendAgentMessage(...args: unknown[]) {
      calls++;
      const callback = args.at(-1) as (error: Error) => void;
      callback(error);
    },
  } as unknown as AgentCommunicationServiceClient;
  await assert.rejects(
    new AgentCommunication(client).send({
      recipientId: "participant-1",
      text: "Peer input",
      idempotencyKey: "stable-key",
      replyTo: undefined,
    }),
    (received) => received === error,
  );
  assert.equal(calls, 1);
});

test("a bad watch cursor releases only its subscription", async () => {
  let cancellations = 0;
  const client = {
    watchAgentMessages() {
      return {
        async *[Symbol.asyncIterator]() {
          yield { sequence: 2n, message: message() };
        },
        cancel() {
          cancellations++;
        },
      };
    },
  } as unknown as AgentCommunicationServiceClient;
  await assert.rejects(async () => {
    for await (const _update of new AgentCommunication(client).watch("run-1"))
      assert.fail("a gap cannot be released");
  }, TypeError);
  assert.equal(cancellations, 1);
});
