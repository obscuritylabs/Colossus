export interface ConversationUsage {
  inputTokens: number | string;
  outputTokens: number | string;
  totalTokens: number | string;
  cachedInputTokens?: number | string | null | undefined;
  reasoningTokens?: number | string | null | undefined;
}

/** Keep normalized provider counts exact, including SDK uint64 strings. */
export function tokenCount(value: unknown): bigint | null {
  if (typeof value === "number")
    return Number.isSafeInteger(value) && value >= 0 ? BigInt(value) : null;
  return typeof value === "string" && /^(?:0|[1-9][0-9]{0,31})$/u.test(value)
    ? BigInt(value)
    : null;
}

/** Add released usage reports; absent optional accounting stays unknown. */
export function sumConversationUsage(
  reports: readonly ConversationUsage[],
): ConversationUsage | null {
  if (!reports.length) return null;
  const result: ConversationUsage = {
    inputTokens: "0",
    outputTokens: "0",
    totalTokens: "0",
  };
  for (const key of [
    "inputTokens",
    "outputTokens",
    "totalTokens",
    "cachedInputTokens",
    "reasoningTokens",
  ] as const) {
    const values = reports.map((report) => tokenCount(report[key]));
    if (values.some((value) => value === null)) {
      if (["inputTokens", "outputTokens", "totalTokens"].includes(key))
        return null;
      continue;
    }
    result[key] = values
      .reduce<bigint>((total, value) => total + value!, 0n)
      .toString();
  }
  return result;
}
