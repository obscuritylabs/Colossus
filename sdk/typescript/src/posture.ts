import { Metadata, type CallOptions } from "@grpc/grpc-js";
import type { AgentRunServiceClient } from "./gen/colossus/api/v1alpha1/agent_run.js";

export type PolicyProvenance = "runtime_reported";
export type PolicySandboxBackend =
  | "native"
  | "windows_job"
  | "oci"
  | "external"
  | "danger_full_access"
  | "unknown";
export type PolicyApprovalMode =
  "deny" | "ask" | "risk_auto" | "danger_auto" | "unknown";
export type PolicyFindingCode =
  | "storage.ephemeral"
  | "storage.plaintext"
  | "sandbox.danger_full_access"
  | "observability.sensitive_journal_payloads"
  | "credentials.mcp_oauth_plaintext";
export interface PolicyModelLabel {
  profile: string;
  label: string;
}
export interface PolicyFinding {
  code: PolicyFindingCode;
  severity: "warning";
}
export interface PolicyTelemetry {
  provenance: "unavailable";
  deniedRequests: null;
  approvalRequests: null;
  outcomeUnknownRuns: null;
}
/** Released configuration metadata only; this report does not attest enforcement. */
export interface RuntimePolicyPosture {
  schemaVersion: 1;
  provenance: PolicyProvenance;
  fingerprint: string;
  configurationRevision: bigint | null;
  accessProfile: string;
  sandboxBackend: PolicySandboxBackend;
  sandboxProfile: string;
  boundaryAcknowledged: boolean;
  approvalMode: PolicyApprovalMode;
  allowedRoles: string[];
  allowedTools: string[];
  capabilities: string[];
  models: PolicyModelLabel[];
  findings: PolicyFinding[];
  telemetry: PolicyTelemetry;
}
const INVALID = "Invalid bounded runtime policy metadata";
function fail(): never {
  throw new TypeError(INVALID);
}

// Bounded JSON reader preserves all integer tokens exactly and rejects duplicate keys.
// JSON.parse alone would round a Rust u64 before a reviver can validate its value.
function exactJson(text: string): unknown {
  let at = 0;
  function whitespace() {
    while (at < text.length && /[\t\n\r ]/u.test(text[at]!)) at++;
  }
  function string(): string {
    const start = at++;
    while (at < text.length) {
      const char = text[at++];
      if (char === '"') {
        try {
          return JSON.parse(text.slice(start, at)) as string;
        } catch {
          fail();
        }
      }
      if (char === "\\") at++;
    }
    return fail();
  }
  function value(depth: number): unknown {
    if (depth > 8) fail();
    whitespace();
    const char = text[at];
    if (char === '"') return string();
    if (char === "{") {
      at++;
      whitespace();
      const object: Record<string, unknown> = Object.create(null) as Record<
        string,
        unknown
      >;
      if (text[at] === "}") {
        at++;
        return object;
      }
      for (;;) {
        whitespace();
        if (text[at] !== '"') fail();
        const key = string();
        if (Object.hasOwn(object, key)) fail();
        whitespace();
        if (text[at++] !== ":") fail();
        object[key] = value(depth + 1);
        whitespace();
        if (text[at] === "}") {
          at++;
          return object;
        }
        if (text[at++] !== ",") fail();
      }
    }
    if (char === "[") {
      at++;
      whitespace();
      const array: unknown[] = [];
      if (text[at] === "]") {
        at++;
        return array;
      }
      for (;;) {
        array.push(value(depth + 1));
        whitespace();
        if (text[at] === "]") {
          at++;
          return array;
        }
        if (text[at++] !== ",") fail();
      }
    }
    for (const [literal, result] of [
      ["null", null],
      ["true", true],
      ["false", false],
    ] as const) {
      if (text.startsWith(literal, at)) {
        at += literal.length;
        return result;
      }
    }
    const match = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/u.exec(
      text.slice(at),
    );
    if (!match || !/^\d{1,20}$/u.test(match[0])) fail();
    at += match[0].length;
    return BigInt(match[0]);
  }
  const result = value(0);
  whitespace();
  if (at !== text.length) fail();
  return result;
}
function object(
  value: unknown,
  required: string[],
  optional: string[] = [],
): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    fail();
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).some(
      (key) => !required.includes(key) && !optional.includes(key),
    ) ||
    required.some((key) => !Object.hasOwn(record, key))
  )
    fail();
  return record;
}
function identifier(value: unknown): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 128 ||
    /[^A-Za-z0-9._-]/u.test(value)
  )
    fail();
  return value;
}
function list(value: unknown, max: number): string[] {
  if (!Array.isArray(value) || value.length > max) fail();
  const values = value.map(identifier);
  if (values.some((item, index) => index > 0 && values[index - 1]! >= item))
    fail();
  return values;
}
function choice<T extends string>(value: unknown, options: readonly T[]): T {
  if (typeof value !== "string" || !options.includes(value as T)) fail();
  return value as T;
}
export function decodeRuntimePolicyPosture(
  payload: Uint8Array,
): RuntimePolicyPosture {
  if (
    !(payload instanceof Uint8Array) ||
    payload.byteLength === 0 ||
    payload.byteLength > 65536
  )
    fail();
  let parsed: unknown;
  try {
    parsed = exactJson(
      new TextDecoder("utf-8", { fatal: true }).decode(payload),
    );
  } catch {
    fail();
  }
  const p = object(
    parsed,
    [
      "schema_version",
      "provenance",
      "fingerprint",
      "access_profile",
      "sandbox_backend",
      "sandbox_profile",
      "boundary_acknowledged",
      "approval_mode",
      "allowed_roles",
      "allowed_tools",
      "capabilities",
      "models",
      "findings",
      "telemetry",
    ],
    ["configuration_revision"],
  );
  if (
    p.schema_version !== 1n ||
    typeof p.fingerprint !== "string" ||
    p.fingerprint.length !== 64 ||
    /[^0-9a-f]/u.test(p.fingerprint) ||
    typeof p.boundary_acknowledged !== "boolean"
  )
    fail();
  const revision = p.configuration_revision ?? null;
  if (
    revision !== null &&
    (typeof revision !== "bigint" ||
      revision < 0n ||
      revision > 18446744073709551615n)
  )
    fail();
  if (
    !Array.isArray(p.models) ||
    p.models.length > 64 ||
    !Array.isArray(p.findings) ||
    p.findings.length > 16
  )
    fail();
  const models = p.models.map((value) => {
    const model = object(value, ["profile", "label"]);
    return {
      profile: identifier(model.profile),
      label: identifier(model.label),
    };
  });
  const findings = p.findings.map((value) => {
    const finding = object(value, ["code", "severity"]);
    return {
      code: choice(finding.code, [
        "storage.ephemeral",
        "storage.plaintext",
        "sandbox.danger_full_access",
        "observability.sensitive_journal_payloads",
        "credentials.mcp_oauth_plaintext",
      ] as const),
      severity: choice(finding.severity, ["warning"] as const),
    };
  });
  const telemetry = object(
    p.telemetry,
    ["provenance"],
    ["denied_requests", "approval_requests", "outcome_unknown_runs"],
  );
  choice(telemetry.provenance, ["unavailable"]);
  if (
    [
      telemetry.denied_requests,
      telemetry.approval_requests,
      telemetry.outcome_unknown_runs,
    ].some((value) => value !== undefined && value !== null)
  )
    fail();
  return {
    schemaVersion: 1,
    provenance: choice(p.provenance, ["runtime_reported"]),
    fingerprint: p.fingerprint,
    configurationRevision: revision,
    accessProfile: identifier(p.access_profile),
    sandboxBackend: choice(p.sandbox_backend, [
      "native",
      "windows_job",
      "oci",
      "external",
      "danger_full_access",
      "unknown",
    ]),
    sandboxProfile: identifier(p.sandbox_profile),
    boundaryAcknowledged: p.boundary_acknowledged,
    approvalMode: choice(p.approval_mode, [
      "deny",
      "ask",
      "risk_auto",
      "danger_auto",
      "unknown",
    ]),
    allowedRoles: list(p.allowed_roles, 64),
    allowedTools: list(p.allowed_tools, 256),
    capabilities: list(p.capabilities, 256),
    models,
    findings,
    telemetry: {
      provenance: "unavailable",
      deniedRequests: null,
      approvalRequests: null,
      outcomeUnknownRuns: null,
    },
  };
}
/** Read once through the caller's authenticated service client. No transport retries. */
export async function getRuntimePolicyPosture(
  client: Pick<AgentRunServiceClient, "getRuntimePolicyPosture">,
  options: Partial<CallOptions> = {},
): Promise<RuntimePolicyPosture> {
  const response = await new Promise<Uint8Array>((resolve, reject) =>
    client.getRuntimePolicyPosture(
      {},
      new Metadata(),
      { deadline: Date.now() + 5000, ...options },
      (error, result) => {
        if (error !== null) {
          reject(error);
          return;
        }
        if (!result || !(result.policyJson instanceof Uint8Array)) {
          reject(new TypeError(INVALID));
          return;
        }
        resolve(result.policyJson);
      },
    ),
  );
  return decodeRuntimePolicyPosture(response);
}
