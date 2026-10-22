import { useId, type ReactNode } from "react";
import { IconArrowLeft, IconListDetails } from "@tabler/icons-react";
import type { SessionPlanReference } from "./selectors.js";
import "../../styles/plan-inspection.css";

export function PlanInspection({
  plan,
  document,
  actions,
  onBack,
}: {
  plan: SessionPlanReference;
  document: ReactNode;
  actions?: ReactNode;
  onBack?: () => void;
}) {
  const heading = useId();
  const planStatus = (value: SessionPlanReference) =>
    value.cancelled
      ? "Saved before cancellation"
      : value.status === null
        ? "Status not reported"
        : value.status.charAt(0).toUpperCase() + value.status.slice(1);
  return (
    <section
      className="thread-details-panel plan-details-panel"
      aria-labelledby={heading}
      data-aside-context="true"
      data-aside-source-run-id={plan.sourceRunId}
    >
      <header>
        {onBack ? (
          <button type="button" onClick={onBack}>
            <IconArrowLeft size={14} stroke={1.8} aria-hidden="true" />
            Thread details
          </button>
        ) : null}
        <div>
          <span className="plan-details-kicker">Plan</span>
          <h2 id={heading}>{plan.sourceRunTitle}</h2>
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
            <strong>Planning response</strong>
            <small>Response released by the planning run</small>
          </div>
        </div>
        {plan.output.trim() === "" ? (
          <p className="plan-details-empty">
            This plan was saved without a released preview.
          </p>
        ) : (
          <div data-aside-selectable="true">{document}</div>
        )}
      </section>

      {actions ? (
        <footer className="plan-details-actions">{actions}</footer>
      ) : null}
    </section>
  );
}
