import { IconChartBar } from "@tabler/icons-react";
import {
  tokenCount,
  type ConversationUsage,
} from "../lib/conversation-usage.js";

export function ConversationUsageDetails({
  usage,
  label = "Reported run usage",
}: {
  usage: ConversationUsage;
  label?: string | undefined;
}) {
  const input = tokenCount(usage.inputTokens);
  const output = tokenCount(usage.outputTokens);
  const total = tokenCount(usage.totalTokens);
  if (input === null || output === null || total === null) return null;
  const cached = tokenCount(usage.cachedInputTokens);
  const reasoning = tokenCount(usage.reasoningTokens);
  return (
    <details className="shared-message-usage">
      <summary
        aria-label={`${label}: ${total.toLocaleString()} tokens; ${input.toLocaleString()} input, ${output.toLocaleString()} output`}
      >
        <IconChartBar size={14} stroke={1.8} aria-hidden="true" />
        <span>{total.toLocaleString()} tokens</span>
        <span className="shared-usage-breakdown">
          · {input.toLocaleString()} in / {output.toLocaleString()} out
        </span>
      </summary>
      <div className="shared-message-usage-details">
        <strong>{label}</strong>
        <dl>
          <div>
            <dt>Input tokens</dt>
            <dd>{input.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Output tokens</dt>
            <dd>{output.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Total tokens</dt>
            <dd>{total.toLocaleString()}</dd>
          </div>
          {cached === null ? null : (
            <div>
              <dt>Cached input tokens</dt>
              <dd>{cached.toLocaleString()}</dd>
            </div>
          )}
          {reasoning === null ? null : (
            <div>
              <dt>Reasoning tokens</dt>
              <dd>{reasoning.toLocaleString()}</dd>
            </div>
          )}
        </dl>
        <p>
          Provider-reported counts. Cached input and reasoning are included in
          input and output when reported.
        </p>
      </div>
    </details>
  );
}
