import type { BackgroundNotificationKind, StatusBarPin } from "./api";
import { safeDisplayLabel } from "./presenters";
import type { Run } from "./types";

const MAX_STATUS_BAR_PINS = 10;

export function selectStatusBarPins(
  sessionIds: readonly string[],
  runs: readonly Run[],
  resolveTitle: (sessionId: string, fallback: string) => string,
): StatusBarPin[] {
  const latestBySession = new Map<string, Run>();
  for (const run of runs) {
    if (run.archived) {
      continue;
    }
    const previous = latestBySession.get(run.sessionId);
    if (
      previous === undefined ||
      Date.parse(run.updatedAt) > Date.parse(previous.updatedAt)
    ) {
      latestBySession.set(run.sessionId, run);
    }
  }
  return sessionIds
    .flatMap((sessionId) => {
      const run = latestBySession.get(sessionId);
      if (run === undefined) {
        return [];
      }
      return [
        {
          runId: run.runId,
          title: safeDisplayLabel(
            resolveTitle(sessionId, run.title),
            "Untitled work",
            72,
          ),
        },
      ];
    })
    .slice(0, MAX_STATUS_BAR_PINS);
}

export interface BackgroundRunState {
  status: Run["status"];
  pendingInteractionCount: number;
}

export function backgroundRunNotifications(
  previous: ReadonlyMap<string, BackgroundRunState>,
  runs: readonly Run[],
): { kind: BackgroundNotificationKind; runId: string }[] {
  const notifications: {
    kind: BackgroundNotificationKind;
    runId: string;
  }[] = [];
  for (const run of runs) {
    const before = previous.get(run.runId);
    if (before === undefined || run.archived) {
      continue;
    }
    const wasWaiting =
      before.status === "waiting" || before.pendingInteractionCount > 0;
    const isWaiting =
      run.status === "waiting" || run.pendingInteractionCount > 0;
    if (!wasWaiting && isWaiting) {
      notifications.push({ kind: "needs_attention", runId: run.runId });
    } else if (before.status !== "completed" && run.status === "completed") {
      notifications.push({ kind: "work_completed", runId: run.runId });
    } else if (
      before.status !== "failed" &&
      before.status !== "outcome_unknown" &&
      (run.status === "failed" || run.status === "outcome_unknown")
    ) {
      notifications.push({ kind: "work_failed", runId: run.runId });
    }
  }
  return notifications;
}

export function backgroundRunSnapshot(
  runs: readonly Run[],
): ReadonlyMap<string, BackgroundRunState> {
  return new Map(
    runs.map((run) => [
      run.runId,
      {
        status: run.status,
        pendingInteractionCount: run.pendingInteractionCount,
      },
    ]),
  );
}
