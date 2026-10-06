import assert from "node:assert/strict";
import { test } from "node:test";
import {
  decodeRuntimePolicyPosture,
  getRuntimePolicyPosture,
} from "../src/posture.js";
import type {
  AgentRunServiceClient,
  GetRuntimePolicyPostureResponse,
} from "../src/gen/colossus/api/v1alpha1/agent_run.js";
import type { ServiceError } from "@grpc/grpc-js";
const valid = () => ({
  schema_version: 1,
  provenance: "runtime_reported",
  fingerprint: "a".repeat(64),
  configuration_revision: null,
  access_profile: "minimal",
  sandbox_backend: "native",
  sandbox_profile: "offline",
  boundary_acknowledged: false,
  approval_mode: "ask",
  allowed_roles: ["primary"],
  allowed_tools: ["filesystem.read"],
  capabilities: ["runs.read"],
  models: [{ profile: "main", label: "safe-model" }],
  findings: [{ code: "storage.ephemeral", severity: "warning" }],
  telemetry: {
    provenance: "unavailable",
    denied_requests: null,
    approval_requests: null,
    outcome_unknown_runs: null,
  },
});
const encode = (value: unknown) =>
  new TextEncoder().encode(JSON.stringify(value));
test("typed policy report preserves u64 revisions and explicit unavailable counters", () => {
  const json = JSON.stringify(valid()).replace(
    '"configuration_revision":null',
    '"configuration_revision":18446744073709551615',
  );
  const result = decodeRuntimePolicyPosture(new TextEncoder().encode(json));
  assert.equal(result.configurationRevision, 18446744073709551615n);
  assert.equal(result.telemetry.deniedRequests, null);
  assert.equal(result.models[0]?.label, "safe-model");
});
test("closed policy metadata rejects private fields and malformed bounded shapes", () => {
  const invalid: unknown[] = [
    { ...valid(), secret: "withheld" },
    { ...valid(), schema_version: 2 },
    { ...valid(), sandbox_backend: "custom" },
    { ...valid(), boundary_acknowledged: null },
    { ...valid(), allowed_roles: ["primary", "primary"] },
    { ...valid(), allowed_roles: ["primary\n"] },
    { ...valid(), fingerprint: "a".repeat(64) + "\n" },
    { ...valid(), allowed_tools: null },
    { ...valid(), capabilities: ["z", "a"] },
    {
      ...valid(),
      models: [{ profile: "main", label: "https://private.invalid" }],
    },
    {
      ...valid(),
      models: [{ profile: "main", label: "safe", endpoint: "withheld" }],
    },
    { ...valid(), findings: [{ code: "arbitrary", severity: "warning" }] },
    { ...valid(), telemetry: { ...valid().telemetry, denied_requests: 0 } },
    { ...valid(), configuration_revision: 1.5 },
  ];
  for (const value of invalid)
    assert.throws(
      () => decodeRuntimePolicyPosture(encode(value)),
      /^TypeError: Invalid bounded runtime policy metadata$/u,
    );
  for (const bytes of [
    new Uint8Array(),
    new Uint8Array(65537),
    new Uint8Array([0xff]),
    new TextEncoder().encode(
      JSON.stringify(valid()).replace(
        '"schema_version":1',
        '"schema_version":1,"schema_version":1',
      ),
    ),
    new TextEncoder().encode(JSON.stringify(valid()) + " {}"),
    new TextEncoder().encode(
      JSON.stringify(valid()).replace(
        '"configuration_revision":null',
        '"configuration_revision":18446744073709551616',
      ),
    ),
  ])
    assert.throws(() => decodeRuntimePolicyPosture(bytes), /Invalid bounded/u);
});
test("policy getter makes one authenticated read and propagates transport failures", async () => {
  let calls = 0;
  const failure = Object.assign(new Error("Unavailable"), {
    code: 14,
  }) as ServiceError;
  const client = {
    getRuntimePolicyPosture(
      _request: unknown,
      _metadata: unknown,
      _options: unknown,
      callback: (
        error: ServiceError | null,
        response?: GetRuntimePolicyPostureResponse,
      ) => void,
    ) {
      calls++;
      callback(failure);
    },
  } as Pick<AgentRunServiceClient, "getRuntimePolicyPosture">;
  await assert.rejects(() => getRuntimePolicyPosture(client), failure);
  assert.equal(calls, 1);
});
