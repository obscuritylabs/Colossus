import { useMemo, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  MarkdownContent,
  ReleasedRunActivity,
} from "@colossus/ui/conversation";
import {
  SessionPlansView,
  SessionResourcesView,
  SessionSnapshotsView,
  SessionSourcesView,
} from "@colossus/ui/session/SessionWorkspace";
import type {
  AgentParticipant,
  ArtifactViewItem,
  PlanStatus,
  RunTerminal,
  RunView,
} from "@colossus/ui/session/types";
import type { SessionPlanReference } from "@colossus/ui/session/selectors";
import {
  AgentInboxInspector,
  type InboxPage,
  type InboxParticipant,
} from "@colossus/ui/agent-inbox";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@colossus/ui/components/ui/sheet";
import {
  boundRunUpdates,
  taskStatus,
  statusLabel,
  visibleOutput,
  type Task,
  type Update,
} from "@colossus/control-plane-client";
import { type MobileClient } from "./client";
import { taskTitle } from "./TaskList";
import "@colossus/ui/styles/session.css";

const LOCAL_RESOURCES =
  "This connection does not release the runtime’s local context or file map. Released plans, activity, sources, and artifact metadata are available here.";
function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
function runTerminal(value: unknown): RunTerminal | null {
  const raw = record(value);
  const result =
    record(raw?.result) ?? (typeof raw?.output === "string" ? raw : null);
  const cancellation = record(raw?.cancellation);
  const metadata = (data: Record<string, unknown>) => ({
    ...(typeof data.plan_id === "string" ? { planId: data.plan_id } : {}),
    ...(Number.isSafeInteger(data.plan_revision) &&
    Number(data.plan_revision) > 0
      ? { planRevision: Number(data.plan_revision) }
      : {}),
    ...(["draft", "approved", "executed", "discarded"].includes(
      String(data.plan_status),
    )
      ? { planStatus: data.plan_status as PlanStatus }
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
function artifactsFrom(updates: Update[]): ArtifactViewItem[] {
  const artifacts = new Map<string, ArtifactViewItem>();
  for (const update of updates) {
    const content = record(update.update.message)?.content;
    if (!Array.isArray(content)) continue;
    for (const part of content) {
      const artifact = record(record(part)?.artifact);
      if (
        !artifact ||
        typeof artifact.artifact_id !== "string" ||
        typeof artifact.file_name !== "string"
      )
        continue;
      const size = artifact.size_bytes ?? artifact.byte_length;
      artifacts.set(artifact.artifact_id, {
        id: artifact.artifact_id,
        fileName: artifact.file_name,
        mediaType:
          typeof artifact.media_type === "string"
            ? artifact.media_type
            : "File",
        sizeLabel:
          Number.isSafeInteger(size) && Number(size) >= 0
            ? `${Number(size).toLocaleString()} bytes`
            : "Size not reported",
        stateLabel:
          typeof artifact.state === "string"
            ? artifact.state
            : "Released metadata",
        createdLabel: "Not reported",
      });
    }
  }
  return [...artifacts.values()];
}
function participantsFrom(
  tasks: Task[],
  updates: Update[],
): AgentParticipant[] {
  const participants = new Map<string, AgentParticipant>();
  for (const task of tasks) {
    if (!task.run_id) continue;
    const status = taskStatus(task);
    participants.set(task.run_id, {
      id: task.run_id,
      name: task.snapshot?.run.title || task.task_id,
      role: task.request.role,
      kind: "primary",
      state:
        status === "completed"
          ? "completed"
          : ["failed", "outcome_unknown"].includes(status)
            ? "failed"
            : ["waiting", "waiting_for_input", "cancelling"].includes(status)
              ? "waiting"
              : ["cancelled", "interrupted"].includes(status)
                ? "cancelled"
                : "working",
    });
  }
  for (const update of updates) {
    if (
      !tasks.some(
        (task) =>
          task.run_id === update.run_id &&
          (!update.task_id || update.task_id === task.task_id),
      )
    )
      continue;
    const activity = record(update.update.tool_activity);
    if (
      activity?.tool_name !== "agent.subagent_update" ||
      typeof activity.preview !== "string" ||
      activity.preview.length > 65536
    )
      continue;
    try {
      const released = record(JSON.parse(activity.preview));
      const job = record(released?.job);
      if (
        released?.kind !== "subagent.lifecycle.v1" ||
        !job ||
        job.parent_run_id !== update.run_id ||
        typeof job.id !== "string" ||
        !job.id ||
        job.id.length > 128 ||
        typeof job.status !== "string"
      )
        continue;
      if (
        ![
          "queued",
          "running",
          "completed",
          "failed",
          "cancelled",
          "interrupted",
        ].includes(job.status)
      )
        continue;
      participants.set(job.id, {
        id: job.id,
        name:
          typeof job.task === "string"
            ? job.task.slice(0, 512)
            : "Delegated task",
        role:
          typeof job.role === "string" ? job.role.slice(0, 128) : "Delegate",
        kind: "delegate",
        state:
          job.status === "completed"
            ? "completed"
            : job.status === "failed"
              ? "failed"
              : ["cancelled", "interrupted"].includes(job.status)
                ? "cancelled"
                : job.status === "queued"
                  ? "waiting"
                  : "working",
      });
    } catch {
      /* Only structured released lifecycle metadata defines participants. */
    }
  }
  return [...participants.values()].slice(-32);
}

type Inspection = {
  kind: "inboxes";
  participants: InboxParticipant[];
  page: InboxPage | null;
};
function Inboxes({
  client,
  project,
  task,
  available,
}: {
  client: MobileClient;
  project: string;
  task: Task;
  available: boolean;
}) {
  async function inspect(participant?: string, after = 0): Promise<Inspection> {
    let { command } = await client.request<{
      command: {
        command_id: string;
        reply:
          Inspection | { kind: "failed"; error: { message: string } } | null;
      };
    }>({
      kind: "inspect_inboxes",
      project,
      task: task.task_id,
      request_id: crypto.randomUUID(),
      ...(participant ? { participant_id: participant } : {}),
      after_sequence: after,
    });
    const deadline = Date.now() + 5000;
    while (!command.reply && Date.now() < deadline) {
      await new Promise<void>((resolve) => setTimeout(resolve, 300));
      ({ command } = await client.request<{ command: typeof command }>({
        kind: "command",
        project,
        task: task.task_id,
        command: command.command_id,
      }));
    }
    if (!command.reply)
      throw new Error(
        "The runtime has not answered the inbox inspection. Refresh to retry.",
      );
    if (command.reply.kind === "failed")
      throw new Error(command.reply.error.message);
    return command.reply;
  }
  if (!task.run_id) return null;
  return (
    <section>
      <h3>{task.snapshot?.run.title || "Run inboxes"}</h3>
      <AgentInboxInspector
        rootRunId={task.run_id}
        available={available && !task.source_read_only}
        loadParticipants={async () => (await inspect()).participants}
        loadMessages={async (participant, after) => {
          const reply = await inspect(participant, after);
          if (!reply.page)
            throw new Error("The runtime returned no inbox page.");
          return reply.page;
        }}
      />
    </section>
  );
}

/** Shared session presentation consumes only released metadata; transport stays in this host. */
export function SessionViews({
  client,
  project,
  tasks,
  updates,
  available,
}: {
  client: MobileClient;
  project: string;
  tasks: Task[];
  updates: Update[];
  available: boolean;
  onError?: (message: string) => void;
}) {
  const [view, setView] = useState("activity");
  const [inboxTaskId, setInboxTaskId] = useState("");
  const inboxTasks = tasks.filter((task) => task.run_id);
  const inboxTask =
    inboxTasks.find((task) => task.task_id === inboxTaskId) ?? inboxTasks[0];
  const [plan, setPlan] = useState<SessionPlanReference | null>(null);
  const [artifact, setArtifact] = useState<ArtifactViewItem | null>(null);
  const bound = useMemo(
    () => boundRunUpdates(tasks, updates),
    [tasks, updates],
  );
  const released = useMemo(() => [...bound.values()].flat(), [bound]);
  const views = useMemo<RunView[]>(
    () =>
      tasks
        .filter((task) => task.snapshot)
        .map((task) => {
          const run = task.snapshot!.run,
            terminal = runTerminal(run.terminal);
          return {
            run: {
              runId: run.run_id,
              sessionId: run.session_id ?? "",
              title: run.title,
              status: taskStatus(task),
              mode: run.mode,
              createdAt: run.created_at,
              updatedAt: run.updated_at,
              terminal,
            },
            output:
              terminal?.type === "result"
                ? terminal.result.output
                : visibleOutput(bound.get(task.task_id) ?? []),
          };
        }),
    [tasks, bound],
  );
  const artifacts = useMemo(() => artifactsFrom(released), [released]);
  const participants = useMemo(
    () => participantsFrom(tasks, released),
    [tasks, released],
  );
  const common = {
    sessionMap: null,
    loading: false,
    error: LOCAL_RESOURCES,
    views,
    artifacts,
    onSelectResource: () => {},
    onSelectArtifact: (id: string) =>
      setArtifact(artifacts.find((item) => item.id === id) ?? null),
  };
  return (
    <section className="mobile-session-views" aria-label="Session details">
      <label htmlFor="mobile-session-view">Session view</label>
      <DropdownSelect
        id="mobile-session-view"
        aria-label="Session view"
        value={view}
        onChange={(event) => setView(event.target.value)}
        density="touch"
      >
        {[
          { value: "activity", label: "Activity" },
          { value: "topology", label: "Participants" },
          { value: "plans", label: "Plans" },
          { value: "inboxes", label: "Agent inboxes" },
          { value: "sources", label: "Sources" },
          { value: "resources", label: "Resources" },
          { value: "snapshots", label: "Snapshots" },
        ].map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </DropdownSelect>
      <div className="mobile-session-content">
        {view === "activity" ? (
          tasks.length ? (
            tasks.map((task) => (
              <ReleasedRunActivity
                key={task.task_id}
                status={taskStatus(task)}
                terminal={task.snapshot?.run.terminal}
                updatedAt={task.snapshot?.run.updated_at}
                updates={bound.get(task.task_id) ?? []}
                label={task.snapshot?.run.title || "Run activity"}
              />
            ))
          ) : (
            <p>No runs have been saved in this conversation.</p>
          )
        ) : view === "topology" ? (
          <section aria-label="Session participants">
            <h3>Participants</h3>
            <p className="mobile-caption">
              Primary runs and delegates from the loaded released activity.
            </p>
            {participants.length ? (
              <div className="mobile-rows" role="list">
                {participants.map((participant) => (
                  <div
                    className="mobile-row"
                    role="listitem"
                    key={participant.id}
                  >
                    <span>
                      <strong>{participant.name}</strong>
                      <small>
                        {participant.kind === "primary"
                          ? "Primary"
                          : "Delegate"}{" "}
                        · {participant.role} · {statusLabel(participant.state)}
                      </small>
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <p>No participants are present in the loaded activity.</p>
            )}
          </section>
        ) : view === "plans" ? (
          <SessionPlansView
            views={views}
            continuationAvailable={false}
            workflowAvailable={false}
            onInspectPlan={setPlan}
            onRevisePlan={() => {}}
            onOpenPlanWorkflow={() => {}}
          />
        ) : view === "sources" ? (
          <SessionSourcesView views={views} />
        ) : view === "snapshots" ? (
          <SessionSnapshotsView {...common} />
        ) : view === "resources" ? (
          <SessionResourcesView
            {...common}
            artifactCoverage="Released artifact metadata from the loaded history. Local file content remains on the runtime."
            onChangeView={setView}
          />
        ) : inboxTask ? (
          <section aria-label="Session agent inboxes">
            <label htmlFor="mobile-inbox-run">Run</label>
            <DropdownSelect
              id="mobile-inbox-run"
              aria-label="Inbox run"
              density="touch"
              value={inboxTask.task_id}
              onChange={(event) => setInboxTaskId(event.target.value)}
            >
              {inboxTasks.map((task) => (
                <option key={task.task_id} value={task.task_id}>
                  {taskTitle(task)}
                </option>
              ))}
            </DropdownSelect>
            <Inboxes
              key={inboxTask.task_id}
              client={client}
              project={project}
              task={inboxTask}
              available={available}
            />
          </section>
        ) : (
          <p>No runtime inboxes are present in the loaded runs.</p>
        )}
      </div>
      <Sheet
        open={Boolean(plan || artifact)}
        onOpenChange={(open) => {
          if (!open) {
            setPlan(null);
            setArtifact(null);
          }
        }}
      >
        <SheetContent
          side="bottom"
          className="mobile-task-options mobile-session-sheet"
        >
          <SheetHeader>
            <SheetTitle>
              {plan?.sourceRunTitle ?? artifact?.fileName ?? "Session item"}
            </SheetTitle>
            <SheetDescription>
              {plan
                ? "Released plan and revision"
                : "Released artifact metadata"}
            </SheetDescription>
          </SheetHeader>
          {plan ? (
            <>
              <MarkdownContent content={plan.output} />
              <p className="mobile-caption">
                Plan execution and revision require a Control Plane continuation
                contract; use Desktop for these actions.
              </p>
            </>
          ) : artifact ? (
            <dl className="mobile-task-facts">
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
          ) : null}
          <Button
            onClick={() => {
              setPlan(null);
              setArtifact(null);
            }}
            variant="secondary"
          >
            Close details
          </Button>
        </SheetContent>
      </Sheet>
    </section>
  );
}
