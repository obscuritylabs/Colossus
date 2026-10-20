import { useEffect, useMemo, useRef, useState } from "react";
import { Button, HostPlatformIcon } from "@colossus/ui";
import {
  ConversationEntry,
  ConversationTimeline,
  ReleasedRunActivity,
} from "@colossus/ui/conversation";
import {
  IconArrowLeft,
  IconArrowUpRight,
  IconMessage,
  IconRefresh,
} from "@tabler/icons-react";
import {
  terminalStatuses,
  boundRunUpdates,
  taskStatus,
  statusLabel,
  type FleetNode,
  type Host,
  type Permission,
  type Task,
} from "@colossus/control-plane-client";
import { messageOf, type MobileClient } from "./client";
import { usePoll } from "./usePoll";
import { useTaskFeed } from "./useTaskFeed";
import { taskTitle } from "./TaskList";
import { TaskStatus } from "./TaskStatus";
import { MobileInteractions } from "./MobileInteractions";
import { SessionViews } from "./SessionViews";
import { taskReleasedOutput } from "./released-output";

export function TaskDetail({
  client,
  project,
  initial,
  nodes,
  hosts,
  permissions,
  onBack,
  onConversation,
  onLocked,
  onTaskUpdated,
}: {
  client: MobileClient;
  project: string;
  initial: Task;
  nodes: FleetNode[];
  hosts: Map<string, Host>;
  permissions: Permission[];
  onBack: () => void;
  onConversation: (id: string) => Promise<void>;
  onLocked: (busy: boolean) => void;
  onTaskUpdated: (task: Task) => void;
}) {
  const snapshot = usePoll<{ task: Task }>(client, {
    kind: "task",
    project,
    task: initial.task_id,
  });
  const task = snapshot.value?.task ?? initial;
  useEffect(() => {
    if (snapshot.value?.task) onTaskUpdated(snapshot.value.task);
  }, [snapshot.value?.task, onTaskUpdated]);
  async function refreshAfterResponse() {
    try {
      const current = await client.request<{ task: Task }>({
        kind: "task",
        project,
        task: initial.task_id,
      });
      onTaskUpdated(current.task);
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      snapshot.refresh();
    }
  }

  const feed = useTaskFeed(client, project, task.task_id, {
    onThreadChanged: snapshot.refresh,
  });
  const node = nodes.find((node) => node.node.node_id === task.node_id),
    host = hosts.get(node?.node.host_id ?? "");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [interactionLocked, setInteractionLocked] = useState(false);
  const cancellation = useRef(crypto.randomUUID());
  useEffect(() => {
    onLocked(busy || interactionLocked);
    return () => onLocked(false);
  }, [busy, interactionLocked, onLocked]);
  async function cancel() {
    if (busy || interactionLocked || !task.run_id || task.source_read_only)
      return;
    setBusy(true);
    setError("");
    try {
      await client.request({
        kind: "cancel",
        project,
        task: task.task_id,
        mutation_id: cancellation.current,
      });
      setNotice(
        "Cancellation requested. The runtime will confirm its outcome in task status.",
      );
      snapshot.refresh();
      feed.refresh();
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  async function openConversation() {
    if (!task.thread_id || busy || interactionLocked) return;
    setBusy(true);
    setError("");
    try {
      await onConversation(task.thread_id);
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  const taskUpdates = useMemo(
    () => boundRunUpdates([task], feed.updates).get(task.task_id) ?? [],
    [feed.updates, task],
  );
  const output = taskReleasedOutput(task, taskUpdates),
    finished = terminalStatuses.has(taskStatus(task));
  return (
    <main className="mobile-task-detail">
      <header className="mobile-surface-heading">
        <Button
          variant="tertiary"
          aria-label="Back to tasks"
          disabled={busy || interactionLocked}
          onClick={onBack}
        >
          <IconArrowLeft size={20} />
        </Button>
        <div>
          <p className="mobile-caption">Task details</p>
          <h1>{taskTitle(task)}</h1>
        </div>
        <Button
          variant="tertiary"
          aria-label="Refresh task"
          onClick={() => {
            snapshot.refresh();
            feed.refresh();
          }}
        >
          <IconRefresh size={20} />
        </Button>
      </header>
      <TaskStatus task={task} />
      {(error || snapshot.error || feed.error || task.dispatch_error) && (
        <p role="alert" className="mobile-error">
          {error ||
            snapshot.error ||
            feed.error ||
            task.dispatch_error?.message}
        </p>
      )}
      {notice && (
        <p role="status" className="mobile-notice">
          {notice}
        </p>
      )}
      <dl className="mobile-task-facts">
        <div>
          <dt>Workspace</dt>
          <dd>
            <HostPlatformIcon
              platform={host?.platform ?? "unknown"}
              size={18}
            />
            {[host?.label ?? node?.node.label, node?.node.workspace_label]
              .filter(Boolean)
              .join(" · ") || "Assigned workspace"}
          </dd>
        </div>
        <div>
          <dt>Mode</dt>
          <dd>
            {statusLabel(task.request.mode)} · {task.request.role}
          </dd>
        </div>
        {task.snapshot?.run.updated_at && (
          <div>
            <dt>Updated</dt>
            <dd>
              <time dateTime={task.snapshot.run.updated_at}>
                {new Date(task.snapshot.run.updated_at).toLocaleString()}
              </time>
            </dd>
          </div>
        )}
      </dl>
      <div className="mobile-task-actions">
        {task.thread_id && (
          <Button
            disabled={busy || interactionLocked}
            onClick={() => void openConversation()}
          >
            <IconMessage size={18} aria-hidden="true" />
            Open conversation
          </Button>
        )}
        {!finished &&
          !task.source_read_only &&
          permissions.includes("control") && (
            <Button
              variant="danger"
              disabled={busy || !task.run_id}
              onClick={() => void cancel()}
            >
              {busy ? "Requesting…" : "Cancel task"}
            </Button>
          )}
      </div>
      {!finished && !task.run_id && permissions.includes("control") && (
        <p className="mobile-caption">
          Cancellation becomes available after the runtime allocates a run.
        </p>
      )}
      <MobileInteractions
        client={client}
        project={project}
        task={task.task_id}
        interactions={task.snapshot?.pending_interactions ?? []}
        permissions={
          task.source_read_only || node?.node.revoked ? [] : permissions
        }
        onUpdate={refreshAfterResponse}
        onError={setError}
        onLocked={setInteractionLocked}
      />
      <section className="mobile-task-output">
        <h2>Run output</h2>
        {feed.limited && !finished ? (
          <p className="mobile-notice">
            This output is partial because older activity has left the recent
            window. Live updates continue.
          </p>
        ) : null}
        <ConversationTimeline>
          <ConversationEntry
            role="user"
            content={task.request.input.map((part) => part.text).join("\n\n")}
          />
          <ReleasedRunActivity
            status={taskStatus(task)}
            updates={taskUpdates}
            terminal={task.snapshot?.run.terminal}
            updatedAt={task.snapshot?.run.updated_at}
            label="Run activity"
          />
          {output ? (
            <ConversationEntry role="assistant" content={output} />
          ) : (
            <p className="mobile-muted" role="status">
              {finished
                ? "No text output was released for this task."
                : task.run_id
                  ? "The runtime is working. Released output will appear here."
                  : "Queued on the selected runtime. Work starts when it is ready."}
            </p>
          )}
        </ConversationTimeline>
      </section>
      {(feed.limited || task.output_limited || task.history_bounded) && (
        <p className="mobile-notice">
          This view is bounded. Open Control Plane for the retained task
          inventory and the runtime for complete local output.
          <Button
            variant="tertiary"
            onClick={() =>
              void client
                .openBrowser(project, task.task_id)
                .catch((failure) => setError(messageOf(failure)))
            }
          >
            Review task in browser
            <IconArrowUpRight size={16} aria-hidden="true" />
          </Button>
        </p>
      )}
      <details className="mobile-task-events">
        <summary>Event activity · {taskUpdates.length} events</summary>
        <ol>
          {taskUpdates.map((event) => (
            <li key={`${event.run_id}:${event.sequence}`}>
              <span>{event.sequence}</span>
              <strong>
                {statusLabel(Object.keys(event.update)[0] ?? "Update")}
              </strong>
              <time dateTime={event.created_at}>
                {new Date(event.created_at).toLocaleTimeString()}
              </time>
            </li>
          ))}
        </ol>
      </details>
      <SessionViews
        client={client}
        project={project}
        tasks={[task]}
        updates={taskUpdates}
        available={Boolean(
          node?.presence?.ready &&
          !node.node.revoked &&
          node.presence.capabilities.includes("agent_messages.read.v1"),
        )}
        onError={setError}
      />
      <p className="mobile-caption mobile-wrap">Task {task.task_id}</p>
    </main>
  );
}
