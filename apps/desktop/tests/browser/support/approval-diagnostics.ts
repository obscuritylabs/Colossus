// Test-owned provider capture only. Never return raw errors, output, arguments,
// identities, or credential-bearing text from a failed command.
type CapturedObject = Record<string, unknown>;

function object(value: unknown): CapturedObject | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as CapturedObject)
    : null;
}

function diagnosticOutput(
  message: CapturedObject,
  messages: unknown[],
): CapturedObject | null {
  const output = object(JSON.parse(message.content as string));
  if (!output) return {};
  if (!("_colossusToolObservation" in output)) return output;
  const metadata = object(output._colossusToolObservation);
  if (
    Object.keys(output).some(
      (key) => key !== "_colossusToolObservation" && key !== "data",
    ) ||
    !metadata ||
    Object.keys(metadata).some(
      (key) =>
        ![
          "truncated",
          "format",
          "toolName",
          "callId",
          "exitCode",
          "originalBytes",
          "sha256",
          "identity",
        ].includes(key),
    ) ||
    metadata.truncated !== false ||
    metadata.format !== "json" ||
    typeof metadata.toolName !== "string" ||
    !metadata.toolName.length ||
    metadata.toolName.length > 128 ||
    typeof metadata.callId !== "string" ||
    !metadata.callId.length ||
    metadata.callId.length > 128 ||
    metadata.callId !== message.tool_call_id ||
    typeof metadata.exitCode !== "number" ||
    !Number.isInteger(metadata.exitCode) ||
    metadata.exitCode < -(2 ** 31) ||
    metadata.exitCode >= 2 ** 31 ||
    typeof metadata.originalBytes !== "number" ||
    !Number.isSafeInteger(metadata.originalBytes) ||
    metadata.originalBytes < 0 ||
    typeof metadata.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/iu.test(metadata.sha256) ||
    (metadata.identity !== undefined && !object(metadata.identity))
  )
    return null;
  const assistants = messages
    .map(object)
    .filter((item): item is CapturedObject => item?.role === "assistant");
  if (assistants.length) {
    const calls = assistants.flatMap((assistant) =>
      Array.isArray(assistant.tool_calls)
        ? assistant.tool_calls
            .map(object)
            .filter(
              (call): call is CapturedObject =>
                call !== null &&
                call.id === metadata.callId &&
                call.type === "function",
            )
        : [],
    );
    const name = calls.length === 1 ? object(calls[0]?.function)?.name : null;
    // Compatible-provider builtins use underscore aliases; accept that exact
    // alias or the canonical name, never a different tool with the same call ID.
    if (
      name !== metadata.toolName &&
      name !== metadata.toolName.replaceAll(".", "_")
    )
      return null;
  }
  const data = object(output.data);
  if (!data) return null;
  // Gateway errors need not carry a process exit_code. The validated ToolResult
  // status fills only an absent field; explicit running/null receipts stay intact.
  return data.exit_code === undefined
    ? { ...data, exit_code: metadata.exitCode }
    : data;
}

export function approvalProcessDiagnostics(requests: readonly string[]) {
  const results: object[] = [];
  for (const request of requests.slice(-4)) {
    if (request.length > 2 * 1024 * 1024) continue;
    try {
      const messages: unknown = JSON.parse(request).messages;
      if (!Array.isArray(messages)) continue;
      for (const entry of messages.slice(-64)) {
        const message = object(entry);
        if (message?.role !== "tool" || typeof message.content !== "string")
          continue;
        const output = diagnosticOutput(message, messages);
        if (!output) continue;
        const error = object(output.error);
        const errorType = error?.type;
        const category =
          typeof errorType === "string" &&
          [
            "execution_error",
            "validation_error",
            "denied",
            "outcome_unknown",
            "invalid_arguments",
            "unknown_tool",
          ].includes(errorType)
            ? errorType
            : "unclassified";
        const detail = [error?.message, output?.stderr]
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.slice(0, 8192))
          .join(" ");
        results.push({
          category,
          exitCode:
            typeof output.exit_code === "number" &&
            Number.isInteger(output.exit_code) &&
            Math.abs(output.exit_code) <= 2 ** 32
              ? (output.exit_code as number)
              : null,
          timeout: /timeout|timed out|deadline/iu.test(detail),
          memoryLimit: /memory|resource limit/iu.test(detail),
          accessDenied: /access.*denied|permission denied/iu.test(detail),
          appContainer: /appcontainer/iu.test(detail),
          cleanup: /cleanup|termination|remove.*ACL/iu.test(detail),
          helper: /sandbox helper/iu.test(detail),
          permit: /permit/iu.test(detail),
        });
      }
    } catch {
      // Malformed capture is unavailable, never echoed into the test report.
    }
  }
  return { providerRequests: requests.length, results: results.slice(-4) };
}
