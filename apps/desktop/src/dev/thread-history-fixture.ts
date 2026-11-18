import type { Run, RunPage } from "../types";
import { CommandFailure } from "../api";

let requests = 0;
let failed = false;
const cursors: string[] = [];
let inFlight = 0;
let maximumInFlight = 0;

export async function readThreadHistoryFixture(
  cursor: string,
): Promise<RunPage> {
  const query = new URLSearchParams(window.location.search);
  const total = Math.min(
    10_000,
    Math.max(3, Number(query.get("historyCount") ?? 4505)),
  );
  const offset = cursor === "" ? 0 : Number(cursor.slice("history:".length));
  const size =
    offset === 0
      ? Math.min(total, Math.max(3, Number(query.get("historyLoaded") ?? 3)))
      : 3;
  requests += 1;
  cursors.push(cursor);
  inFlight += 1;
  maximumInFlight = Math.max(maximumInFlight, inFlight);
  document.documentElement.dataset.historyRequests = String(requests);
  document.documentElement.dataset.historyMaximumInFlight =
    String(maximumInFlight);
  document.documentElement.dataset.historyCursorLog = JSON.stringify(cursors);
  document.documentElement.dataset.historyLastCursor = cursor;
  await new Promise((resolve) => window.setTimeout(resolve, 25));
  inFlight -= 1;
  if (!failed && query.get("historyFailAt") === String(offset)) {
    failed = true;
    throw new CommandFailure({
      code: "fixture_history_failure",
      message: "History fixture temporarily unavailable.",
      retryable: true,
      outcomeUnknown: false,
      violations: [],
    });
  }
  const runs: Run[] = Array.from(
    { length: Math.min(size, total - offset) },
    (_, index) => {
      const number = offset + index + 1;
      const timestamp = new Date(
        Date.UTC(2026, 10, 18) - number * 60_000,
      ).toISOString();
      return {
        runId: `history-run-${number}`,
        sessionId: `history-session-${number}`,
        title: `History thread ${String(number).padStart(5, "0")}`,
        role: "primary",
        mode: "execute",
        status: "completed",
        createdAt: timestamp,
        updatedAt: timestamp,
        startedAt: timestamp,
        finishedAt: timestamp,
        lastSequence: 4,
        pendingInteractionCount: 0,
        terminal: null,
        etag: `history-${number}`,
        archived: false,
      };
    },
  );
  const next = offset + runs.length;
  return { runs, nextPageToken: next < total ? `history:${next}` : "" };
}
