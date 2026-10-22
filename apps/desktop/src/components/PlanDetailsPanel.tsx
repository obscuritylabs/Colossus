import { PlanInspection } from "@colossus/ui/session/PlanInspection";
import { IconPlayerPlay, IconRefresh } from "@tabler/icons-react";

import type { SessionPlanReference } from "../session-resources";
import { canContinuePlan } from "../session-resources";
import { MarkdownContent } from "./MarkdownContent";

export function PlanDetailsPanel({
  plan,
  sessionId,
  continuationAvailable,
  workflowAvailable,
  onBack,
  onRevise,
  onOpenWorkflow,
}: {
  plan: SessionPlanReference;
  sessionId: string;
  continuationAvailable: boolean;
  workflowAvailable: boolean;
  onBack: () => void;
  onRevise: (sourceRunId: string, planId: string, revision: number) => void;
  onOpenWorkflow: (sessionId: string, planId: string) => void;
}) {
  return (
    <PlanInspection
      plan={plan}
      onBack={onBack}
      document={
        <MarkdownContent
          className="plan-details-markdown"
          content={plan.output}
        />
      }
      actions={
        <>
          <button
            type="button"
            disabled={!canContinuePlan(plan, continuationAvailable)}
            onClick={() =>
              onRevise(plan.sourceRunId, plan.planId, plan.revision)
            }
          >
            <IconRefresh size={14} stroke={1.7} aria-hidden="true" /> Revise in
            chat
          </button>
          <button
            type="button"
            disabled={!workflowAvailable}
            onClick={() => onOpenWorkflow(sessionId, plan.planId)}
          >
            <IconPlayerPlay size={14} stroke={1.7} aria-hidden="true" /> Open
            workflow
          </button>
        </>
      }
    />
  );
}
