import { expect, test } from "@playwright/test";
import { createHash } from "node:crypto";
import { approvalProcessDiagnostics } from "./support/approval-diagnostics";

function captured(output: unknown, callId?: string) {
  return JSON.stringify({
    messages: [
      { role: "tool", tool_call_id: callId, content: JSON.stringify(output) },
    ],
  });
}

function enveloped(output: unknown, changed: Record<string, unknown> = {}) {
  const source = JSON.stringify(output);
  return {
    _colossusToolObservation: {
      truncated: false,
      format: "json",
      toolName: "shell.run",
      callId: "approval-call",
      exitCode: 1,
      originalBytes: Buffer.byteLength(source),
      sha256: createHash("sha256").update(source).digest("hex"),
      ...changed,
    },
    data: output,
  };
}

function correlated(
  output: unknown,
  names = ["shell_run"],
  assistantCallId = "approval-call",
  resultCallId = "approval-call",
) {
  return JSON.stringify({
    messages: [
      {
        role: "assistant",
        tool_calls: names.map((name) => ({
          id: assistantCallId,
          type: "function",
          function: { name },
        })),
      },
      {
        role: "tool",
        tool_call_id: resultCallId,
        content: JSON.stringify(output),
      },
    ],
  });
}

test("runtime execution_error retains its category without exposing private adapter details", () => {
  const result = approvalProcessDiagnostics([
    captured({
      error: {
        type: "execution_error",
        message: "adapter failed: private-executable and private-binding",
        tool: "shell.run",
        recoverable: true,
      },
    }),
  ]);
  expect(result.results).toEqual([
    {
      category: "execution_error",
      exitCode: null,
      timeout: false,
      memoryLimit: false,
      accessDenied: false,
      appContainer: false,
      cleanup: false,
      helper: false,
      permit: false,
    },
  ]);
  expect(JSON.stringify(result)).not.toContain("private-");
});

test("approval failure diagnostics disclose only bounded categories and numeric exit codes", () => {
  const result = approvalProcessDiagnostics([
    captured({
      error: {
        type: "validation_error",
        message: "sandbox helper exceeded its timeout private-credential",
      },
      exit_code: 1,
      stdout: "private-output",
      invocation: "private-command",
      request_hash: "private-binding",
    }),
  ]);
  expect(result).toEqual({
    providerRequests: 1,
    results: [
      {
        category: "validation_error",
        exitCode: 1,
        timeout: true,
        memoryLimit: false,
        accessDenied: false,
        appContainer: false,
        cleanup: false,
        helper: true,
        permit: false,
      },
    ],
  });
  expect(JSON.stringify(result)).not.toContain("private-");
});

test("approval diagnostics distinguish setup, cleanup, and resource failures", () => {
  const result = approvalProcessDiagnostics([
    captured({
      error: {
        type: "private-category",
        message: "AppContainer access denied while removing protected ACL",
      },
      exit_code: "private-code",
    }),
    captured({
      error: {
        type: "validation_error",
        message: "sandboxed process exceeded its memory limit",
      },
    }),
    captured({
      error: {
        type: "denied",
        message: "permit deadline exceeded during cleanup",
      },
    }),
  ]);
  expect(result.results).toEqual([
    expect.objectContaining({
      category: "unclassified",
      exitCode: null,
      accessDenied: true,
      appContainer: true,
    }),
    expect.objectContaining({
      category: "validation_error",
      memoryLimit: true,
    }),
    expect.objectContaining({
      category: "denied",
      timeout: true,
      cleanup: true,
      permit: true,
    }),
  ]);
  expect(JSON.stringify(result)).not.toContain("private-");
});

test("malformed, oversized, and unrecognized captured content is never echoed", () => {
  const result = approvalProcessDiagnostics([
    "private-malformed",
    captured({ error: { type: "private-category" }, exit_code: 2 ** 40 }),
    "private-oversized".repeat(256 * 1024),
    JSON.stringify({
      messages: [{ role: "assistant", content: "private-assistant" }],
    }),
  ]);
  expect(result.results).toHaveLength(1);
  expect(result.results[0]).toMatchObject({
    category: "unclassified",
    exitCode: null,
  });
  expect(JSON.stringify(result)).not.toContain("private-");
  expect(
    approvalProcessDiagnostics(
      Array.from({ length: 20 }, () => captured({ exit_code: 0 })),
    ).results,
  ).toHaveLength(4);
});

test("complete correlated failure receipts retain safe diagnostics without releasing private fields", () => {
  const output = {
    error: {
      type: "validation_error",
      message: "sandbox helper timed out private-credential private-path",
    },
    exit_code: 1,
    stderr: "private-stderr",
    invocation: "private-command",
    request_hash: "private-binding",
  };
  const expected = approvalProcessDiagnostics([captured(output)]);
  for (const name of ["shell_run", "shell.run"]) {
    const result = approvalProcessDiagnostics([
      correlated(enveloped(output), [name]),
    ]);
    expect(result).toEqual(expected);
    expect(result.results).toEqual([
      expect.objectContaining({
        category: "validation_error",
        exitCode: 1,
        timeout: true,
        helper: true,
      }),
    ]);
    expect(JSON.stringify(result)).not.toContain("private-");
    expect(JSON.stringify(result)).not.toContain("approval-call");
    expect(JSON.stringify(result)).not.toContain("sha256");
  }
  expect(
    approvalProcessDiagnostics([captured(enveloped(output), "approval-call")]),
  ).toEqual(expected);
});

test("error-only gateway receipts use the bounded ToolResult status without exposing private error text", () => {
  const result = approvalProcessDiagnostics([
    correlated(
      enveloped({
        error: {
          type: "execution_error",
          tool: "shell.run",
          recoverable: true,
          message:
            "sandbox helper timed out private-command private-credential",
        },
      }),
    ),
  ]);
  expect(result.results).toEqual([
    expect.objectContaining({
      category: "execution_error",
      exitCode: 1,
      timeout: true,
      helper: true,
    }),
  ]);
  expect(JSON.stringify(result)).not.toContain("private-");
  const running = { exit_code: null, status: "running" };
  expect(
    approvalProcessDiagnostics([
      correlated(enveloped(running, { exitCode: 0 })),
    ]),
  ).toEqual(approvalProcessDiagnostics([captured(running)]));
});

test("truncated or malformed observation metadata cannot supply nested diagnostics", () => {
  const output = {
    error: {
      type: "execution_error",
      message: "sandbox helper timed out private-path",
    },
    exit_code: 7,
  };
  for (const changed of [
    { truncated: true },
    { truncated: "false" },
    { format: "text" },
    { callId: undefined },
    { callId: "foreign-call" },
    { toolName: "filesystem.read" },
    { exitCode: undefined },
    { exitCode: 0.5 },
    { exitCode: 2 ** 31 },
    { originalBytes: -1 },
    { originalBytes: 2 ** 53 },
    { sha256: "private-not-a-digest" },
    { identity: "private-not-an-object" },
    { privateMetadata: true },
  ]) {
    const result = approvalProcessDiagnostics([
      correlated(enveloped(output, changed)),
    ]);
    expect(result.results, JSON.stringify(changed)).toEqual([]);
    expect(JSON.stringify(result)).not.toContain("private-");
  }
  const complete = enveloped(output);
  for (const invalid of [
    { ...complete, data: [output] },
    { ...complete, _colossusToolObservation: null },
    { ...complete, preview: "private-extra-payload" },
    { ...complete, error: output.error },
  ])
    expect(approvalProcessDiagnostics([correlated(invalid)]).results).toEqual(
      [],
    );
});

test("receipt diagnostics require the matching assistant call and reject duplicate or foreign correlation", () => {
  const receipt = enveloped({
    error: { type: "denied", message: "permit timeout private-value" },
  });
  for (const request of [
    correlated(receipt, ["shell_run"], "foreign-call"),
    correlated(receipt, ["shell_run"], "approval-call", "foreign-call"),
    correlated(receipt, ["filesystem_read"]),
    correlated(receipt, ["shell_run", "shell_run"]),
    correlated(receipt, []),
    captured(receipt),
  ]) {
    const result = approvalProcessDiagnostics([request]);
    expect(result.results).toEqual([]);
    expect(JSON.stringify(result)).not.toContain("private-");
  }
});

test("complete success receipts preserve legacy diagnostics and normalize only once", () => {
  const output = { exit_code: 0, stdout: "private-output" };
  expect(
    approvalProcessDiagnostics([
      correlated(enveloped(output, { exitCode: 0 })),
    ]),
  ).toEqual(approvalProcessDiagnostics([captured(output)]));
  const nested = enveloped(
    enveloped({
      error: { type: "denied", message: "private-helper timeout" },
    }),
    { exitCode: 0 },
  );
  const result = approvalProcessDiagnostics([correlated(nested)]);
  expect(result.results).toEqual([
    expect.objectContaining({ category: "unclassified", timeout: false }),
  ]);
  expect(JSON.stringify(result)).not.toContain("private-");
});
