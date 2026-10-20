import {
  taskStatus,
  terminalStatuses,
  visibleOutput,
  type Task,
  type Update,
} from "@colossus/control-plane-client";

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
/** A terminal snapshot can retain the canonical released result after its event leaves the window. */
export function taskReleasedOutput(task: Task, updates: Update[]): string {
  if (terminalStatuses.has(taskStatus(task))) {
    const terminal = record(task.snapshot?.run.terminal);
    const result = record(terminal?.result) ?? terminal;
    if (typeof result?.output === "string") return result.output;
  }
  return visibleOutput(updates);
}
