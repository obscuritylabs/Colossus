export interface WorkflowContext {
  selection_epoch: number;
  workflows_read: boolean;
  workflows_register: boolean;
  schedules_read: boolean;
  schedules_create: boolean;
  schedules_control: boolean;
  workflow_runs_read: boolean;
  managed: boolean;
}
export interface RegisteredWorkflow {
  workflow_id: string;
  name: string;
  version: string;
  workflow_hash: string;
  description: string;
  input_schema: Record<string, unknown> | null;
  scheduling_eligible: boolean;
  unavailable_reason: string | null;
}
export interface WorkflowPage<T> {
  items: T[];
  next_cursor: string | null;
}
export type MisfirePolicy = "fire_once" | "skip";
export interface ScheduleRecord {
  schedule_id: string;
  workflow_name: string;
  workflow_version: string;
  workflow_hash: string;
  inputs: Record<string, unknown> | null;
  cadence_seconds: number;
  misfire_policy: MisfirePolicy;
  enabled: boolean;
  starts_at: string;
  next_fire_at: string;
  last_scheduled_at: string | null;
  last_run_id: string | null;
  blocked_reason: string | null;
  created_at: string;
  updated_at: string;
}
export interface WorkflowSchedule {
  record: ScheduleRecord;
  origin: {
    owner: { actor_type: string; id: string };
    session_id: string | null;
    run_id: string | null;
  } | null;
  etag: string;
  controllable: boolean;
  last_dispatch: "queued" | "skipped" | "blocked" | null;
}
export interface WorkflowRun {
  run_id: string;
  workflow_id: string;
  workflow_hash: string;
  status:
    | "queued"
    | "running"
    | "waiting"
    | "completed"
    | "failed"
    | "cancelled"
    | "interrupted";
  created_at: string;
  updated_at: string;
  last_sequence: number;
  failure_reason: string | null;
  waiting_reason: string | null;
}
export interface CreateScheduleRequest {
  schedule_id: string;
  workflow_id: string;
  expected_hash: string;
  inputs: Record<string, unknown>;
  cadence_seconds: number;
  starts_at: string;
  misfire_policy: MisfirePolicy;
  enabled: boolean;
  idempotency_key: string;
}
export function firstOccurrence(value: string, zone: "local" | "utc"): string {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))
    throw new Error("Choose a complete first occurrence.");
  const [year, month, day, hour, minute] = value.split(/[-T:]/).map(Number) as [
    number,
    number,
    number,
    number,
    number,
  ];
  const date =
    zone === "utc"
      ? new Date(`${value}:00Z`)
      : new Date(year, month - 1, day, hour, minute);
  const matches = (date: Date) =>
    zone === "utc"
      ? date.getUTCFullYear() === year &&
        date.getUTCMonth() === month - 1 &&
        date.getUTCDate() === day &&
        date.getUTCHours() === hour &&
        date.getUTCMinutes() === minute
      : date.getFullYear() === year &&
        date.getMonth() === month - 1 &&
        date.getDate() === day &&
        date.getHours() === hour &&
        date.getMinutes() === minute;
  if (!Number.isFinite(date.getTime()) || !matches(date))
    throw new Error(
      "This date/time does not exist. Choose another time or use UTC.",
    );
  if (zone === "local") {
    // Detect a repeated local wall time across any offset transition within a day.
    for (let minutes = -1440; minutes <= 1440; minutes++) {
      if (minutes && matches(new Date(date.getTime() + minutes * 60000)))
        throw new Error(
          "This local time occurs twice across a clock change. Use UTC to select the exact occurrence.",
        );
    }
  }
  return date.toISOString();
}
export function scheduleInputs(text: string): Record<string, unknown> {
  if (new TextEncoder().encode(text).length > 65536)
    throw new Error("Inputs must be no larger than 64 KiB.");
  const value: unknown = JSON.parse(text);
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Inputs must be a JSON object.");
  return value as Record<string, unknown>;
}
/** Compare immutable JSON intent independently of object insertion order. */
export function canonicalJson(value: unknown): string {
  const ordered = (value: unknown): unknown =>
    Array.isArray(value)
      ? value.map(ordered)
      : value !== null && typeof value === "object"
        ? Object.fromEntries(
            Object.entries(value)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([key, value]) => [key, ordered(value)]),
          )
        : value;
  return JSON.stringify(ordered(value));
}
export function cadence(seconds: number): string {
  const value = seconds % 3600 === 0 ? seconds / 3600 : seconds / 60;
  const unit = seconds % 3600 === 0 ? "hour" : "minute";
  return `Every ${value === 1 ? "" : `${value} `}${unit}${value === 1 ? "" : "s"}`;
}
export function occurrence(value: string | null): string {
  if (!value) return "None yet";
  return `${new Date(value).toLocaleString()} (${new Date(value).toISOString()} UTC)`;
}
export function workflowFailure(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "The workflow request failed. Refresh the selected runtime and try again.";
}
export const MISFIRE_GUIDANCE =
  "With one due occurrence, both options queue a run. With multiple due occurrences, Fire once queues the latest once; Skip queues none and advances to the next future boundary.";
