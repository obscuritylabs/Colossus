export * from "@colossus/ui/session/selectors";
import type { RunView } from "./state";
export function sessionActionCount(views: readonly RunView[]): number {
  const calls = new Set<string>();
  for (const view of views) {
    for (const update of view.updates) {
      if (update.update.type === "tool_activity") {
        calls.add(`${view.run.runId}:${update.update.activity.callId}`);
      }
    }
  }
  return calls.size;
}
