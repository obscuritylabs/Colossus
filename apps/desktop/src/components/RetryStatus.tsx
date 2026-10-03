import { IconRefresh } from "@tabler/icons-react";
import { useEffect, useState } from "react";

import type { RunView } from "../state";
import type { ProviderRetry } from "../types";

export function currentProviderRetry(view: RunView): ProviderRetry | null {
  if (view.run.status !== "running") return null;
  for (const { update } of [...view.updates].reverse()) {
    if (update.type === "provider_retry") {
      return update.retry.state === "recovered" ? null : update.retry;
    }
    if (
      update.type === "output_delta" ||
      update.type === "reasoning_summary" ||
      update.type === "tool_activity" ||
      (update.type === "notice" && update.reason.startsWith("run.phase."))
    ) {
      return null;
    }
  }
  return null;
}

export function RetryStatus({ retry }: { retry: ProviderRetry }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    setNow(Date.now());
    if (retry.state !== "backoff") return;
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [retry.state, retry.retry_at]);
  const deadline = Date.parse(retry.retry_at ?? "");
  const seconds = Number.isFinite(deadline)
    ? Math.max(0, Math.ceil((deadline - now) / 1_000))
    : 0;
  const waiting = retry.state === "backoff" && seconds > 0;

  return (
    <div
      className="feed-entry live-run-status provider-retry-status"
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      <span className="feed-marker" aria-hidden="true">
        <IconRefresh size={16} stroke={1.8} />
      </span>
      <span className="live-run-status-copy">
        <strong>Reconnecting to provider</strong>
        <small>
          Retry {retry.attempt} of {retry.max_retries}
          <span aria-hidden="true">
            {waiting ? ` · Next attempt in ${seconds}s` : " · Retrying now…"}
          </span>
        </small>
      </span>
    </div>
  );
}
