import { useEffect, useRef, useState } from "react";
import { Button, HostPlatformIcon } from "@colossus/ui";
import {
  ConversationEntry,
  ConversationTimeline,
} from "@colossus/ui/conversation";
import {
  IconArrowLeft,
  IconArrowUpRight,
  IconMessage,
  IconRefresh,
} from "@tabler/icons-react";
import {
  terminalStatuses,
  taskStatus,
  statusLabel,
  visibleOutput,
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
}) {
  const snapshot = usePoll<{ task: Task }>(client, {
    kind: "task",
    project,
    task: initial.task_id,
  });
  const task = snapshot.value?.task ?? initial;
  const feed = useTaskFeed(client, project, task.task_id);
  const node = nodes.find((node) => node.node.node_id === task.node_id),
    host = hosts.get(node?.node.host_id ?? "");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const cancellation = useRef(crypto.randomUUID());
  useEffect(() => {
    onLocked(busy);
    return () => onLocked(false);
  }, [busy, onLocked]);
  async function cancel() {
    if (busy || !task.run_id || task.source_read_only) return;
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
    if (!task.thread_id || busy) return;
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
  const output = visibleOutput(feed.updates),
    finished = terminalStatuses.has(taskStatus(task));
  return (
    <main className="mobile-task-detail">
      <header className="mobile-surface-heading">
        <Button
          variant="tertiary"
          aria-label="Back to tasks"
          disabled={busy}
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
          <Button disabled={busy} onClick={() => void openConversation()}>
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
      {task.snapshot?.pending_interactions.some(
        (interaction) => interaction.status === "pending",
      ) && (
        <section className="mobile-attention">
          <h2>Your input is needed</h2>
          {task.snapshot.pending_interactions
            .filter((interaction) => interaction.status === "pending")
            .map((interaction) => (
              <p key={interaction.interaction_id}>
                {interaction.content.approval?.reason ??
                  interaction.content.user_prompt?.question ??
                  "Review the runtime’s request."}
              </p>
            ))}
          <Button
            onClick={() =>
              void client
                .openBrowser(project, task.task_id)
                .catch((failure) => setError(messageOf(failure)))
            }
          >
            Review in Control Plane
            <IconArrowUpRight size={16} aria-hidden="true" />
          </Button>
        </section>
      )}
      <section className="mobile-task-output">
        <h2>Run output</h2>
        <ConversationTimeline>
          <ConversationEntry
            role="user"
            content={task.request.input.map((part) => part.text).join("\n\n")}
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
        <summary>Event activity · {feed.updates.length} events</summary>
        <ol>
          {feed.updates.map((event) => (
            <li key={event.sequence}>
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
      <p className="mobile-caption mobile-wrap">Task {task.task_id}</p>
    </main>
  );
}
