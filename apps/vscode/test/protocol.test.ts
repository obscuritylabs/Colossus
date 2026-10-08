import assert from "node:assert/strict";
import { test } from "node:test";
import { parseAction } from "../src/model.js";
import * as grpc from "@grpc/grpc-js";
import { connectionStep, safeError } from "../src/connection.js";
import {
  ConnectionError,
  UserError,
  type ConnectionDiagnostic,
} from "../src/errors.js";

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
