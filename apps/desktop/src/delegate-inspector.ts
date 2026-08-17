import { safeDisplayLabel } from "./presenters";
import type { RunView } from "./state";
import type { ThreadDelegateInspection, ToolActivityState } from "./types";

const MAX_DELEGATE_ACTIVITIES = 24;

export interface DelegateActivityItem {
  callId: string;
  title: string;
  toolName: string;
  state: ToolActivityState;
  durationLabel: string;
  createdAt: string;
  input: string;
  preview: string;
}

interface MutableDelegateActivity extends DelegateActivityItem {
  startedAtMs: number | null;
  finishedAtMs: number | null;
  order: number;
}

const TOOL_TITLES: Readonly<Record<string, string>> = {
  "filesystem.list": "Listed workspace files",
  "filesystem.read": "Read workspace files",
  "repo.file_summary": "Reviewed file summary",
  "repo.map": "Mapped repository structure",
  "repo.search": "Searched repository",
  "shell.run": "Ran a shell command",
  "web.search": "Searched web sources",
  "agent.result": "Returned findings",
};

function activityTitle(toolName: string): string {
  const known = TOOL_TITLES[toolName];
  if (known !== undefined) {
    return known;
  }
  const readable = toolName.replaceAll(".", " ").replaceAll("_", " ");
  const label = safeDisplayLabel(readable, "Tool activity", 72);
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function timestampMs(timestamp: string): number | null {
  const value = Date.parse(timestamp);
  return Number.isFinite(value) ? value : null;
}

function elapsedLabel(startedAtMs: number | null, finishedAtMs: number | null) {
  if (
    startedAtMs === null ||
    finishedAtMs === null ||
    finishedAtMs < startedAtMs
  ) {
    return "";
  }
  const seconds = Math.max(0, (finishedAtMs - startedAtMs) / 1_000);
  return seconds < 10 ? `${seconds.toFixed(1)}s` : `${Math.round(seconds)}s`;
}

export function selectDelegateActivities(
  view: RunView | undefined,
): readonly DelegateActivityItem[] {
  if (view === undefined) {
    return [];
  }

  const calls = new Map<string, MutableDelegateActivity>();
  let order = 0;
  for (const update of view.updates) {
    if (update.update.type !== "tool_activity") {
      continue;
    }
    const activity = update.update.activity;
    const observedAtMs = timestampMs(update.createdAt);
    const current = calls.get(activity.callId);
    const startedAtMs =
      activity.state === "started"
        ? observedAtMs
        : (current?.startedAtMs ?? null);
    const finishedAtMs =
      activity.state === "completed" ||
      activity.state === "cancelled" ||
      activity.state === "failed" ||
      activity.state === "outcome_unknown"
        ? observedAtMs
        : (current?.finishedAtMs ?? null);
    calls.set(activity.callId, {
      callId: activity.callId,
      title: activityTitle(activity.toolName),
      toolName: safeDisplayLabel(activity.toolName, "tool", 64),
      state: activity.state,
      durationLabel: elapsedLabel(startedAtMs, finishedAtMs),
      createdAt: current?.createdAt ?? update.createdAt,
      input: activity.input ?? current?.input ?? "",
      preview: activity.preview ?? current?.preview ?? "",
      startedAtMs,
      finishedAtMs,
      order: current?.order ?? order++,
    });
  }

  return [...calls.values()]
    .sort((left, right) => left.order - right.order)
    .slice(-MAX_DELEGATE_ACTIVITIES)
    .map(
      ({
        startedAtMs: _started,
        finishedAtMs: _finished,
        order: _order,
        ...item
      }) => item,
    );
}

export function selectInspectedDelegateActivities(
  inspection: ThreadDelegateInspection | null,
): readonly DelegateActivityItem[] {
  if (inspection === null) {
    return [];
  }
  return inspection.activities
    .slice(-MAX_DELEGATE_ACTIVITIES)
    .map((activity) => {
      const startedAtMs = timestampMs(activity.startedAt);
      const finishedAtMs = timestampMs(activity.completedAt ?? "");
      return {
        callId: activity.callId,
        title: activityTitle(activity.toolName),
        toolName: safeDisplayLabel(activity.toolName, "tool", 64),
        state: activity.state,
        durationLabel: elapsedLabel(startedAtMs, finishedAtMs),
        createdAt: activity.startedAt,
        input: activity.input ?? "",
        preview: activity.preview ?? "",
      };
    });
}
