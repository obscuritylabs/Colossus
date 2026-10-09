import { useMemo, useState } from "react";
import {
  SessionPlansView,
  SessionResourcesView,
  SessionSnapshotsView,
  SessionSourcesView,
  SessionTopology,
} from "@colossus/ui/session/SessionWorkspace";
import { SessionPresentationProvider } from "@colossus/ui/session/links";
import type {
  RunView,
  RunTerminal,
  PlanStatus,
} from "@colossus/ui/session/types";
import type { SessionPlanReference } from "@colossus/ui/session/selectors";
import { WorkflowDialog } from "@colossus/ui/automations/WorkflowDialog";
import {
  MarkdownContent,
  type SessionWorkspaceView,
} from "@colossus/ui/conversation";
import { ThreadRunActivity } from "./ThreadRunActivity";
import {
  taskTitle,
  taskStatus,
  visibleOutput,
  type Task,
  type Update,
} from "./api";
import { WebLink } from "./WebLink";
import { Button } from "@colossus/ui";
import { releasedArtifacts } from "./released-artifacts";
import type { ArtifactViewItem } from "@colossus/ui/session/types";
import "@colossus/ui/styles/session.css";
import "@colossus/ui/styles/workflows.css";

const LOCAL_RESOURCES =
  "This runtime connection does not release its local session map. Conversations, plans and released activity remain available; inspect local context and files in Desktop.";
function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
function terminal(value: unknown): RunTerminal | null {
  const raw = record(value);
  const result = record(raw?.result);
  const cancellation = record(raw?.cancellation);
  const metadata = (record: Record<string, unknown>) => ({
    ...(typeof record.plan_id === "string" ? { planId: record.plan_id } : {}),
    ...(Number.isSafeInteger(record.plan_revision) &&
    Number(record.plan_revision) > 0
      ? { planRevision: Number(record.plan_revision) }
      : {}),
    ...(["draft", "approved", "executed", "discarded"].includes(
      String(record.plan_status),
    )
      ? { planStatus: record.plan_status as PlanStatus }
      : {}),
  });
  if (result)
    return {
      type: "result",
      result: {
        output: typeof result.output === "string" ? result.output : "",
        ...metadata(result),
      },
    };
  if (cancellation)
    return { type: "cancellation", cancellation: metadata(cancellation) };
  return null;
}
export function sessionRunViews(tasks: Task[], updates: Update[]): RunView[] {
  return tasks
    .filter((task) => task.snapshot)
    .map((task) => {
      const run = task.snapshot!.run;
      const finished = terminal(run.terminal);
      return {
        run: {
          runId: run.run_id,
          sessionId: run.session_id ?? "",
          title: run.title || taskTitle(task),
          status: taskStatus(task),
          mode: run.mode,
          createdAt: run.created_at,
          updatedAt: run.updated_at,
          terminal: finished,
        },
        output:
          finished?.type === "result"
            ? finished.result.output
            : visibleOutput(
                updates.filter(
                  (update) =>
                    update.run_id === task.run_id &&
                    (!update.task_id || update.task_id === task.task_id),
                ),
              ),
      };
    })
    .sort(
      (a, b) =>
        a.run.createdAt.localeCompare(b.run.createdAt) ||
        a.run.runId.localeCompare(b.run.runId),
    );
}
export function ThreadSessionViews({
  view,
  tasks,
  updates,
  onChangeView,
  onRevisePlan,
  canContinuePlan,
}: {
  view: SessionWorkspaceView;
  tasks: Task[];
  updates: Update[];
  onChangeView: (view: SessionWorkspaceView) => void;
  onRevisePlan: (plan: SessionPlanReference) => void;
  canContinuePlan: boolean;
}) {
  const views = useMemo(
    () => sessionRunViews(tasks, updates),
    [tasks, updates],
  );
  const [plan, setPlan] = useState<SessionPlanReference | null>(null);
  const [artifact, setArtifact] = useState<ArtifactViewItem | null>(null);
  const artifacts = useMemo(
    () =>
      releasedArtifacts(updates).map((item) => ({
        id: item.key,
        fileName: item.fileName,
        mediaType: item.typeLabel,
        sizeLabel: item.sizeLabel,
        stateLabel: item.stateLabel,
        createdLabel: "Not reported",
      })),
    [updates],
  );
  const common = {
    sessionMap: null,
    loading: false,
    error: LOCAL_RESOURCES,
    onSelectResource: () => {},
    onSelectArtifact: (id: string) =>
      setArtifact(artifacts.find((item) => item.id === id) ?? null),
    artifacts,
    views,
  };
  return (
    <SessionPresentationProvider linkComponent={WebLink}>
      <div className="thread-session-view">
        {view === "plans" ? (
          <SessionPlansView
            views={views}
            continuationAvailable={canContinuePlan}
            workflowAvailable={false}
            onInspectPlan={setPlan}
            onRevisePlan={(sourceRunId) => {
              const match =
                plan ?? views.find((view) => view.run.runId === sourceRunId);
              if (match && "planId" in match)
                onRevisePlan(match as SessionPlanReference);
            }}
            onOpenPlanWorkflow={() => {}}
          />
        ) : view === "sources" ? (
          <SessionSourcesView views={views} />
        ) : view === "snapshots" ? (
          <SessionSnapshotsView {...common} />
        ) : view === "resources" ? (
          <SessionResourcesView
            {...common}
            artifactCoverage="Artifact metadata from loaded retained messages. Local files and earlier history may be available in Desktop."
            onChangeView={onChangeView}
          />
        ) : view === "topology" ? (
          <SessionTopology
            {...common}
            participants={
              views.length
                ? [
                    {
                      id: views[0]!.run.runId,
                      name: "Colossus",
                      role: "primary",
                      kind: "primary",
                      state: tasks.some((task) =>
                        ["queued", "running"].includes(taskStatus(task)),
                      )
                        ? "working"
                        : "completed",
                    },
                  ]
                : []
            }
          />
        ) : (
          <section className="session-activity" aria-label="Session activity">
            <header className="session-view-summary">
              <div>
                <h3>Activity</h3>
                <p>Released activity across this session.</p>
              </div>
            </header>
            {tasks.map((task) => (
              <ThreadRunActivity
                key={task.task_id}
                task={task}
                updates={updates.filter(
                  (update) =>
                    update.run_id === task.run_id &&
                    (!update.task_id || update.task_id === task.task_id),
                )}
                label={taskTitle(task)}
              />
            ))}
          </section>
        )}
        {artifact ? (
          <WorkflowDialog
            title={artifact.fileName}
            busy={false}
            onClose={() => setArtifact(null)}
          >
            <dl className="configuration-list">
              <div>
                <dt>Media type</dt>
                <dd>{artifact.mediaType}</dd>
              </div>
              <div>
                <dt>Size</dt>
                <dd>{artifact.sizeLabel}</dd>
              </div>
              <div>
                <dt>State</dt>
                <dd>{artifact.stateLabel}</dd>
              </div>
            </dl>
            <p>
              Released metadata is available here. Open local file contents in
              Desktop.
            </p>
            <Button onClick={() => setArtifact(null)}>Close artifact</Button>
          </WorkflowDialog>
        ) : null}
        {plan ? (
          <WorkflowDialog
            title={plan.sourceRunTitle}
            busy={false}
            onClose={() => setPlan(null)}
          >
            <MarkdownContent content={plan.output} linkComponent={WebLink} />
            <Button onClick={() => setPlan(null)}>Close plan</Button>
          </WorkflowDialog>
        ) : null}
      </div>
    </SessionPresentationProvider>
  );
}
