import { useMemo } from "react";
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
import {
  selectSessionPlans,
  type SessionPlanReference,
} from "@colossus/ui/session/selectors";
import { type SessionWorkspaceView } from "@colossus/ui/conversation";
import { ThreadRunActivity } from "./ThreadRunActivity";
import {
  taskTitle,
  taskStatus,
  visibleOutput,
  type Task,
  type Update,
} from "./api";
import { WebLink } from "./WebLink";
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  EmptyDescription,
} from "@colossus/ui/components/ui/empty";
import { IconActivity } from "@tabler/icons-react";
import { releasedArtifacts } from "./released-artifacts";
import type { ArtifactViewItem } from "@colossus/ui/session/types";
import type { PlanStrategy } from "./plan-continuation";
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
  onInspectPlan,
  onInspectArtifact,
}: {
  view: SessionWorkspaceView;
  tasks: Task[];
  updates: Update[];
  onChangeView: (view: SessionWorkspaceView) => void;
  onRevisePlan: (plan: SessionPlanReference) => void;
  canContinuePlan: boolean;
  onExecutePlan?: (plan: SessionPlanReference, strategy: PlanStrategy) => void;
  goalAvailable?: boolean;
  busy?: boolean;
  onInspectPlan?: (plan: SessionPlanReference) => void;
  onInspectArtifact?: (artifact: ArtifactViewItem) => void;
}) {
  const views = useMemo(
    () => sessionRunViews(tasks, updates),
    [tasks, updates],
  );
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
    onSelectArtifact: (id: string) => {
      const artifact = artifacts.find((item) => item.id === id);
      if (artifact) onInspectArtifact?.(artifact);
    },
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
            onInspectPlan={(plan) => onInspectPlan?.(plan)}
            onRevisePlan={(sourceRunId, planId, revision) => {
              const match = selectSessionPlans(views).find(
                (candidate) =>
                  candidate.sourceRunId === sourceRunId &&
                  candidate.planId === planId &&
                  candidate.revision === revision,
              );
              if (match) onRevisePlan(match);
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
                <p>Released activity from loaded runs in this session.</p>
              </div>
            </header>
            {tasks.length === 0 ? (
              <Empty className="session-view-empty">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <IconActivity size={20} aria-hidden="true" />
                  </EmptyMedia>
                  <EmptyTitle>No released activity</EmptyTitle>
                  <EmptyDescription>
                    Run activity will appear here when released.
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : null}
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
      </div>
    </SessionPresentationProvider>
  );
}
