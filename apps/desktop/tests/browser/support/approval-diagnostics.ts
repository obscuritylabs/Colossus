// Test-owned provider capture only. Never return raw errors, output, arguments,
// identities, or credential-bearing text from a failed command.
export function approvalProcessDiagnostics(requests: readonly string[]) {
  const results: object[] = [];
  for (const request of requests.slice(-4)) {
    if (request.length > 2 * 1024 * 1024) continue;
    try {
      const messages: unknown = JSON.parse(request).messages;
      if (!Array.isArray(messages)) continue;
      for (const message of messages.slice(-64)) {
        if (message?.role !== "tool" || typeof message.content !== "string")
          continue;
        const output = JSON.parse(message.content);
        const error = output?.error;
        const category = [
          "execution_error",
          "validation_error",
          "denied",
          "outcome_unknown",
          "invalid_arguments",
          "unknown_tool",
        ].includes(error?.type)
          ? (error.type as string)
          : "unclassified";
        const detail = [error?.message, output?.stderr]
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.slice(0, 8192))
          .join(" ");
        results.push({
          category,
          exitCode:
            Number.isInteger(output?.exit_code) &&
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
