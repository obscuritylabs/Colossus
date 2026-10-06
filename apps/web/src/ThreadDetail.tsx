import { WebLink } from "./WebLink";
import {
  ConversationEntry,
  ConversationTimeline,
} from "@colossus/ui/conversation";
import { Badge } from "@colossus/ui/components/ui/badge";
import { Button as LinkButton } from "@colossus/ui/components/ui/button";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import {
  IconArchive,
  IconArchiveOff,
  IconArrowDown,
  IconArrowLeft,
  IconCheck,
  IconEdit,
  IconLoader2,
  IconMessageCircle,
  IconPlayerStop,
  IconX,
} from "@tabler/icons-react";
import {
  awaitReceipt,
  ApiFailure,
  projectPath,
  request,
  statusLabel,
  taskStatus,
  terminalStatuses,
  type CommandReceipt,
  type FleetNode,
  type Permission,
  type Thread,
  type Task,
  type ThreadMessage,
  type ThreadDetailResponse,
  type Update,
} from "./api";
import { Interactions } from "./Interactions";
import { RunComposer, type RunRequest } from "./RunComposer";
import { conversationProjection } from "./conversation";
import { ThreadRunActivity } from "./ThreadRunActivity";

function mergeMessages(current: ThreadMessage[], incoming: ThreadMessage[]) {
  const byId = new Map(current.map((item) => [item.message_id, item]));
  for (const item of incoming) byId.set(item.message_id, item);
  return [...byId.values()].sort(
    (a, b) =>
      a.created_at.localeCompare(b.created_at) ||
      a.message_id.localeCompare(b.message_id),
  );
}
function mergeTasks(current: Task[], incoming: Task[]) {
  const byId = new Map(current.map((item) => [item.task_id, item]));
  for (const item of incoming) byId.set(item.task_id, item);
  const created = (task: Task) =>
    Date.parse(task.created_at || task.snapshot?.run.created_at || "") || 0;
  return [...byId.values()].sort(
    (a, b) => created(b) - created(a) || b.task_id.localeCompare(a.task_id),
  );
}

export function ThreadDetail({
  initial,
  project,
  nodes,
  permissions,
  onBack,
  onChanged,
  onPolicy,
  backLabel = "All threads",
  backHref,
}: {
  initial: Thread;
  project: string;
  nodes: FleetNode[];
  permissions: Permission[];
  onBack: () => void;
  onChanged: () => void;
  onPolicy?: (() => void) | undefined;
  backLabel?: string;
  backHref?: string;
}) {
  const [detail, setDetail] = useState<ThreadDetailResponse>({
      thread: initial,
      tasks: [],
      messages: [],
    }),
    [events, setEvents] = useState<Update[]>([]),
    [connected, setConnected] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false),
    [controlling, setControlling] = useState(false),
    [editing, setEditing] = useState(false),
    [title, setTitle] = useState(initial.title),
    [below, setBelow] = useState(false),
    [historyBusy, setHistoryBusy] = useState(false),
    [resolvedTarget, setResolvedTarget] = useState<FleetNode | null>(null);
  const path = `${projectPath(project)}/threads/${encodeURIComponent(initial.thread_id)}`;
  const nodePath = `${projectPath(project)}/nodes/${encodeURIComponent(initial.node_id)}`;
  const hasListedTarget = nodes.some(
    (item) => item.node.node_id === initial.node_id,
  );
  const alive = useRef(true),
    heading = useRef<HTMLHeadingElement>(null),
    conversation = useRef<HTMLDivElement>(null),
    follow = useRef(true),
    cancellations = useRef(new Map<string, string>()),
    historyLoaded = useRef(false);
  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      try {
        const [value, target] = await Promise.all([
          request<ThreadDetailResponse>(path, undefined, signal),
          hasListedTarget
            ? Promise.resolve(null)
            : request<FleetNode>(nodePath, undefined, signal),
        ]);
        if (target && !signal?.aborted && alive.current)
          setResolvedTarget(target);
        if (
          value.thread.active_task_id &&
          !value.tasks.some(
            (task) => task.task_id === value.thread.active_task_id,
          )
        ) {
          const active = await request<{ task: Task }>(
            `${projectPath(project)}/tasks/${encodeURIComponent(value.thread.active_task_id)}`,
            undefined,
            signal,
          );
          value.tasks.push(active.task);
        }
        if (!signal?.aborted && alive.current)
          setDetail((current) => ({
            ...value,
            messages: mergeMessages(current.messages, value.messages),
            tasks: mergeTasks(current.tasks, value.tasks),
            next_message_cursor:
              (historyLoaded.current
                ? current.next_message_cursor
                : value.next_message_cursor) ?? null,
            next_task_cursor:
              (historyLoaded.current
                ? current.next_task_cursor
                : value.next_task_cursor) ?? null,
          }));
      } catch (error) {
        if (!signal?.aborted && alive.current)
          setError(
            error instanceof Error
              ? error.message
              : "The conversation could not be refreshed.",
          );
      }
    },
    [path, project, nodePath, hasListedTarget],
  );
  useEffect(() => {
    heading.current?.focus();
  }, []);
  useEffect(() => {
    alive.current = true;
    const abort = new AbortController();
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        await refresh(abort.signal);
      } finally {
        pending = false;
      }
    };
    void poll();
    const timer = setInterval(() => {
      if (!document.hidden) void poll();
    }, 2000);
    return () => {
      alive.current = false;
      abort.abort();
      clearInterval(timer);
    };
  }, [refresh]);
  useEffect(() => {
    const source = new EventSource(`${path}/events?after=0`);
    source.onopen = () => setConnected(true);
    source.onerror = () => setConnected(false);
    source.addEventListener("run_update", (event) => {
      try {
        const update = JSON.parse(
          (event as MessageEvent<string>).data,
        ) as Update;
        setEvents((current) =>
          current.some(
            (item) =>
              item.run_id === update.run_id &&
              item.sequence === update.sequence &&
              item.task_id === update.task_id,
          )
            ? current
            : [...current, update],
        );
        if (
          update.update.message ||
          update.update.result ||
          update.update.state ||
          update.update.interaction
        )
          void refresh();
      } catch {
        setError("The conversation stream returned an invalid update.");
        source.close();
      }
    });
    source.addEventListener("thread_changed", () => void refresh());
    return () => source.close();
  }, [path, refresh]);
  useEffect(() => {
    if (!conversation.current) return;
    if (follow.current) {
      conversation.current.scrollTop = conversation.current.scrollHeight;
      setBelow(false);
    } else setBelow(true);
  }, [detail.messages.length, events.length]);
  const { thread, tasks, messages } = detail;
  const projection = useMemo(
    () => conversationProjection(detail, events),
    [detail, events],
  );
  const target =
    nodes.find((item) => item.node.node_id === thread.node_id) ??
    resolvedTarget;
  const composerNodes = target && !hasListedTarget ? [...nodes, target] : nodes;
  const active = tasks.filter(
    (task) => !terminalStatuses.has(taskStatus(task)),
  );
  const running = active.find(
    (task) =>
      !task.source_read_only && task.run_id && taskStatus(task) !== "queued",
  );
  async function olderHistory() {
    if (historyBusy) return;
    const params = new URLSearchParams();
    if (detail.next_message_cursor)
      params.set("message_after", detail.next_message_cursor);
    if (detail.next_task_cursor)
      params.set("task_after", detail.next_task_cursor);
    setHistoryBusy(true);
    setError("");
    const panel = conversation.current,
      oldHeight = panel?.scrollHeight ?? 0;
    try {
      const value = await request<ThreadDetailResponse>(`${path}?${params}`);
      if (!alive.current) return;
      follow.current = false;
      historyLoaded.current = true;
      setDetail((current) => ({
        ...current,
        messages: mergeMessages(current.messages, value.messages),
        tasks: mergeTasks(current.tasks, value.tasks),
        next_message_cursor: value.next_message_cursor ?? null,
        next_task_cursor: value.next_task_cursor ?? null,
      }));
      requestAnimationFrame(() => {
        if (panel) panel.scrollTop += panel.scrollHeight - oldHeight;
      });
    } catch (error) {
      if (alive.current)
        setError(
          error instanceof Error
            ? error.message
            : "Earlier history could not be loaded.",
        );
    } finally {
      if (alive.current) setHistoryBusy(false);
    }
  }
  async function send(runRequest: RunRequest) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const submit = (revision: number) =>
        request<{ thread: Thread }>(`${path}/messages`, {
          expected_revision: revision,
          request: runRequest,
        });
      let result: { thread: Thread };
      try {
        result = await submit(thread.revision);
      } catch (error) {
        if (!(error instanceof ApiFailure && error.status === 409)) throw error;
        const [latest, latestTarget] = await Promise.all([
          request<ThreadDetailResponse>(path),
          request<FleetNode>(nodePath),
        ]);
        if (
          latest.thread.archived ||
          !latest.thread.can_continue ||
          latest.thread.node_id !== thread.node_id ||
          latest.thread.session_id !== thread.session_id ||
          latestTarget.node.revoked ||
          !target ||
          latestTarget.node.instance_id !== target.node.instance_id ||
          latestTarget.node.roles.join("\u0000") !==
            target.node.roles.join("\u0000")
        )
          throw error;
        // Runtime progress can advance the revision without changing this human request.
        // Reconcile once with the same idempotency key and immutable placement.
        result = await submit(latest.thread.revision);
      }
      if (!alive.current) return false;
      setDetail((value) => ({ ...value, thread: result.thread }));
      follow.current = true;
      await refresh();
      onChanged();
      setNotice(
        target?.presence?.ready
          ? "Message accepted. The agent will process turns in order."
          : "Message accepted and queued for this agent to reconnect.",
      );
      return true;
    } catch (error) {
      if (alive.current) {
        setError(
          error instanceof Error
            ? error.message
            : "Message submission failed. Retry to reconcile the same request.",
        );
        void refresh();
      }
      return false;
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function metadata(values: { title?: string; archived?: boolean }) {
    setBusy(true);
    setError("");
    try {
      const result = await request<{ thread: Thread }>(
        path,
        { revision: thread.revision, ...values },
        undefined,
        "PATCH",
      );
      if (alive.current) {
        setDetail((value) => ({ ...value, thread: result.thread }));
        setEditing(false);
        onChanged();
      }
    } catch (error) {
      if (alive.current) {
        setError(
          error instanceof Error
            ? error.message
            : "Conversation changes could not be saved.",
        );
        void refresh();
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function cancel() {
    if (!running) return;
    setControlling(true);
    setError("");
    let mutation = cancellations.current.get(running.task_id);
    if (!mutation) {
      mutation = crypto.randomUUID();
      cancellations.current.set(running.task_id, mutation);
    }
    const taskPath = `${projectPath(project)}/tasks/${encodeURIComponent(running.task_id)}`;
    try {
      const { command } = await request<{ command: CommandReceipt }>(
        `${taskPath}/cancel`,
        { mutation_id: mutation },
      );
      const settled = await awaitReceipt(taskPath, command);
      if (alive.current) {
        setNotice(
          settled
            ? "Cancellation confirmed by the runtime."
            : "Cancellation queued. The runtime will reconcile this exact request after reconnecting.",
        );
        void refresh();
      }
    } catch (error) {
      if (alive.current)
        setError(
          error instanceof Error ? error.message : "Cancellation failed.",
        );
    } finally {
      if (alive.current) setControlling(false);
    }
  }
  const queuedTurns = Math.max(
    0,
    (thread.queued_task_ids?.length ?? active.length) - (running ? 1 : 0),
  );
  const canWrite =
    permissions.includes("execute") &&
    thread.can_continue &&
    Boolean(thread.session_id) &&
    !thread.archived &&
    Boolean(target && !target.node.revoked);
  return (
    <section className="detail thread-detail" aria-labelledby="thread-heading">
      {backHref ? (
        <LinkButton asChild variant="ghost" className="back">
          <a
            href={backHref}
            onClick={(event) => {
              if (
                event.defaultPrevented ||
                event.button !== 0 ||
                event.metaKey ||
                event.ctrlKey ||
                event.shiftKey ||
                event.altKey
              )
                return;
              event.preventDefault();
              onBack();
            }}
          >
            <IconArrowLeft size={16} aria-hidden="true" />
            {backLabel}
          </a>
        </LinkButton>
      ) : (
        <Button variant="tertiary" className="back" onClick={onBack}>
          <IconArrowLeft size={16} aria-hidden="true" />
          {backLabel}
        </Button>
      )}
      <div className="detail-header">
        <div>
          <div className="eyebrow">
            {target?.node.label ?? thread.node_id}
            {target?.node.workspace_label
              ? ` / ${target.node.workspace_label}`
              : ""}
          </div>
          {editing ? (
            <form
              className="thread-rename"
              onSubmit={(event) => {
                event.preventDefault();
                if (title.trim()) void metadata({ title: title.trim() });
              }}
            >
              <h2 id="thread-heading" className="sr-only">
                Rename {thread.title}
              </h2>
              <TextInput
                aria-label="Thread title"
                value={title}
                maxLength={256}
                onChange={(event) => setTitle(event.target.value)}
                autoFocus
              />
              <Button
                type="submit"
                aria-label="Save thread title"
                disabled={busy || !title.trim()}
              >
                <IconCheck size={16} aria-hidden="true" />
              </Button>
              <Button
                aria-label="Cancel rename"
                onClick={() => setEditing(false)}
              >
                <IconX size={16} aria-hidden="true" />
              </Button>
            </form>
          ) : (
            <h2 id="thread-heading" ref={heading} tabIndex={-1}>
              {thread.title || "Untitled thread"}
            </h2>
          )}
        </div>
        <div className="thread-header-actions">
          {permissions.includes("control") ? (
            <>
              <Button
                variant="tertiary"
                aria-label="Rename thread"
                onClick={() => {
                  setTitle(thread.title);
                  setEditing(true);
                }}
                disabled={busy}
              >
                <IconEdit size={16} aria-hidden="true" />
              </Button>
              <Button
                variant="secondary"
                onClick={() => void metadata({ archived: !thread.archived })}
                disabled={busy}
              >
                {thread.archived ? (
                  <IconArchiveOff size={16} aria-hidden="true" />
                ) : (
                  <IconArchive size={16} aria-hidden="true" />
                )}
                {thread.archived ? "Restore" : "Archive"}
              </Button>
            </>
          ) : null}
        </div>
      </div>
      <div className="detail-meta">
        <span>
          <span className={`dot ${target?.presence?.ready ? "live" : ""}`} />
          {target?.presence?.ready ? "Agent online" : "Agent offline"}
        </span>
        <span>
          {connected ? "Live updates connected" : "Reconnecting updates"}
        </span>
        <span>
          {thread.source === "runtime"
            ? "Shared local session"
            : "Control Plane conversation"}
        </span>
        {thread.archived ? <Badge className="status">Archived</Badge> : null}
      </div>
      {error ? (
        <div className="alert" role="alert">
          {error}
          <Button aria-label="Dismiss error" onClick={() => setError("")}>
            <IconX size={16} aria-hidden="true" />
          </Button>
        </div>
      ) : null}
      {notice ? (
        <p className="sync-notice" role="status">
          {notice}
        </p>
      ) : null}
      {thread.sync_status !== "current" ? (
        <p className="sync-notice" role="status">
          Released history is incomplete or bounded. Saved messages are
          available here; canonical conversation context remains with the agent.
          Additional released history appears when available.
        </p>
      ) : null}
      {!target?.presence?.ready ? (
        <p className="sync-notice">
          You’re viewing saved history. The conversation remains assigned to its
          original agent.
        </p>
      ) : null}
      {detail.next_message_cursor || detail.next_task_cursor ? (
        <div className="thread-history-toolbar">
          <Button disabled={historyBusy} onClick={() => void olderHistory()}>
            {historyBusy ? "Loading history…" : "Load earlier history"}
          </Button>
          <span>
            Saved messages and runs are paged; earlier history remains
            available.
          </span>
        </div>
      ) : null}
      <div
        className="thread-conversation"
        ref={conversation}
        tabIndex={0}
        aria-label="Conversation history"
        onScroll={() => {
          const panel = conversation.current;
          if (panel) {
            follow.current =
              panel.scrollHeight - panel.clientHeight - panel.scrollTop < 80;
            if (follow.current) setBelow(false);
          }
        }}
      >
        <ConversationTimeline>
          {!messages.length && !tasks.length ? (
            <div className="output-empty">
              <IconMessageCircle size={28} aria-hidden="true" />
              <h3>Loading conversation</h3>
              <p>Released messages and saved agent work will appear here.</p>
            </div>
          ) : null}
          {projection.turns.map((turn, index) => {
            const firstResponse = turn.messages.findIndex(
                (message) => message.role !== "user",
              ),
              split = firstResponse < 0 ? turn.messages.length : firstResponse,
              task = turn.task,
              status = task ? taskStatus(task) : "";
            const messageEntry = (message: (typeof turn.messages)[number]) => (
              <ConversationEntry
                key={message.message_id}
                role={
                  message.role === "user"
                    ? "user"
                    : message.role === "assistant"
                      ? "assistant"
                      : "system"
                }
                author={
                  message.role === "user"
                    ? "Human"
                    : message.role === "assistant"
                      ? "Colossus"
                      : statusLabel(message.role)
                }
                createdAt={message.created_at}
                content={message.text}
                linkComponent={WebLink}
                status={
                  message.status ? (
                    <Badge className={`status status-${message.status}`}>
                      {statusLabel(message.status)}
                    </Badge>
                  ) : null
                }
              />
            );
            return (
              <section
                className="shared-conversation-turn"
                key={turn.key}
                data-task-id={
                  task?.task_id ?? turn.messages[0]?.task_id ?? undefined
                }
                data-run-id={task?.run_id ?? undefined}
                aria-label={`Conversation turn ${index + 1}`}
              >
                {turn.messages.slice(0, split).map(messageEntry)}
                {task ? (
                  <div className="conversation-run" data-task-id={task.task_id}>
                    {!terminalStatuses.has(status) ||
                    task.dispatch_error ||
                    task.output_limited ? (
                      <div className="run-progress" role="status">
                        {!terminalStatuses.has(status) ? (
                          <IconLoader2
                            size={15}
                            className={status === "queued" ? undefined : "spin"}
                            aria-hidden="true"
                          />
                        ) : null}
                        <strong>{statusLabel(status)}</strong>
                        <span>
                          {task.dispatch_error?.message ??
                            (status === "queued"
                              ? "Waiting for this agent and earlier turns"
                              : status === "waiting"
                                ? "The agent needs a response"
                                : terminalStatuses.has(status)
                                  ? "Viewing saved run details"
                                  : "The agent is working")}
                        </span>
                      </div>
                    ) : null}
                    {task.output_limited ? (
                      <p className="sync-notice">
                        This run reached its released-output limit. Open the
                        local session for complete output.
                      </p>
                    ) : null}
                    {task.history_bounded ? (
                      <p className="sync-notice">
                        This run’s released history is bounded. Its canonical
                        conversation context remains with the agent.
                      </p>
                    ) : null}
                    <ThreadRunActivity
                      task={task}
                      updates={turn.updates}
                      label={`Run activity for turn ${index + 1}`}
                    />
                    <Interactions
                      interactions={task.snapshot?.pending_interactions ?? []}
                      project={project}
                      task={task.task_id}
                      permissions={
                        task.source_read_only || target?.node.revoked
                          ? []
                          : permissions
                      }
                      onUpdate={() => void refresh()}
                      onError={setError}
                    />
                  </div>
                ) : null}
                {turn.messages.slice(split).map(messageEntry)}
              </section>
            );
          })}
        </ConversationTimeline>
      </div>
      {below ? (
        <Button
          className="jump-latest"
          onClick={() => {
            if (conversation.current)
              conversation.current.scrollTop =
                conversation.current.scrollHeight;
            follow.current = true;
            setBelow(false);
          }}
        >
          <IconArrowDown size={16} aria-hidden="true" />
          Jump to latest
        </Button>
      ) : null}
      {running && permissions.includes("control") ? (
        <div className="thread-run-controls">
          <Button
            variant="secondary"
            disabled={controlling || !target || target.node.revoked}
            onClick={() => void cancel()}
          >
            <IconPlayerStop size={16} aria-hidden="true" />
            {controlling ? "Requesting cancellation…" : "Stop active run"}
          </Button>
          <span>
            {queuedTurns > 0
              ? `${queuedTurns} queued turn${queuedTurns === 1 ? "" : "s"}`
              : "Runs stay assigned to this agent"}
          </span>
        </div>
      ) : null}
      <RunComposer
        onPolicy={onPolicy}
        nodes={composerNodes}
        nodeId={thread.node_id}
        lockedRuntime
        busy={busy}
        disabled={!canWrite}
        label="Message the agent"
        action={
          active.length || thread.active_task_id
            ? "Queue message"
            : "Send message"
        }
        onSubmit={send}
      />
      {!canWrite ? (
        <p className="composer-offline">
          {thread.archived
            ? "Restore this thread to continue the conversation."
            : !thread.can_continue
              ? "This local session was shared for viewing. Continue it locally or enable continuation in its workspace sharing settings."
              : !thread.session_id
                ? "The initial turn is queued or starting. You can continue once this agent establishes its session."
                : target?.node.revoked
                  ? "This agent’s enrollment was revoked. Saved history remains available."
                  : "Your project access does not allow messages to this agent."}
        </p>
      ) : null}
    </section>
  );
}
