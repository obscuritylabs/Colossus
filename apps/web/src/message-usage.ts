import {
  sumConversationUsage,
  tokenCount,
  type ConversationUsage,
} from "@colossus/ui/lib/conversation-usage";
import type { Update } from "./api";

/** Input must already be fenced to one task/run by conversationProjection. */
export function releasedRunUsage(
  updates: readonly Update[],
): ConversationUsage | null {
  const reports: ConversationUsage[] = [];
  for (const event of updates) {
    const raw = event.update.usage;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const usage = raw as Record<string, unknown>;
    const input = tokenCount(usage.input_tokens);
    const output = tokenCount(usage.output_tokens);
    const total = tokenCount(usage.total_tokens);
    if (input === null || output === null || total === null) continue;
    reports.push({
      inputTokens: input.toString(),
      outputTokens: output.toString(),
      totalTokens: total.toString(),
      cachedInputTokens: tokenCount(usage.cached_input_tokens)?.toString(),
      reasoningTokens: tokenCount(usage.reasoning_tokens)?.toString(),
    });
  }
  return sumConversationUsage(reports);
}
