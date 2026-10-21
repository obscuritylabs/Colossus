import { useState } from "react";
import { Button } from "@colossus/ui";
import {
  GoalControls,
  DEFAULT_GOAL_ITERATIONS,
  validGoalIterations,
} from "@colossus/ui/components/GoalControls";
import { MarkdownContent } from "@colossus/ui/conversation";
import { WorkflowDialog } from "@colossus/ui/automations/WorkflowDialog";
import {
  canContinuePlan,
  type SessionPlanReference,
} from "@colossus/ui/session/selectors";
import type { PlanStrategy } from "./plan-continuation";
import { WebLink } from "./WebLink";

/** Host-owned Plan decisions use the same public run admission as ordinary chat. */
export function ThreadPlanDecision({
  plan,
  available,
  goalAvailable,
  busy = false,
  onRevise,
  onExecute,
  inline = false,
}: {
  plan: SessionPlanReference;
  available: boolean;
  goalAvailable: boolean;
  busy?: boolean;
  onRevise: (plan: SessionPlanReference) => void;
  onExecute: (plan: SessionPlanReference, strategy: PlanStrategy) => void;
  inline?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [iterations, setIterations] = useState(DEFAULT_GOAL_ITERATIONS);
  const actionable = canContinuePlan(plan, available);
  const controls = actionable ? (
    <>
      <Button
        variant="secondary"
        disabled={busy}
        onClick={() => {
          setOpen(false);
          onRevise(plan);
        }}
      >
        Revise in chat
      </Button>
      <Button
        disabled={busy}
        onClick={() => {
          setOpen(false);
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
              setOpen(false);
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
          <Button variant="secondary" onClick={() => setOpen(true)}>
            Read plan
          </Button>
        )}
        {controls}
      </div>
      {open ? (
        <WorkflowDialog
          title={plan.sourceRunTitle}
          busy={busy}
          onClose={() => setOpen(false)}
        >
          <MarkdownContent content={plan.output} linkComponent={WebLink} />
          <div className="thread-plan-actions">
            {controls}
            <Button variant="secondary" onClick={() => setOpen(false)}>
              Close plan
            </Button>
          </div>
        </WorkflowDialog>
      ) : null}
    </section>
  );
}
