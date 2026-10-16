import assert from "node:assert/strict";
import { test } from "node:test";
import { parseAction, parseInboxInspectionRequest } from "../src/model.js";
import * as grpc from "@grpc/grpc-js";
import { connectionStep, safeError } from "../src/connection.js";
import {
  ConnectionError,
  UserError,
  type ConnectionDiagnostic,
} from "../src/errors.js";

test("inbox reads cannot nominate run ownership, transport or unbounded cursors", () => {
  const valid = {
    type: "agentInbox",
    requestId: "read-1",
    participantId: "participant-1",
    afterSequence: 0,
  };
  assert.deepEqual(parseInboxInspectionRequest(valid), valid);
  for (const changed of [
    { runId: "foreign-run" },
    { ownerApplicationId: "other-app" },
    { endpoint: "https://other.example" },
    { afterSequence: -1 },
    { afterSequence: 4097 },
    { afterSequence: 0.5 },
    { participantId: "x".repeat(129) },
    { requestId: "bad\nidentity" },
  ])
    assert.equal(
      parseInboxInspectionRequest({ ...valid, ...changed }),
      undefined,
    );
});

test("the renderer can request native review but cannot supply approval authority or SDK arguments", () => {
  assert.deepEqual(parseAction({ type: "respond", id: "interaction-1" }), {
    type: "respond",
    id: "interaction-1",
  });
  for (const input of [
    { type: "respond", id: "interaction-1", approved: true },
    { type: "respond", id: "interaction-1", requestHash: "forged" },
    { type: "send", text: "task", mode: "execute", workspace: "/other" },
    { type: "send", text: "task", mode: "research" },
    { type: "invokeSdk", method: "createRun" },
    { type: "readFile", path: "/etc/passwd" },
    { type: "send", text: "é".repeat(64 * 1024), mode: "plan" },
    null,
  ])
    assert.equal(parseAction(input), undefined);
});

test("Research accepts only an explicit bounded depth and unique evidence lanes", () => {
  const research = {
    type: "send",
    text: "Investigate MCP evidence",
    mode: "research",
    researchDepth: "deep",
    researchSources: ["repo", "web", "mcp"],
  };
  assert.deepEqual(parseAction(research), research);
  for (const input of [
    { ...research, researchDepth: undefined },
    { ...research, researchDepth: "unbounded" },
    { ...research, researchSources: [] },
    { ...research, researchSources: ["mcp", "mcp"] },
    { ...research, researchSources: ["filesystem"] },
    { ...research, researchSources: new Array(1) },
    { ...research, researchSources: "mcp" },
    { ...research, mode: "execute" },
    { ...research, mode: "plan" },
    { ...research, tools: ["*"] },
    { ...research, server: "forged-server" },
    { ...research, researchSources: ["mcp"], arguments: { query: "forged" } },
  ])
    assert.equal(parseAction(input), undefined);
});

test("only deliberate public errors cross the webview boundary", () => {
  const raw = new Error("private-path and sensitive-token-fixture");
  assert.doesNotMatch(safeError(raw), /private-path|sensitive-token/u);
  assert.equal(
    safeError(new UserError("Select a workspace.")),
    "Select a workspace.",
  );
});

test("model preferences accept bounded choices without granting connection or tool authority", () => {
  const selection = { modelProfile: "deep", reasoningEffort: "high" };
  const send = {
    type: "send",
    text: "Continue",
    mode: "execute",
    modelSelection: selection,
  };
  assert.deepEqual(parseAction(send), send);
  assert.deepEqual(parseAction({ type: "setModelSelection", selection }), {
    type: "setModelSelection",
    selection,
  });
  for (const invalid of [
    null,
    [],
    { modelProfile: "" },
    { modelProfile: "x".repeat(129) },
    { modelProfile: "é".repeat(65) },
    { modelProfile: "bad\nprofile" },
    { reasoningEffort: "unlimited" },
    { reasoningEffort: undefined },
    { provider: "other" },
    { model: "unconfigured" },
    { tools: ["*"] },
    { endpoint: "http://other" },
    { role: "admin" },
  ]) {
    assert.equal(parseAction({ ...send, modelSelection: invalid }), undefined);
    assert.equal(
      parseAction({ type: "setModelSelection", selection: invalid }),
      undefined,
    );
  }
  assert.equal(
    parseAction({
      ...send,
      mode: "research",
      researchDepth: "quick",
      researchSources: ["repo"],
    }),
    undefined,
  );
});

test("connection diagnostics identify credential failures without exposing raw errors or retrying", async () => {
  for (const [code, reason] of [
    ["ENOENT", "missing-file"],
    [grpc.status.UNAUTHENTICATED, "unauthenticated"],
    [grpc.status.UNAVAILABLE, "unavailable"],
    [undefined, "failed"],
  ] as const) {
    const events: ConnectionDiagnostic[] = [];
    let calls = 0;
    const failure = Object.assign(
      new Error("sensitive-token-fixture /private/path"),
      {
        code,
        details: "sensitive-token-fixture",
        metadata: { authorization: "sensitive-token-fixture" },
      },
    );
    await assert.rejects(
      connectionStep(
        "handshake",
        async () => {
          calls++;
          throw failure;
        },
        (event) => events.push(event),
      ),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "handshake");
        assert.equal(error.reason, reason);
        assert.doesNotMatch(safeError(error), /sensitive-token|private\/path/u);
        return true;
      },
    );
    assert.equal(calls, 1);
    assert.deepEqual(events, [
      { stage: "handshake", state: "started" },
      { stage: "handshake", state: "failed", reason },
    ]);
    assert.doesNotMatch(
      JSON.stringify(events),
      /sensitive-token|private\/path|authorization/u,
    );
  }
});
