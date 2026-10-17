import assert from "node:assert/strict";
import { test } from "node:test";
import { PrivateFrames, runtimeConfiguration } from "../src/managed.js";

test("private IPC frames accept fragmented input and reject unbounded or malformed output", () => {
  const received: unknown[] = [];
  const frames = new PrivateFrames((value) => received.push(value));
  const payload = Buffer.from('{"type":"stopped"}');
  const length = Buffer.alloc(4);
  length.writeUInt32BE(payload.length);
  for (const byte of Buffer.concat([length, payload, length, payload]))
    frames.push(Buffer.from([byte]));
  assert.deepEqual(received, [{ type: "stopped" }, { type: "stopped" }]);
  for (const input of [
    Buffer.alloc(4),
    Buffer.from([0, 32, 0, 1]),
    Buffer.from([0, 0, 0, 1, 0]),
  ])
    assert.throws(() => new PrivateFrames(() => {}).push(input));
});

test("managed setup cannot select an unknown default or widen the host access contract", () => {
  assert.throws(() =>
    runtimeConfiguration({
      version: 1,
      workspaceIdentity: "fixture",
      access: "minimal",
      primary: "missing",
      providers: [],
      models: [],
    }),
  );
  assert.throws(() =>
    runtimeConfiguration({
      version: 1,
      workspaceIdentity: "fixture",
      access: "allow_all" as "minimal",
      primary: "missing",
      providers: [],
      models: [],
    }),
  );
});
