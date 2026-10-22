import type { SessionPlanReference } from "@colossus/ui/session/selectors";

export type PlanStrategy = "direct" | { goal: { max_iterations: number } };
export type PlanRunAction =
  | { revise: { source_run_id: string; expected_revision: number } }
  | {
      execute: {
        source_run_id: string;
        expected_revision: number;
        strategy: PlanStrategy;
      };
    };

/** This selection is reviewed in the composer before a durable request is made. */
export interface PlanContinuation {
  plan: SessionPlanReference;
  role: string;
  strategy?: PlanStrategy;
}

export function planAction(selection: PlanContinuation): PlanRunAction {
  const reference = {
    source_run_id: selection.plan.sourceRunId,
    expected_revision: selection.plan.revision,
  };
  return selection.strategy
    ? { execute: { ...reference, strategy: selection.strategy } }
    : { revise: reference };
}

export function planPrompt(selection: PlanContinuation): string {
  const revision = selection.plan.revision;
  return !selection.strategy
    ? ""
    : selection.strategy === "direct"
      ? `Approve and execute Plan revision ${revision} once.`
      : `Approve and run Plan revision ${revision} as a bounded Goal with ${selection.strategy.goal.max_iterations} iterations.`;
}
