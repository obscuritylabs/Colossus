import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_BROWSER_SETTINGS,
  PrivateFrames,
  runtimeConfiguration,
  type ManagedSetup,
} from "../src/managed.js";

test("browser configuration reaches the native runtime and cannot widen its ceilings", () => {
  const setup: ManagedSetup = {
    version: 1,
    workspaceIdentity: "fixture",
    access: "minimal",
    primary: "echo",
    providers: [
      {
        profile: "echo",
        kind: "echo",
        base_url: null,
        credential_id: null,
        timeout_ms: 5000,
      },
    ],
    models: [
      {
        profile: "echo",
        provider_profile: "echo",
        model: "echo",
        context_window_tokens: 32768,
        max_output_tokens: 4096,
        capabilities: {
          tool_calls: "auto",
          streaming: "auto",
          image_inputs: "auto",
          server_compaction: "auto",
          declared: {},
        },
      },
    ],
  };
  assert.deepEqual(
    runtimeConfiguration(setup, {
      enabled: false,
      publicWeb: false,
      maxSessions: 1,
      maxTabs: 2,
      maxSnapshotNodes: 64,
    }).field_overrides,
    [
      { field_id: "browser.enabled", value: false },
      { field_id: "browser.publicWeb", value: false },
      { field_id: "browser.maxSessions", value: 1 },
      { field_id: "browser.maxTabs", value: 2 },
      { field_id: "browser.maxSnapshotNodes", value: 64 },
    ],
  );
  for (const invalid of [
    { maxTabs: 9 },
    { maxSessions: 0 },
    { maxSnapshotNodes: 1.5 },
  ]) {
    assert.throws(() =>
      runtimeConfiguration(setup, { ...DEFAULT_BROWSER_SETTINGS, ...invalid }),
    );
  }
});

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
