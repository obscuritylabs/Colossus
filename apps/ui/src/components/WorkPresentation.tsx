import type { ReactNode, Ref } from "react";
import { IconClock } from "@tabler/icons-react";

export type SessionWorkspaceView =
  | "conversation"
  | "topology"
  | "plans"
  | "snapshots"
  | "activity"
  | "inboxes"
  | "sources"
  | "resources";
export const SESSION_WORKSPACE_VIEWS: readonly {
  id: SessionWorkspaceView;
  label: string;
}[] = [
  { id: "conversation", label: "Conversation" },
  { id: "topology", label: "Topology" },
  { id: "plans", label: "Plans" },
  { id: "snapshots", label: "Snapshots" },
  { id: "activity", label: "Activity" },
  { id: "inboxes", label: "Inboxes" },
  { id: "sources", label: "Sources" },
  { id: "resources", label: "Resources" },
];
export function SessionWorkspaceTabs({
  active,
  onChange,
}: {
  active: SessionWorkspaceView;
  onChange: (view: SessionWorkspaceView) => void;
}) {
  return (
    <nav className="session-workspace-tabs" aria-label="Session views">
      {SESSION_WORKSPACE_VIEWS.map((tab) => (
        <button
          key={tab.id}
          type="button"
          aria-current={active === tab.id ? "page" : undefined}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}
export interface WorkStatusPresentation {
  label: string;
  copy: string;
  tone: "neutral" | "progress" | "attention" | "success" | "danger";
}
export const WORK_STATUS_PRESENTATIONS: Readonly<
  Record<string, WorkStatusPresentation>
> = {
  queued: { label: "Queued", copy: "Waiting to start", tone: "neutral" },
  running: {
    label: "In progress",
    copy: "Work is in progress",
    tone: "progress",
  },
  waiting: {
    label: "Needs input",
    copy: "Waiting for your input",
    tone: "attention",
  },
  cancelling: { label: "Stopping", copy: "Stopping safely", tone: "attention" },
  completed: { label: "Completed", copy: "Work completed", tone: "success" },
  failed: { label: "Failed", copy: "Work failed", tone: "danger" },
  cancelled: { label: "Cancelled", copy: "Work cancelled", tone: "neutral" },
  interrupted: {
    label: "Interrupted",
    copy: "Work was interrupted",
    tone: "attention",
  },
  outcome_unknown: {
    label: "Outcome unknown",
    copy: "Verify the external outcome before retrying",
    tone: "danger",
  },
};
export function WorkSurfaceHeader({
  title,
  titleId,
  titleRef,
  statusLabel,
  status,
  startedLabel,
  modeLabel,
  breadcrumbTrailing,
  actions,
  editing,
}: {
  title: string;
  titleId?: string;
  titleRef?: Ref<HTMLHeadingElement>;
  statusLabel: string;
  status?: WorkStatusPresentation | undefined;
  startedLabel?: string | null | undefined;
  modeLabel?: string | undefined;
  breadcrumbTrailing?: ReactNode;
  actions?: ReactNode;
  editing?: ReactNode;
}) {
  return (
    <header className="surface-header work-surface-header shared-work-header">
      <div className="surface-title-copy">
        <p className="surface-breadcrumb">
          <span>Work</span>
          <span aria-hidden="true">/</span>
          <span>{statusLabel}</span>
          {breadcrumbTrailing}
        </p>
        {editing ?? (
          <h2 id={titleId} ref={titleRef} tabIndex={titleRef ? -1 : undefined}>
            {title}
          </h2>
        )}
        {status ? (
          <p className="surface-run-meta">
            <span className={"tone-" + status.tone}>{status.copy}</span>
            {startedLabel ? (
              <span>
                <IconClock size={12} stroke={1.7} aria-hidden="true" />
                Started {startedLabel}
              </span>
            ) : null}
            {modeLabel ? <span>{modeLabel}</span> : null}
          </p>
        ) : null}
      </div>
      <div className="surface-header-actions">{actions}</div>
    </header>
  );
}
