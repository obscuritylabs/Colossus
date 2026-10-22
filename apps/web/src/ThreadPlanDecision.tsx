import { useState } from "react";
import { Button } from "@colossus/ui";
import {
  GoalControls,
  DEFAULT_GOAL_ITERATIONS,
  validGoalIterations,
} from "@colossus/ui/components/GoalControls";
import {
  canContinuePlan,
  type SessionPlanReference,
} from "@colossus/ui/session/selectors";
import type { PlanStrategy } from "./plan-continuation";

/** Host-owned Plan decisions use the same public run admission as ordinary chat. */
export function ThreadPlanDecision({
  plan,
  available,
  goalAvailable,
  busy = false,
  onRevise,
  onExecute,
  inline = false,
  onInspect,
}: {
  plan: SessionPlanReference;
  available: boolean;
  goalAvailable: boolean;
  busy?: boolean;
  onRevise: (plan: SessionPlanReference) => void;
  onExecute: (plan: SessionPlanReference, strategy: PlanStrategy) => void;
  inline?: boolean;
  onInspect?: (plan: SessionPlanReference) => void;
}) {
  const [iterations, setIterations] = useState(DEFAULT_GOAL_ITERATIONS);
  const actionable = canContinuePlan(plan, available);
  const controls = actionable ? (
    <>
      <Button
        variant="secondary"
        disabled={busy}
        onClick={() => {
          onRevise(plan);
        }}
      >
        Revise in chat
      </Button>
      <Button
        disabled={busy}
        onClick={() => {
          onExecute(plan, "direct");
        }}
      >
        Execute once
      </Button>
      {goalAvailable ? (
        <details className="plan-goal-decision">
          <summary>Execute as bounded Goal</summary>
          <GoalControls
            value={iterations}
            disabled={busy}
            onChange={setIterations}
          />
          <Button
            disabled={busy || !validGoalIterations(iterations)}
            onClick={() => {
              onExecute(plan, { goal: { max_iterations: iterations } });
            }}
          >
            Review Goal execution
          </Button>
        </details>
      ) : null}
    </>
  ) : null;
  return (
    <section
      className="thread-plan-decision"
      aria-label={`Plan revision ${plan.revision} decisions`}
    >
      <p>
        <strong>Plan revision {plan.revision}</strong> ·{" "}
        {plan.status ?? "Status unavailable"}
      </p>
      {actionable ? (
        <p>
          Review the plan, then choose how to continue. Execution requires your
          approval in the composer.
        </p>
      ) : null}
      <div className="thread-plan-actions">
        {inline ? null : (
          <Button
            variant="secondary"
            onClick={() => onInspect?.(plan)}
            disabled={!onInspect}
          >
            Read plan
          </Button>
        )}
        {controls}
      </div>
    </section>
  );
}
