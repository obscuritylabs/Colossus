import type { ToolActivity } from "../types";
import { cadence } from "../workflows";

export type InspectSchedule = (scheduleId: string, showRun: boolean) => void;

function object(
  text: string | null | undefined,
): Record<string, unknown> | null {
  if (!text || text.length > 16_384) return null;
  try {
    const value: unknown = JSON.parse(text);
    return value !== null && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

/** Only released activity evidence supplies labels; inspection re-authorizes the resource. */
export function ScheduleActivityCard({
  activity,
  input,
  onInspect,
}: {
  activity: ToolActivity;
  input: string | null;
  onInspect?: InspectSchedule | undefined;
}) {
  if (
    ![
      "workflow.schedule.create",
      "workflow.schedule.get",
      "workflow.schedule.set_enabled",
    ].includes(activity.toolName)
  )
    return null;
  const released =
    activity.state === "completed" ? object(activity.preview) : null;
  const record =
    released?.record !== null &&
    typeof released?.record === "object" &&
    !Array.isArray(released.record)
      ? (released.record as Record<string, unknown>)
      : null;
  const intent = object(input);
  const id = record?.schedule_id ?? intent?.schedule_id;
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9.-]{0,127}$/.test(id))
    return null;
  const confirmed = record !== null && record.schedule_id === id;
  const seconds = record?.cadence_seconds;
  return (
    <article className="schedule-activity-card" aria-label="Schedule activity">
      <strong>
        {confirmed ? "Schedule" : "Schedule request"}: {id}
      </strong>
      <p>
        {confirmed
          ? `${record.enabled === true ? "Enabled" : "Paused"}${typeof seconds === "number" && Number.isSafeInteger(seconds) ? ` · ${cadence(seconds)}` : ""}`
          : activity.state === "outcome_unknown"
            ? "Outcome unconfirmed. Inspect stored state before retrying."
            : `Request ${activity.state.replaceAll("_", " ")}. Inspect for current state.`}
      </p>
      <p>Future occurrences start independent workflow runs.</p>
      {onInspect && (
        <div className="workflow-actions">
          <button
            className="button secondary compact"
            onClick={() => onInspect(id, false)}
          >
            Inspect schedule
          </button>
          {confirmed &&
            typeof record.last_run_id === "string" &&
            record.last_run_id && (
              <button
                className="button secondary compact"
                onClick={() => onInspect(id, true)}
              >
                Inspect last workflow run
              </button>
            )}
        </div>
      )}
    </article>
  );
}
