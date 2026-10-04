import {
  RunMode,
  planStatusToJSON,
  runModeToJSON,
  runStatusToJSON,
  sessionActivityKindToJSON,
  sessionActivityLaneToJSON,
  sessionActivityStatusToJSON,
  type Run,
  type SessionActivity,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";
import type { ActivityView, PlanView, RunView } from "./model.js";

const label = (value: string, prefix: string) =>
  value.replace(prefix, "").toLowerCase().replaceAll("_", " ");
const date = (value: Date | undefined) => value?.toISOString() ?? "";

// Only explicit public fields cross into renderers; etags and native request
// objects stay in the extension host. Decimal strings preserve uint64 precision.
export function runView(run: Run): RunView {
  return {
    id: run.runId,
    sessionId: run.sessionId,
    title: run.title || "Untitled conversation",
    role: run.role,
    mode: label(runModeToJSON(run.mode), "RUN_MODE_"),
    status: label(runStatusToJSON(run.status), "RUN_STATUS_"),
    createdAt: date(run.createdAt),
    updatedAt: date(run.updatedAt),
    startedAt: date(run.startedAt),
    finishedAt: date(run.finishedAt),
    sequence: run.lastSequence.toString(),
    pendingInteractions: run.pendingInteractionCount,
  };
}
export function planView(run: Run): PlanView | undefined {
  const terminal = run.terminal;
  if (terminal?.$case !== "result" && terminal?.$case !== "cancellation")
    return;
  const value = terminal.value;
  if (
    !value.planId ||
    value.planRevision === undefined ||
    value.planRevision <= 0n
  )
    return;
  return {
    id: value.planId,
    sourceRunId: run.runId,
    sessionId: run.sessionId,
    title: run.title || "Untitled plan",
    revision: value.planRevision.toString(),
    status: label(planStatusToJSON(value.planStatus), "PLAN_STATUS_"),
    goalId: value.goalId ?? "",
  };
}
export function plansFor(runs: Run[]): PlanView[] {
  const plans = new Map<string, PlanView>();
  // Runs arrive newest first. At equal revision the newest canonical state wins.
  for (const run of runs) {
    const plan = planView(run);
    const previous = plan && plans.get(plan.id);
    if (
      plan &&
      (!previous || BigInt(plan.revision) > BigInt(previous.revision))
    )
      plans.set(plan.id, plan);
  }
  return [...plans.values()];
}
export function planOutputRun(runs: Run[], plan: PlanView): Run | undefined {
  return runs.find((run) => {
    const reference = planView(run);
    return (
      run.mode === RunMode.RUN_MODE_PLAN &&
      reference?.id === plan.id &&
      reference.revision === plan.revision
    );
  });
}
export function activityView(activity: SessionActivity): ActivityView {
  return {
    id: activity.activityId,
    title: activity.title,
    summary: activity.summary,
    kind: label(
      sessionActivityKindToJSON(activity.kind),
      "SESSION_ACTIVITY_KIND_",
    ),
    lane: label(
      sessionActivityLaneToJSON(activity.lane),
      "SESSION_ACTIVITY_LANE_",
    ),
    status:
      activity.status === undefined
        ? ""
        : label(
            sessionActivityStatusToJSON(activity.status),
            "SESSION_ACTIVITY_STATUS_",
          ),
    startedAt: date(activity.startedAt),
    completedAt: date(activity.completedAt),
    result: activity.result?.value ?? "",
  };
}
