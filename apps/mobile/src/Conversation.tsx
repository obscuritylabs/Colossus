import { useEffect, useRef, useState } from "react";
import {
  Button,
  ComposerModeSwitch,
  DropdownSelect,
  HostPlatformIcon,
} from "@colossus/ui";
import {
  ConversationComposer,
  ConversationEntry,
  ConversationTimeline,
} from "@colossus/ui/conversation";
import {
  IconArrowLeft,
  IconArrowUpRight,
  IconRefresh,
} from "@tabler/icons-react";
import type {
  FleetNode,
  Host,
  Permission,
  Task,
  Thread,
  ThreadDetailResponse,
} from "@colossus/control-plane-client";
import {
  ClientFailure,
  messageOf,
  runRequest,
  type MobileClient,
  type Operation,
} from "./client";
import { usePoll } from "./usePoll";

export function Conversation({
  client,
  project,
  thread,
  nodes,
  permissions,
  onBack,
  onCreated,
  onLocked,
  hosts,
}: {
  client: MobileClient;
  project: string;
  thread: Thread | null;
  nodes: FleetNode[];
  permissions: Permission[];
  onBack: () => void;
  onCreated: (thread: Thread) => void;
  onLocked: (locked: boolean) => void;
  hosts: Host[];
}) {
  const detail = usePoll<ThreadDetailResponse>(
    client,
    thread
      ? { kind: "thread", project, thread: thread.thread_id }
      : { kind: "me" },
  );
  const current = thread ? (detail.value?.thread ?? thread) : null;
  const [nodeId, setNodeId] = useState(
    thread?.node_id ??
      nodes.find((n) => n.presence?.ready && !n.node.revoked)?.node.node_id ??
      "",
  );
  const target = nodes.find(
    (n) => n.node.node_id === (current?.node_id ?? nodeId),
  );
  const host = hosts.find((host) => host.host_id === target?.node.host_id);
  const [draft, setDraft] = useState(""),
    [mode, setMode] = useState<"execute" | "plan">("execute"),
    [role, setRole] = useState(target?.node.roles[0] ?? "");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [uncertain, setUncertain] = useState(false);
  const attempt = useRef<{ signature: string; operation: Operation } | null>(
    null,
  );
  useEffect(() => {
    onLocked(busy || uncertain);
    return () => onLocked(false);
  }, [busy, uncertain, onLocked]);
  const cancelKey = useRef(new Map<string, string>());
  const active = thread
    ? detail.value?.tasks.find((t) => t.task_id === current?.active_task_id)
    : undefined;
  useEffect(() => {
    if (target && !target.node.roles.includes(role))
      setRole(target.node.roles[0] ?? "");
  }, [target, role]);
  const canSend =
    permissions.includes("execute") &&
    !current?.archived &&
    (current
      ? current.can_continue && Boolean(current.session_id)
      : Boolean(target?.presence?.ready)) &&
    Boolean(target && !target.node.revoked && target.node.roles.includes(role));
  async function send() {
    if (busy || (!uncertain && (!canSend || !draft.trim()))) return;
    const signature = JSON.stringify([
      project,
      current?.thread_id,
      nodeId,
      role,
      mode,
      draft,
    ]);
    // Reconciliation uses the entire original operation, even if polling has
    // changed the thread revision, role availability, or continuation state.
    if (!uncertain && attempt.current?.signature !== signature) {
      const request = runRequest(draft, role, mode, crypto.randomUUID());
      attempt.current = {
        signature,
        operation: current
          ? {
              kind: "send_message",
              project,
              thread: current.thread_id,
              revision: current.revision,
              request,
            }
          : { kind: "create_thread", project, node: nodeId, request },
      };
    }
    if (!attempt.current) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await client.request<{ thread: Thread; task: Task }>(
        attempt.current.operation,
      );
      setDraft("");
      attempt.current = null;
      setUncertain(false);
      if (!current) onCreated(result.thread);
      else detail.refresh();
      setNotice("Task queued. Runtime progress will appear here.");
    } catch (failure) {
      setError(messageOf(failure));
      const unknown =
        failure instanceof ClientFailure && failure.code === "outcome_unknown";
      setUncertain(unknown);
      if (!unknown) {
        attempt.current = null;
        detail.refresh();
      }
    } finally {
      setBusy(false);
    }
  }
  async function cancel(task: Task) {
    if (busy) return;
    setBusy(true);
    setError("");
    const key = cancelKey.current.get(task.task_id) ?? crypto.randomUUID();
    cancelKey.current.set(task.task_id, key);
    try {
      await client.request({
        kind: "cancel",
        project,
        task: task.task_id,
        mutation_id: key,
      });
      setNotice(
        "Cancellation requested. The runtime will confirm its outcome in task status.",
      );
      detail.refresh();
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="mobile-conversation">
      <header className="mobile-surface-heading">
        <Button
          variant="tertiary"
          aria-label="Back to work"
          disabled={busy || uncertain}
          onClick={onBack}
        >
          <IconArrowLeft size={20} />
        </Button>
        <div>
          <h1>{current?.title ?? "New task"}</h1>
          <p className="mobile-caption">
            {[host?.label ?? target?.node.label, target?.node.workspace_label]
              .filter(Boolean)
              .join(" · ") || "Choose a workspace"}
            {target?.presence?.ready ? " · Connected" : " · Offline"}
          </p>
        </div>
        <Button
          variant="tertiary"
          aria-label="Refresh conversation"
          onClick={detail.refresh}
        >
          <IconRefresh size={20} />
        </Button>
      </header>
      {error || detail.error ? (
        <p role="alert" className="mobile-error">
          {error || detail.error}
        </p>
      ) : null}
      {notice && (
        <p role="status" className="mobile-notice">
          {notice}
        </p>
      )}
      {current && !detail.value && !detail.error && (
        <p role="status" className="mobile-muted">
          Loading conversation…
        </p>
      )}
      <div className="mobile-conversation-body">
        {!current && (
          <div className="mobile-empty">
            <h2>What would you like to work on?</h2>
            <p>Select a workspace. Colossus will run the task on that host.</p>
          </div>
        )}
        {current && (
          <ConversationTimeline>
            {detail.value?.messages.map((message) => (
              <ConversationEntry
                key={message.message_id}
                role={
                  message.role === "user"
                    ? "user"
                    : message.role === "assistant"
                      ? "assistant"
                      : "system"
                }
                content={message.text}
                createdAt={message.created_at}
              />
            ))}
          </ConversationTimeline>
        )}
        {current?.sync_status === "incomplete" && (
          <p className="mobile-notice">
            Some history has not synchronized yet.
          </p>
        )}
        {(detail.value?.next_message_cursor ||
          detail.value?.next_task_cursor) && (
          <p className="mobile-notice">
            This is the latest history.{" "}
            <Button
              variant="tertiary"
              onClick={() =>
                void client
                  .openBrowser(project)
                  .catch((failure) => setError(messageOf(failure)))
              }
            >
              Open Control Plane for earlier history{" "}
              <IconArrowUpRight size={16} />
            </Button>
          </p>
        )}
        {active && (
          <p role="status" className="mobile-status">
            <HostPlatformIcon platform={host?.platform ?? "unknown"} />
            {active.snapshot?.run.status.replaceAll("_", " ") ??
              "Queued on assigned host"}
          </p>
        )}
        {active && permissions.includes("control") && (
          <Button
            variant="danger"
            disabled={busy}
            onClick={() => void cancel(active)}
          >
            Cancel task
          </Button>
        )}
        {active?.snapshot?.pending_interactions.some(
          (i) => i.status === "pending",
        ) && (
          <div className="mobile-attention">
            <h2>Your input is needed</h2>
            <p>Review the runtime’s request in Control Plane.</p>
            <Button
              onClick={() =>
                void client
                  .openBrowser(project, active.task_id)
                  .catch((failure) => setError(messageOf(failure)))
              }
            >
              Review in browser <IconArrowUpRight size={16} />
            </Button>
          </div>
        )}
      </div>
      {!canSend && (
        <p className="mobile-caption mobile-compose-notice">
          {current
            ? "Continuation is unavailable for this conversation or your project permissions."
            : "Connect an authorized runtime before starting work."}
        </p>
      )}
      <ConversationComposer
        value={draft}
        onChange={setDraft}
        disabled={!canSend || uncertain}
        busy={busy}
        label="Message Colossus"
        placeholder="Continue this thread…"
        action={uncertain ? "Reconcile submission" : "Send"}
        shortcut="modEnter"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
        context={
          <>
            <span>
              {target?.node.policy?.models.length
                ? `Host models: ${target.node.policy.models.map((model) => model.label).join(" · ")}`
                : "Host-configured model"}
            </span>
            <span className="mobile-caption">
              {target?.node.policy
                ? `Approvals: ${target.node.policy.approval_mode.replaceAll("_", " ")}`
                : "Policy not reported"}
            </span>
          </>
        }
        controls={
          <>
            <DropdownSelect
              aria-label="Execution workspace"
              value={current?.node_id ?? nodeId}
              disabled={Boolean(current) || busy || uncertain}
              onChange={(event) => {
                setNodeId(event.target.value);
                setRole(
                  nodes.find((n) => n.node.node_id === event.target.value)?.node
                    .roles[0] ?? "",
                );
              }}
            >
              {nodes
                .filter((n) => !n.node.revoked)
                .map((n) => (
                  <option value={n.node.node_id} key={n.node.node_id}>
                    {[
                      hosts.find((h) => h.host_id === n.node.host_id)?.label ??
                        n.node.label,
                      n.node.workspace_label,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                    {n.presence?.ready ? "" : " · Offline"}
                  </option>
                ))}
            </DropdownSelect>
            <DropdownSelect
              aria-label="Agent role"
              value={role}
              disabled={busy || uncertain}
              onChange={(event) => setRole(event.target.value)}
            >
              {target?.node.roles.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </DropdownSelect>
            <ComposerModeSwitch
              value={mode}
              onChange={(value) => setMode(value as "execute" | "plan")}
              options={[
                { value: "plan", label: "Plan" },
                { value: "execute", label: "Execute" },
              ]}
              disabled={busy || uncertain}
            />
          </>
        }
        stopping={busy}
      />
      {uncertain && (
        <Button variant="tertiary" disabled={busy} onClick={onBack}>
          Leave and inspect recent work
        </Button>
      )}
      {uncertain && (
        <Button
          className="mobile-reconcile"
          disabled={busy}
          onClick={() => void send()}
        >
          Reconcile the original submission
        </Button>
      )}
    </section>
  );
}
