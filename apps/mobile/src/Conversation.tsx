import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Button,
  RadioGroup,
  HostPlatformIcon,
  ComposerModeSwitch,
} from "@colossus/ui";
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
  ReleasedRunActivity,
} from "@colossus/ui/conversation";
import {
  IconArrowLeft,
  IconRefresh,
  IconAdjustmentsHorizontal,
  IconLayoutList,
  IconPlayerStop,
} from "@tabler/icons-react";
import type {
  FleetNode,
  Host,
  Permission,
  Task,
  Thread,
  ConversationMessage,
} from "@colossus/control-plane-client";
import {
  conversationProjection,
  taskStatus,
  terminalStatuses,
  statusLabel,
} from "@colossus/control-plane-client";
import {
  ClientFailure,
  messageOf,
  runRequest,
  type MobileClient,
  type Operation,
} from "./client";
import { useThreadHistory } from "./useThreadHistory";
import { useTaskFeed } from "./useTaskFeed";
import { MobileInteractions } from "./MobileInteractions";
import { SessionViews } from "./SessionViews";
import { useRuntimeOptions } from "./useRuntimeOptions";
import { ModelPicker } from "@colossus/ui/model-picker";
import {
  GoalControls,
  DEFAULT_GOAL_ITERATIONS,
  validGoalIterations,
} from "@colossus/ui/components/GoalControls";
import {
  ResearchControls,
  RESEARCH_SOURCE_OPTIONS,
} from "@colossus/ui/components/ResearchControls";
import type {
  ResearchDepth,
  ResearchSourceKind,
} from "@colossus/ui/session/types";
import "@colossus/ui/styles/research-controls.css";
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
  const detail = useThreadHistory(client, project, thread?.thread_id);
  const feed = useTaskFeed(client, project, "", {
    ...(thread ? { thread: thread.thread_id } : {}),
    onThreadChanged: detail.refresh,
  });
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
    [mode, setMode] = useState<"execute" | "plan" | "goal" | "research">(
      "execute",
    ),
    [role, setRole] = useState(target?.node.roles[0] ?? "");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [goalIterations, setGoalIterations] = useState(DEFAULT_GOAL_ITERATIONS);
  const [researchDepth, setResearchDepth] = useState<ResearchDepth>("standard");
  const [researchSources, setResearchSources] = useState<ResearchSourceKind[]>([
    "repo",
  ]);
  const runtime = useRuntimeOptions(
    client,
    project,
    target,
    permissions.includes("execute"),
  );
  const [modelChoice, setModelChoice] = useState({
    scope: "",
    model: "",
    reasoning: "",
  });
  const model = modelChoice.scope === runtime.scope ? modelChoice.model : "";
  const reasoning =
    modelChoice.scope === runtime.scope ? modelChoice.reasoning : "";
  const selectedModel = runtime.models.find(
    (profile) => profile.profile === model,
  );
  const defaultModel = runtime.models.find((profile) =>
    profile.default_roles.includes(role),
  );
  const researchAvailable = runtime.capabilities.includes("research.create");
  const goalAvailable = runtime.capabilities.includes("goal.create");
  const tools = target?.node.policy?.allowed_tools ?? [];
  const availableSources = RESEARCH_SOURCE_OPTIONS.filter(
    (source) =>
      tools.includes("*") ||
      tools.includes(
        { repo: "filesystem.search", web: "web.search", mcp: "mcp.call" }[
          source.value
        ],
      ),
  ).map((source) => source.value);
  const sources = RESEARCH_SOURCE_OPTIONS.map((source) => source.value).filter(
    (source) =>
      researchSources.includes(source) && availableSources.includes(source),
  );
  const modeBlocked =
    mode === "goal"
      ? !goalAvailable || !validGoalIterations(goalIterations)
      : mode === "research"
        ? !researchAvailable || !sources.length
        : false;
  const modelBlocked =
    mode !== "research" &&
    (Boolean((model || reasoning) && !runtime.models.length) ||
      Boolean(model && !selectedModel));
  const [interactionLocked, setInteractionLocked] = useState(false);
  const interactionLocks = useRef(new Set<string>());
  const interactionLock = useCallback((locked: boolean, task: string) => {
    if (locked) interactionLocks.current.add(task);
    else interactionLocks.current.delete(task);
    setInteractionLocked(interactionLocks.current.size > 0);
  }, []);
  const [sessionOpen, setSessionOpen] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const [optionsContainer, setOptionsContainer] =
    useState<HTMLDivElement | null>(null);
  const optionsTrigger = useRef<HTMLButtonElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);
  const bodyHeight = useRef(0);
  const attempt = useRef<{ signature: string; operation: Operation } | null>(
    null,
  );
  useEffect(() => {
    onLocked(busy || uncertain || interactionLocked);
    return () => onLocked(false);
  }, [busy, uncertain, interactionLocked, onLocked]);
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
  const projection = useMemo(
    () =>
      detail.value ? conversationProjection(detail.value, feed.updates) : null,
    [detail.value, feed.updates],
  );
  useLayoutEffect(() => {
    if (followLatest.current && scrollRef.current)
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [detail.value?.messages, feed.updates]);
  useLayoutEffect(() => {
    if (
      (sessionOpen || error || detail.error || feed.error || notice) &&
      scrollRef.current
    ) {
      followLatest.current = false;
      scrollRef.current.scrollTop = 0;
    }
  }, [sessionOpen, error, detail.error, feed.error, notice]);
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
  const continuationAvailable =
    permissions.includes("execute") &&
    !current?.archived &&
    (current
      ? current.can_continue && Boolean(current.session_id)
      : Boolean(target?.presence?.ready)) &&
    Boolean(target && !target.node.revoked && target.node.roles.includes(role));
  const canSend =
    continuationAvailable &&
    !modeBlocked &&
    !modelBlocked &&
    !interactionLocked;
  useLayoutEffect(() => {
    // Explain a durable loss of continuation once, without resetting a reader's
    // position on every live event or while an interaction is being submitted.
    if (!continuationAvailable && scrollRef.current) {
      followLatest.current = false;
      scrollRef.current.scrollTop = 0;
    }
  }, [continuationAvailable, project, current?.thread_id]);
  async function send() {
    if (busy || (!uncertain && (!canSend || !draft.trim()))) return;
    const signature = JSON.stringify([
      project,
      current?.thread_id,
      nodeId,
      role,
      mode,
      draft,
      model,
      reasoning,
      goalIterations,
      researchDepth,
      sources,
      runtime.scope,
    ]);
    // Reconciliation uses the entire original operation, even if polling has
    // changed the thread revision, role availability, or continuation state.
    if (!uncertain && attempt.current?.signature !== signature) {
      const request = {
        ...runRequest(draft, role, mode, crypto.randomUUID()),
        ...(mode !== "research" && model ? { model_profile: model } : {}),
        ...(mode !== "research" && reasoning
          ? { reasoning_effort: reasoning }
          : {}),
        ...(mode === "goal" ? { goal_max_iterations: goalIterations } : {}),
        research_depth: mode === "research" ? researchDepth : null,
        research_sources: mode === "research" ? sources : [],
      };
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
        uncertain ||
        (failure instanceof ClientFailure &&
          failure.code === "outcome_unknown");
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
    if (
      busy ||
      uncertain ||
      interactionLocked ||
      !task.run_id ||
      task.source_read_only
    )
      return;
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
          disabled={busy || uncertain || interactionLocked}
          onClick={onBack}
        >
          <IconArrowLeft size={20} />
        </Button>
        <div className="mobile-conversation-heading-copy">
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
            {current && (
              <span className="mobile-caption mobile-stream-status">
                {feed.connected
                  ? "Live activity"
                  : "Reconnecting · saved history available"}
              </span>
            )}
          </div>
        </div>
        <div className="mobile-conversation-actions">
          {current && (
            <Button
              variant="tertiary"
              onClick={() => setSessionOpen((open) => !open)}
              aria-label={sessionOpen ? "Hide session views" : "Session views"}
              title={sessionOpen ? "Hide session views" : "Session views"}
              aria-expanded={sessionOpen}
            >
              <IconLayoutList size={20} aria-hidden="true" />
            </Button>
          )}
          {active &&
          !terminalStatuses.has(taskStatus(active)) &&
          !active.source_read_only &&
          permissions.includes("control") ? (
            <Button
              variant="danger"
              aria-label="Cancel task"
              title="Cancel task"
              disabled={
                busy || uncertain || interactionLocked || !active.run_id
              }
              onClick={() => void cancel(active)}
            >
              <IconPlayerStop size={20} aria-hidden="true" />
            </Button>
          ) : null}
          <Button
            variant="tertiary"
            aria-label="Refresh conversation"
            onClick={detail.refresh}
          >
            <IconRefresh size={20} />
          </Button>
        </div>
      </header>
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
        {error || detail.error || feed.error ? (
          <p role="alert" className="mobile-error">
            {error || detail.error || feed.error}
          </p>
        ) : null}
        {notice && (
          <p role="status" className="mobile-notice">
            {notice}
          </p>
        )}
        {!canSend && (
          <p className="mobile-caption mobile-compose-notice">
            {current
              ? "Continuation is unavailable for this conversation or your project permissions."
              : "Connect an authorized runtime before starting work."}
          </p>
        )}
        {sessionOpen && current ? (
          <SessionViews
            client={client}
            project={project}
            tasks={detail.value?.tasks ?? []}
            updates={feed.updates}
            available={Boolean(
              target?.presence?.ready &&
              !target.node.revoked &&
              runtime.capabilities.includes("agent_messages.read.v1"),
            )}
            onError={setError}
          />
        ) : null}
        <h1 className="mobile-conversation-reading-title">
          {current?.title ?? "New task"}
        </h1>
        {current && !detail.value && !detail.error && (
          <p role="status" className="mobile-muted">
            Loading conversation…
          </p>
        )}
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
            {detail.value?.next_message_cursor ||
            detail.value?.next_task_cursor ? (
              <Button
                variant="tertiary"
                disabled={detail.loadingMore}
                onClick={async () => {
                  const panel = scrollRef.current,
                    height = panel?.scrollHeight ?? 0;
                  followLatest.current = false;
                  await detail.loadMore();
                  requestAnimationFrame(() => {
                    if (panel) panel.scrollTop += panel.scrollHeight - height;
                  });
                }}
              >
                {detail.loadingMore
                  ? "Loading earlier history…"
                  : "Load earlier history"}
              </Button>
            ) : null}
            {projection?.turns.map((turn, index) => {
              const split = turn.messages.findIndex(
                (message) => message.role !== "user",
              );
              const boundary = split < 0 ? turn.messages.length : split;
              const entry = (message: ConversationMessage) => (
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
                  author={
                    message.role === "user"
                      ? detail.value?.message_authors?.[message.message_id]
                          ?.display_name || "User"
                      : message.role === "assistant"
                        ? "Colossus"
                        : statusLabel(message.role)
                  }
                  createdAt={message.created_at}
                />
              );
              return (
                <section
                  className="shared-conversation-turn"
                  key={turn.key}
                  aria-label={`Conversation turn ${index + 1}`}
                  data-task-id={turn.task?.task_id}
                >
                  {turn.messages.slice(0, boundary).map(entry)}
                  {turn.task ? (
                    <>
                      {!terminalStatuses.has(taskStatus(turn.task)) ? (
                        <p className="mobile-caption" role="status">
                          {["waiting", "waiting_for_input"].includes(
                            taskStatus(turn.task),
                          )
                            ? "The agent needs your response"
                            : taskStatus(turn.task) === "queued"
                              ? "Queued on the runtime"
                              : "The agent is working"}
                        </p>
                      ) : null}
                      <ReleasedRunActivity
                        status={taskStatus(turn.task)}
                        updates={turn.updates}
                        terminal={turn.task.snapshot?.run.terminal}
                        updatedAt={turn.task.snapshot?.run.updated_at}
                        label={`Run activity for turn ${index + 1}`}
                      />
                      <MobileInteractions
                        client={client}
                        project={project}
                        task={turn.task.task_id}
                        interactions={
                          turn.task.snapshot?.pending_interactions ?? []
                        }
                        permissions={
                          turn.task.source_read_only || target?.node.revoked
                            ? []
                            : permissions
                        }
                        onUpdate={detail.refresh}
                        onError={setError}
                        onLocked={interactionLock}
                      />
                      {turn.task.output_limited || turn.task.history_bounded ? (
                        <p className="mobile-caption">
                          This run’s released output is bounded. Complete local
                          context remains on its runtime.
                        </p>
                      ) : null}
                    </>
                  ) : null}
                  {turn.messages.slice(boundary).map(entry)}
                </section>
              );
            })}
          </ConversationTimeline>
        )}
        {current?.sync_status === "incomplete" ? (
          <p className="mobile-notice">
            Some history has not synchronized yet.
          </p>
        ) : null}
        {feed.limited ? (
          <p className="mobile-caption">
            Older activity is omitted from this bounded recent window. New live
            activity continues and saved conversation messages remain available.
          </p>
        ) : null}
      </div>
      <ConversationComposer
        value={draft}
        onChange={setDraft}
        disabled={!canSend || uncertain || interactionLocked}
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
            disabled={busy || interactionLocked}
            aria-label={`Task options · ${statusLabel(mode)}`}
            onClick={() => setOptionsOpen(true)}
          >
            <IconAdjustmentsHorizontal size={18} aria-hidden="true" />
            {statusLabel(mode)}
          </Button>
        }
        stopping={busy}
      />
      <Sheet open={optionsOpen} onOpenChange={setOptionsOpen}>
        <SheetContent
          ref={setOptionsContainer}
          onEscapeKeyDown={(event) => {
            if (
              event.target instanceof Element &&
              event.target.closest(".ui-model-picker")
            )
              event.preventDefault();
          }}
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
              <ComposerModeSwitch
                aria-label="Run mode"
                value={mode}
                disabled={busy || uncertain}
                onChange={(value) => setMode(value as typeof mode)}
                options={[
                  {
                    value: "plan",
                    label: "Plan",
                    title: "Ask for a plan.",
                  },
                  {
                    value: "execute",
                    label: "Execute",
                    title: "Run the requested work.",
                  },
                  {
                    value: "goal",
                    label: "Goal",
                    title: goalAvailable
                      ? "Work toward an objective within an iteration budget."
                      : "Requires Goal support on the connected runtime.",
                    disabled: !goalAvailable,
                  },
                  {
                    value: "research",
                    label: "Research",
                    title: researchAvailable
                      ? "Gather evidence and citations."
                      : "Requires Research support on the connected runtime.",
                    disabled: !researchAvailable,
                  },
                ]}
              />
              {mode === "goal" ? (
                <GoalControls
                  value={goalIterations}
                  onChange={setGoalIterations}
                  disabled={busy || uncertain || !goalAvailable}
                />
              ) : null}
              {mode === "research" ? (
                <ResearchControls
                  researchDepth={researchDepth}
                  researchSources={sources}
                  availableSources={availableSources}
                  onResearchDepthChange={setResearchDepth}
                  onResearchSourcesChange={setResearchSources}
                  submitting={busy || uncertain || !researchAvailable}
                />
              ) : null}
              {modeBlocked ? (
                <p className="mobile-caption" role="status">
                  {mode === "goal"
                    ? "Goal requires an available runtime and an iteration limit from 1 to 50."
                    : "Research requires an available runtime and at least one authorized evidence source."}
                </p>
              ) : null}
            </section>
            <section>
              <h2>Model and reasoning</h2>
              {mode === "research" ? (
                <p className="mobile-caption">
                  Research uses the model configured for the host’s research
                  worker.
                </p>
              ) : runtime.models.length ? (
                <ModelPicker
                  portalContainer={optionsContainer}
                  model={model}
                  models={[
                    {
                      value: "",
                      label: "Role default",
                      detail: defaultModel
                        ? `${defaultModel.model} · ${defaultModel.provider}`
                        : "Runtime role routing",
                    },
                    ...runtime.models.map((profile) => ({
                      value: profile.profile,
                      label: profile.model,
                      detail: `${profile.provider} · ${profile.profile}`,
                    })),
                  ]}
                  label={
                    selectedModel?.model ??
                    defaultModel?.model ??
                    "Role default"
                  }
                  detail={
                    selectedModel?.provider ??
                    defaultModel?.provider ??
                    "Runtime role routing"
                  }
                  reasoning={reasoning}
                  reasoningLabel={
                    reasoning ||
                    `Model default${(selectedModel ?? defaultModel)?.reasoning_effort ? ` · ${(selectedModel ?? defaultModel)?.reasoning_effort}` : ""}`
                  }
                  reasoningOptions={[
                    { value: "", label: "Model default" },
                    ...[
                      "none",
                      "minimal",
                      "low",
                      "medium",
                      "high",
                      "xhigh",
                      "max",
                      "ultra",
                    ].map((effort) => ({
                      value: effort,
                      label: statusLabel(effort),
                    })),
                  ]}
                  onModelChange={(value) =>
                    setModelChoice({
                      scope: runtime.scope,
                      model: value,
                      reasoning: "",
                    })
                  }
                  onReasoningChange={(value) =>
                    setModelChoice({
                      scope: runtime.scope,
                      model,
                      reasoning: value,
                    })
                  }
                  disabled={busy || uncertain || interactionLocked}
                />
              ) : (
                <p className="mobile-caption">
                  {runtime.loading
                    ? "Loading authorized models…"
                    : "Model selection is unavailable on this runtime. Its configured role default will be used."}
                </p>
              )}
              {runtime.error ? (
                <p className="mobile-caption">{runtime.error}</p>
              ) : null}
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
