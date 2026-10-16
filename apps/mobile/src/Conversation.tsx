import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, RadioGroup, HostPlatformIcon } from "@colossus/ui";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetClose,
} from "@colossus/ui/components/ui/sheet";
import {
  ConversationComposer,
  ConversationEntry,
  ConversationTimeline,
} from "@colossus/ui/conversation";
import {
  IconArrowLeft,
  IconArrowUpRight,
  IconRefresh,
  IconAdjustmentsHorizontal,
} from "@tabler/icons-react";
import type {
  FleetNode,
  Host,
  Permission,
  Task,
  Thread,
  ThreadDetailResponse,
} from "@colossus/control-plane-client";
import { taskStatus, terminalStatuses } from "@colossus/control-plane-client";
import {
  ClientFailure,
  messageOf,
  runRequest,
  type MobileClient,
  type Operation,
} from "./client";
import { usePoll } from "./usePoll";
import { TaskStatus } from "./TaskStatus";

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
  const [optionsOpen, setOptionsOpen] = useState(false);
  const optionsTrigger = useRef<HTMLButtonElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);
  const bodyHeight = useRef(0);
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
  const latestTask =
    active ??
    (current
      ? [...(detail.value?.tasks ?? [])].sort((a, b) =>
          (b.created_at ?? "").localeCompare(a.created_at ?? ""),
        )[0]
      : undefined);
  useLayoutEffect(() => {
    if (followLatest.current && scrollRef.current)
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [detail.value?.messages]);
  useEffect(() => {
    const body = scrollRef.current;
    if (!body) return;
    const observer = new ResizeObserver(() => {
      bodyHeight.current = body.clientHeight;
      if (followLatest.current) body.scrollTop = body.scrollHeight;
    });
    observer.observe(body);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (notice && latestTask && terminalStatuses.has(taskStatus(latestTask)))
      setNotice("");
  }, [notice, latestTask]);
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
      followLatest.current = true;
      attempt.current = null;
      setUncertain(false);
      if (!current) onCreated(result.thread);
      else detail.refresh();
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
    if (busy || !task.run_id || task.source_read_only) return;
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
          <div className="mobile-conversation-context" role="status">
            <HostPlatformIcon
              platform={host?.platform ?? "unknown"}
              size={16}
            />
            <span>
              {target?.node.workspace_label ??
                target?.node.label ??
                "Choose a workspace"}
            </span>
            {latestTask ? (
              <TaskStatus task={latestTask} />
            ) : (
              <span className="mobile-muted">
                {target?.presence?.ready ? "Connected" : "Offline"}
              </span>
            )}
          </div>
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
      <div
        className="mobile-conversation-body"
        ref={scrollRef}
        onScroll={() => {
          const body = scrollRef.current;
          if (body && body.clientHeight !== bodyHeight.current) {
            bodyHeight.current = body.clientHeight;
            if (followLatest.current) body.scrollTop = body.scrollHeight;
          } else if (body)
            followLatest.current =
              body.scrollHeight - body.scrollTop - body.clientHeight < 64;
        }}
      >
        {!current && (
          <div className="mobile-empty">
            <h2>Start something new</h2>
            <p>
              Your task will run in{" "}
              <strong>
                {target?.node.workspace_label ??
                  target?.node.label ??
                  "your selected workspace"}
              </strong>
              . Change the workspace or run mode in Task options.
            </p>
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
        {active &&
          !terminalStatuses.has(taskStatus(active)) &&
          !active.source_read_only &&
          permissions.includes("control") && (
            <Button
              variant="danger"
              disabled={busy || !active.run_id}
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
        placeholder={
          current ? "Message Colossus…" : "What would you like to do?"
        }
        textareaProps={{ rows: 1 }}
        action={uncertain ? "Reconcile submission" : "Send"}
        shortcut="modEnter"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
        controls={
          <Button
            ref={optionsTrigger}
            variant="tertiary"
            aria-haspopup="dialog"
            aria-label={`Task options · ${mode === "plan" ? "Plan" : "Execute"}`}
            onClick={() => setOptionsOpen(true)}
          >
            <IconAdjustmentsHorizontal size={18} aria-hidden="true" />
            {mode === "plan" ? "Plan" : "Execute"}
          </Button>
        }
        stopping={busy}
      />
      <Sheet open={optionsOpen} onOpenChange={setOptionsOpen}>
        <SheetContent
          side="bottom"
          className="mobile-task-options"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            optionsTrigger.current?.focus();
          }}
        >
          <SheetHeader>
            <SheetTitle>Task options</SheetTitle>
            <SheetDescription>
              Choose how your next message runs.
            </SheetDescription>
          </SheetHeader>
          <div className="mobile-options-body">
            <section>
              <h2>Workspace</h2>
              {current ? (
                <div className="mobile-selected-workspace">
                  <strong>
                    {target?.node.workspace_label ?? target?.node.label}
                  </strong>
                  <p>
                    {host?.label ?? target?.node.label} ·{" "}
                    {target?.presence?.ready ? "Connected" : "Offline"}
                  </p>
                  <p className="mobile-muted">
                    This conversation stays on its original workspace.
                  </p>
                </div>
              ) : (
                <RadioGroup
                  aria-label="Execution workspace"
                  value={nodeId}
                  disabled={busy || uncertain}
                  onValueChange={(value) => {
                    setNodeId(value);
                    setRole(
                      nodes.find((n) => n.node.node_id === value)?.node
                        .roles[0] ?? "",
                    );
                  }}
                  options={nodes
                    .filter((n) => !n.node.revoked)
                    .map((n) => ({
                      value: n.node.node_id,
                      label: n.node.workspace_label ?? n.node.label,
                      description: `${hosts.find((h) => h.host_id === n.node.host_id)?.label ?? n.node.label} · ${n.presence?.ready ? "Connected" : "Offline"}`,
                    }))}
                />
              )}
            </section>
            <section>
              <h2>Run mode</h2>
              <RadioGroup
                aria-label="Run mode"
                value={mode}
                disabled={busy || uncertain}
                onValueChange={(value) => setMode(value as "execute" | "plan")}
                options={[
                  {
                    value: "plan",
                    label: "Plan",
                    description: "Ask for a plan.",
                  },
                  {
                    value: "execute",
                    label: "Execute",
                    description: "Run the requested work.",
                  },
                ]}
              />
            </section>
            <section>
              <h2>Agent role</h2>
              <RadioGroup
                aria-label="Agent role"
                value={role}
                disabled={busy || uncertain}
                onValueChange={setRole}
                options={(target?.node.roles ?? []).map((r) => ({
                  value: r,
                  label: r === "primary" ? "Primary agent" : r,
                }))}
              />
            </section>
            <section className="mobile-policy-summary">
              <h2>Host configuration</h2>
              <p>
                {target?.node.policy?.models.length
                  ? `Models: ${target.node.policy.models.map((model) => model.label).join(" · ")}`
                  : "Model configured on the host"}
              </p>
              <p>
                {target?.node.policy
                  ? `Approvals: ${target.node.policy.approval_mode.replaceAll("_", " ")}`
                  : "Approval policy not reported"}
              </p>
            </section>
          </div>
          <div className="mobile-options-footer">
            <SheetClose asChild>
              <Button variant="primary">Done</Button>
            </SheetClose>
          </div>
        </SheetContent>
      </Sheet>
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
