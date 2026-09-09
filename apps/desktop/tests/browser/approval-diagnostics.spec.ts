import { expect, test } from "@playwright/test";
import { approvalProcessDiagnostics } from "./support/approval-diagnostics";

function captured(output: unknown) {
  return JSON.stringify({
    messages: [{ role: "tool", content: JSON.stringify(output) }],
  });
}

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
