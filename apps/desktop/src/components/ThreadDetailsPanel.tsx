import {
  IconAdjustments,
  IconAlertCircle,
  IconArrowLeft,
  IconBook2,
  IconCheck,
  IconChevronRight,
  IconFileText,
  IconFolder,
  IconLoader2,
  IconListDetails,
  IconPin,
  IconRobot,
  IconSearch,
  IconTerminal2,
  IconUser,
} from "@tabler/icons-react";
import { useState } from "react";

import {
  selectDelegateActivities,
  selectInspectedDelegateActivities,
} from "../delegate-inspector";
import {
  agentRoleLabel,
  presentToolState,
  presentRunStatus,
  runModeLabel,
  shortDateLabel,
} from "../presenters";
import type { RunView } from "../state";
import type { Run, ThreadDelegateInspection } from "../types";
import type { AgentParticipant } from "./AgentFlow";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import type { SessionWorkspaceView } from "./SessionWorkspace";

interface ThreadDetailsPanelProps {
  run: Run;
  spaceName: string;
  pinned: boolean;
  participants: readonly AgentParticipant[];
  files: readonly ArtifactViewItem[];
  selectedParticipantId: string | null;
  delegateView: RunView | undefined;
  delegateInspection: ThreadDelegateInspection | null;
  delegateLoading: boolean;
  delegateError: string;
  sessionRunCount: number;
  sessionPlanCount: number;
  sessionResourceCount: number;
  sessionSnapshotCount: number;
  sessionSourceCount: number;
  onSelectParticipant: (participant: AgentParticipant) => void;
  onBackToThread: () => void;
  onOpenSessionView: (view: SessionWorkspaceView) => void;
}

function durationLabel(run: Run): string {
  if (
    run.terminal?.type === "result" &&
    Number.isFinite(run.terminal.result.elapsedSeconds)
  ) {
    return elapsedLabel(run.terminal.result.elapsedSeconds);
  }

  const started = Date.parse(run.startedAt ?? run.createdAt);
  const ended = Date.parse(run.finishedAt ?? run.updatedAt);
  if (!Number.isFinite(started) || !Number.isFinite(ended) || ended < started) {
    return "—";
  }
  return elapsedLabel(Math.round((ended - started) / 1_000));
}

function elapsedLabel(seconds: number): string {
  const bounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(bounded / 60);
  const remainder = bounded % 60;
  return minutes > 0 ? `${minutes}m ${remainder}s` : `${remainder}s`;
}

function participantDurationLabel(
  participant: AgentParticipant,
  inspection: ThreadDelegateInspection | null,
): string {
  const started = Date.parse(
    inspection?.startedAt ??
      inspection?.createdAt ??
      participant.startedAt ??
      participant.createdAt ??
      "",
  );
  const ended = Date.parse(
    inspection?.completedAt ??
      inspection?.updatedAt ??
      participant.completedAt ??
      participant.updatedAt ??
      "",
  );
  if (!Number.isFinite(started) || !Number.isFinite(ended) || ended < started) {
    return "—";
  }
  return elapsedLabel(Math.round((ended - started) / 1_000));
}

function participantStatus(
  participant: AgentParticipant,
  view: RunView | undefined,
) {
  if (view !== undefined) {
    return presentRunStatus(view.run.status);
  }
  switch (participant.state) {
    case "working":
    case "coordinating":
    case "reviewing":
      return presentRunStatus("running");
    case "waiting":
      return presentRunStatus("queued");
    case "completed":
      return presentRunStatus("completed");
    case "failed":
      return presentRunStatus("failed");
    case "cancelled":
      return presentRunStatus("cancelled");
    case "idle":
      return presentRunStatus("queued");
  }
}

function toolIcon(toolName: string) {
  if (toolName === "repo.search" || toolName === "web.search") {
    return <IconSearch size={14} stroke={1.7} aria-hidden="true" />;
  }
  if (toolName === "shell.run") {
    return <IconTerminal2 size={14} stroke={1.7} aria-hidden="true" />;
  }
  return <IconFileText size={14} stroke={1.7} aria-hidden="true" />;
}

function DelegateRunDetails({
  participant,
  view,
  inspection,
  loading,
  error,
  spaceName,
  sessionRunCount,
  sessionPlanCount,
  sessionSnapshotCount,
  sessionSourceCount,
  fileCount,
  onOpenSessionView,
  onBack,
}: {
  participant: AgentParticipant;
  view: RunView | undefined;
  inspection: ThreadDelegateInspection | null;
  loading: boolean;
  error: string;
  spaceName: string;
  sessionRunCount: number;
  sessionPlanCount: number;
  sessionSnapshotCount: number;
  sessionSourceCount: number;
  fileCount: number;
  onOpenSessionView: (view: SessionWorkspaceView) => void;
  onBack: () => void;
}) {
  const [resultExpanded, setResultExpanded] = useState(false);
  const [delegateSection, setDelegateSection] = useState<
    "overview" | "activity"
  >("activity");
  const status =
    inspection === null
      ? participantStatus(participant, view)
      : presentRunStatus(inspection.status);
  const activities =
    view === undefined
      ? selectInspectedDelegateActivities(inspection)
      : selectDelegateActivities(view);
  const result =
    view?.output.trim() ||
    (view?.run.terminal?.type === "failure"
      ? view.run.terminal.failure.message
      : "") ||
    inspection?.finalOutput.trim() ||
    inspection?.error.trim() ||
    participant.finalOutput?.trim() ||
    participant.error?.trim() ||
    "";
  const completedWithoutResult =
    view === undefined && participant.state === "completed" && result === "";

  return (
    <>
      <header className="delegate-run-header">
        <button type="button" onClick={onBack}>
          <IconArrowLeft size={14} stroke={1.8} aria-hidden="true" />
          Thread details
        </button>
        <h2 id="thread-details-title">{participant.name}</h2>
        <div className="delegate-run-badges">
          <span className={`tone-${status.tone}`}>{status.label}</span>
          <span>Read-only run</span>
        </div>
        <nav aria-label="Agent detail view">
          <button
            type="button"
            aria-current={delegateSection === "overview" ? "page" : undefined}
            onClick={() => setDelegateSection("overview")}
          >
            Overview
          </button>
          <button
            type="button"
            aria-current={delegateSection === "activity" ? "page" : undefined}
            onClick={() => setDelegateSection("activity")}
          >
            Activity
          </button>
        </nav>
      </header>

      <section className="delegate-run-summary">
        <p>{inspection?.task || participant.task || participant.role}</p>
        <dl>
          <div>
            <dt>Role</dt>
            <dd>
              {agentRoleLabel(
                inspection?.role ?? participant.modelRole ?? participant.role,
              )}
            </dd>
          </div>
          <div>
            <dt>Duration</dt>
            <dd>
              {view === undefined
                ? participantDurationLabel(participant, inspection)
                : durationLabel(view.run)}
            </dd>
          </div>
          <div>
            <dt>Parent</dt>
            <dd>
              Primary
              {participant.parentRunIndex === undefined
                ? ""
                : ` · Run ${participant.parentRunIndex}`}
            </dd>
          </div>
          <div>
            <dt>Workspace</dt>
            <dd>
              <IconFolder size={14} stroke={1.7} aria-hidden="true" />
              {spaceName}
            </dd>
          </div>
        </dl>
      </section>

      {delegateSection === "activity" ? (
        <section className="delegate-activity-section">
          <h3>Released activity</h3>
          {loading ? (
            <p className="delegate-run-state">
              <IconLoader2 className="spin" size={15} aria-hidden="true" />
              Loading released activity…
            </p>
          ) : error !== "" ? (
            <p className="delegate-run-state tone-danger" role="alert">
              <IconAlertCircle size={15} aria-hidden="true" />
              {error}
            </p>
          ) : activities.length === 0 ? (
            <p className="delegate-run-state">
              {participant.state === "working" ||
              participant.state === "waiting"
                ? "No released actions are available yet."
                : "Detailed child actions were not released to this thread."}
            </p>
          ) : (
            <ol className="delegate-activity-list">
              {activities.map((activity) => {
                const activityStatus = presentToolState(activity.state);
                const expandable =
                  activity.input !== "" || activity.preview !== "";
                return (
                  <li key={activity.callId}>
                    <details>
                      <summary
                        aria-disabled={!expandable}
                        onClick={(event) => {
                          if (!expandable) {
                            event.preventDefault();
                          }
                        }}
                      >
                        <span className="delegate-activity-icon">
                          {toolIcon(activity.toolName)}
                        </span>
                        <span>
                          <strong>{activity.title}</strong>
                          <small>{activity.toolName}</small>
                        </span>
                        {activity.durationLabel === "" ? null : (
                          <time>{activity.durationLabel}</time>
                        )}
                        <i
                          className={`delegate-activity-status tone-${activityStatus.tone}`}
                          aria-label={activityStatus.label}
                        />
                        {expandable ? (
                          <IconChevronRight
                            className="delegate-activity-chevron"
                            size={14}
                            stroke={1.6}
                            aria-hidden="true"
                          />
                        ) : null}
                      </summary>
                      {expandable ? (
                        <div className="delegate-activity-details">
                          {activity.input === "" ? null : (
                            <div>
                              <span>Input</span>
                              <pre>{activity.input}</pre>
                            </div>
                          )}
                          {activity.preview === "" ? null : (
                            <div>
                              <span>Output</span>
                              <pre>{activity.preview}</pre>
                            </div>
                          )}
                        </div>
                      ) : null}
                    </details>
                  </li>
                );
              })}
            </ol>
          )}
        </section>
      ) : null}

      <section className="delegate-result-section">
        <h3>Result</h3>
        <p className={resultExpanded ? "is-expanded" : undefined}>
          {result === ""
            ? completedWithoutResult
              ? "The delegated agent completed without a released final response."
              : "The delegated agent has not released a final response yet."
            : result}
        </p>
        {result === "" ? null : (
          <button
            type="button"
            aria-expanded={resultExpanded}
            onClick={() => setResultExpanded((current) => !current)}
          >
            {resultExpanded ? "Collapse final response" : "View final response"}
            <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
          </button>
        )}
      </section>

      <section className="delegate-related-resources">
        <h3>Related resources</h3>
        <button type="button" onClick={() => onOpenSessionView("topology")}>
          <IconRobot size={15} stroke={1.6} aria-hidden="true" />
          <span>Session runs</span>
          <b>{sessionRunCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("plans")}>
          <IconListDetails size={15} stroke={1.6} aria-hidden="true" />
          <span>Plans</span>
          <b>{sessionPlanCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("sources")}>
          <IconBook2 size={15} stroke={1.6} aria-hidden="true" />
          <span>Sources</span>
          <b>{sessionSourceCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("snapshots")}>
          <IconAdjustments size={15} stroke={1.6} aria-hidden="true" />
          <span>Context snapshots</span>
          <b>{sessionSnapshotCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("resources")}>
          <IconFileText size={15} stroke={1.6} aria-hidden="true" />
          <span>Artifacts</span>
          <b>{fileCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
      </section>
    </>
  );
}

export function ThreadDetailsPanel({
  run,
  spaceName,
  pinned,
  participants,
  files,
  selectedParticipantId,
  delegateView,
  delegateInspection,
  delegateLoading,
  delegateError,
  sessionRunCount,
  sessionPlanCount,
  sessionResourceCount,
  sessionSnapshotCount,
  sessionSourceCount,
  onSelectParticipant,
  onBackToThread,
  onOpenSessionView,
}: ThreadDetailsPanelProps) {
  const status = presentRunStatus(run.status);
  const displayedParticipants =
    participants.length > 0
      ? participants
      : [
          {
            id: run.runId,
            name: agentRoleLabel(run.role),
            role: "Agent",
            state: "idle" as const,
            icon: "lead" as const,
            kind: "primary" as const,
          },
        ];
  const selectedParticipant = displayedParticipants.find(
    (participant) =>
      participant.kind === "delegate" &&
      participant.id === selectedParticipantId,
  );

  if (selectedParticipant !== undefined) {
    return (
      <aside
        className="thread-details-panel delegate-run-panel"
        aria-labelledby="thread-details-title"
      >
        <DelegateRunDetails
          participant={selectedParticipant}
          view={delegateView}
          inspection={delegateInspection}
          loading={delegateLoading}
          error={delegateError}
          spaceName={spaceName}
          sessionRunCount={sessionRunCount}
          sessionPlanCount={sessionPlanCount}
          sessionSnapshotCount={sessionSnapshotCount}
          sessionSourceCount={sessionSourceCount}
          fileCount={files.length}
          onOpenSessionView={onOpenSessionView}
          onBack={onBackToThread}
        />
      </aside>
    );
  }

  return (
    <aside
      className="thread-details-panel"
      aria-labelledby="thread-details-title"
    >
      <header>
        <h2 id="thread-details-title">Thread details</h2>
      </header>

      <dl className="thread-details-list">
        <div>
          <dt>Type</dt>
          <dd>{runModeLabel(run.mode)}</dd>
        </div>
        <div>
          <dt>Status</dt>
          <dd className={`tone-${status.tone}`}>
            {status.tone === "success" ? (
              <IconCheck size={14} stroke={2} aria-hidden="true" />
            ) : (
              <IconAlertCircle size={14} stroke={1.9} aria-hidden="true" />
            )}
            {status.label}
          </dd>
        </div>
        <div>
          <dt>Agent</dt>
          <dd>{agentRoleLabel(run.role)}</dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd>{shortDateLabel(run.createdAt)}</dd>
        </div>
        <div>
          <dt>Last updated</dt>
          <dd>{shortDateLabel(run.updatedAt)}</dd>
        </div>
        <div>
          <dt>Duration</dt>
          <dd>{durationLabel(run)}</dd>
        </div>
        <div>
          <dt>Workspace</dt>
          <dd>
            <IconFolder size={15} stroke={1.7} aria-hidden="true" />
            {spaceName}
          </dd>
        </div>
        {pinned ? (
          <div>
            <dt>Pinned by</dt>
            <dd>
              <IconPin size={14} stroke={1.8} aria-hidden="true" />
              You
            </dd>
          </div>
        ) : null}
      </dl>

      <section className="thread-details-section">
        <h3>Participants</h3>
        <div className="thread-detail-participants">
          {displayedParticipants.map((participant, index) => (
            <button
              key={participant.id}
              type="button"
              disabled={participant.kind !== "delegate"}
              onClick={() => onSelectParticipant(participant)}
            >
              <span className="thread-detail-person-icon" aria-hidden="true">
                {index === displayedParticipants.length - 1 &&
                displayedParticipants.length > 1 ? (
                  <IconUser size={16} stroke={1.7} />
                ) : (
                  <IconRobot size={16} stroke={1.7} />
                )}
              </span>
              <span>
                <strong>{participant.name}</strong>
                <small>{participant.role}</small>
              </span>
              <i
                className={`participant-presence participant-presence-${participant.state}`}
                aria-label={participant.state}
              />
              {participant.kind === "delegate" ? (
                <IconChevronRight
                  className="thread-detail-participant-chevron"
                  size={14}
                  stroke={1.6}
                  aria-hidden="true"
                />
              ) : null}
            </button>
          ))}
        </div>
        {participants.length <= 1 ? (
          <p className="thread-details-hint">
            Delegated agents appear here when released by this thread.
          </p>
        ) : null}
      </section>

      {files.length > 0 ? (
        <section className="thread-details-section thread-detail-files">
          <h3>
            Files <span>{files.length}</span>
          </h3>
          <div>
            {files.slice(0, 6).map((file) => (
              <article key={file.id}>
                <IconFileText size={15} stroke={1.6} aria-hidden="true" />
                <span>
                  <strong>{file.fileName}</strong>
                  <small>{file.stateLabel}</small>
                </span>
              </article>
            ))}
          </div>
        </section>
      ) : null}

      <section className="thread-details-section delegate-related-resources">
        <h3>Session resources</h3>
        <button type="button" onClick={() => onOpenSessionView("topology")}>
          <IconRobot size={15} stroke={1.6} aria-hidden="true" />
          <span>Runs</span>
          <b>{sessionRunCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("plans")}>
          <IconListDetails size={15} stroke={1.6} aria-hidden="true" />
          <span>Plans</span>
          <b>{sessionPlanCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("sources")}>
          <IconBook2 size={15} stroke={1.6} aria-hidden="true" />
          <span>Sources</span>
          <b>{sessionSourceCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("snapshots")}>
          <IconAdjustments size={15} stroke={1.6} aria-hidden="true" />
          <span>Snapshots</span>
          <b>{sessionSnapshotCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onOpenSessionView("resources")}>
          <IconFolder size={15} stroke={1.6} aria-hidden="true" />
          <span>All resources</span>
          <b>{sessionResourceCount}</b>
          <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
        </button>
      </section>
    </aside>
  );
}
