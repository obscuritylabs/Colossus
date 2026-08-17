import {
  IconArrowLeft,
  IconListDetails,
  IconPlayerPlay,
  IconRefresh,
} from "@tabler/icons-react";

import type { SessionPlanReference } from "../session-resources";
import { MarkdownContent } from "./MarkdownContent";

function planStatus(plan: SessionPlanReference): string {
  if (plan.cancelled) {
    return "Saved before cancellation";
  }
  return plan.status === null
    ? "Draft"
    : plan.status.charAt(0).toUpperCase() + plan.status.slice(1);
}

export function PlanDetailsPanel({
  plan,
  sessionId,
  workflowAvailable,
  onBack,
  onRevise,
  onOpenWorkflow,
}: {
  plan: SessionPlanReference;
  sessionId: string;
  workflowAvailable: boolean;
  onBack: () => void;
  onRevise: (sourceRunId: string, planId: string, revision: number) => void;
  onOpenWorkflow: (sessionId: string, planId: string) => void;
}) {
  return (
    <aside
      className="thread-details-panel plan-details-panel"
      aria-labelledby="plan-details-title"
      data-aside-context="true"
      data-aside-source-run-id={plan.sourceRunId}
    >
      <header>
        <button type="button" onClick={onBack}>
          <IconArrowLeft size={14} stroke={1.8} aria-hidden="true" />
          Thread details
        </button>
        <div>
          <span className="plan-details-kicker">Plan</span>
          <h2 id="plan-details-title">{plan.sourceRunTitle}</h2>
          <p>
            Run {plan.runIndex} · Revision {plan.revision}
          </p>
        </div>
        <span className="plan-details-status">{planStatus(plan)}</span>
      </header>

      <section className="plan-details-content">
        <div className="plan-details-document-heading">
          <span aria-hidden="true">
            <IconListDetails size={18} stroke={1.6} />
          </span>
          <div>
            <strong>Released plan</strong>
            <small>Rendered from the durable plan output</small>
          </div>
        </div>
        {plan.output.trim() === "" ? (
          <p className="plan-details-empty">
            This plan was saved without a released preview.
          </p>
        ) : (
          <div data-aside-selectable="true">
            <MarkdownContent
              className="plan-details-markdown"
              content={plan.output}
            />
          </div>
        )}
      </section>

      <footer className="plan-details-actions">
        <button
          type="button"
          onClick={() => onRevise(plan.sourceRunId, plan.planId, plan.revision)}
        >
          <IconRefresh size={14} stroke={1.7} aria-hidden="true" />
          Revise in chat
        </button>
        <button
          type="button"
          disabled={!workflowAvailable}
          onClick={() => onOpenWorkflow(sessionId, plan.planId)}
        >
          <IconPlayerPlay size={14} stroke={1.7} aria-hidden="true" />
          Open workflow
        </button>
      </footer>
    </aside>
  );
}
