import { useSidebarDropdown } from "@colossus/ui/hooks/useSidebarDropdown";
import { InspectorLayout } from "@colossus/ui/components/InspectorLayout";
import {
  ThreadPlanInspection,
  ThreadArtifactInspection,
  type ThreadInspection,
  inspectionKey,
} from "./ThreadInspection";
import { ThreadAsidePanel } from "./ThreadAsidePanel";
import type { AsideDraftSource } from "./ThreadBranchDraft";
import { latestChatForkSource } from "./ChatHistorySidebar.actions";
import "@colossus/ui/styles/inspector.css";
import "./thread-inspector.css";
import "./conversation-text.css";
import { ThreadAgentInboxes } from "./ThreadAgentInboxes";
import { WebLink } from "./WebLink";
import { releasedRunUsage } from "./message-usage";
import { loadNextPrompt } from "./next-prompt";
import {
  WorkSurfaceHeader,
  SESSION_WORKSPACE_VIEWS,
  WORK_STATUS_PRESENTATIONS,
  type SessionWorkspaceView,
  ConversationEntry,
  ConversationTimeline,
  ConversationSelectionLauncher,
  boundedConversationQuote,
} from "@colossus/ui/conversation";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
import {
  IconChevronDown,
  IconChevronUp,
  IconCopy,
  IconDots,
  IconDownload,
  IconGitFork,
  IconMessagePlus,
  IconSearch,
} from "@tabler/icons-react";
import { Badge } from "@colossus/ui/components/ui/badge";
import { Button as LinkButton } from "@colossus/ui/components/ui/button";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useId,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
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
  IconLock,
  IconPlugConnected,
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
import { HostedFileLinks } from "./HostedAttachments";
import { conversationProjection } from "./conversation";
import {
  conversationMarkdown,
  downloadConversationMarkdown,
  findConversationMessages,
  type ConversationTextMessage,
} from "./conversation-text";
import { ThreadSessionViews, sessionRunViews } from "./ThreadSessionViews";
import { ThreadRunActivity } from "./ThreadRunActivity";
import {
  selectSessionPlans,
  canContinuePlanFromRun,
  type SessionPlanReference,
} from "@colossus/ui/session/selectors";
import { useRunModeCapabilities } from "./research-capability";
import { ThreadPlanDecision } from "./ThreadPlanDecision";
import type { PlanContinuation, PlanStrategy } from "./plan-continuation";

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
  personalChat,
  onConnections,
  initialComposerRequest,
  onFork,
  embedded = false,
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
  personalChat?: { owner: string; node: string } | undefined;
  onConnections?: (() => void) | undefined;
  initialComposerRequest?: RunRequest | undefined;
  onFork?: ((thread: Thread, source: Task) => void) | undefined;
  embedded?: boolean;
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
    [view, setView] = useState<SessionWorkspaceView>("conversation"),
    [below, setBelow] = useState(false),
    [historyBusy, setHistoryBusy] = useState(false),
    [resolvedTarget, setResolvedTarget] = useState<FleetNode | null>(null);
  const actionsMenu = useSidebarDropdown();
  const skipActionsFocus = useRef(false);
  const headingId = useId();
  const [inspectionState, setInspectionState] = useState<{
    tabs: ThreadInspection[];
    active: string | null;
  }>({ tabs: [], active: null });
  const inspection =
    inspectionState.tabs.find(
      (tab) => inspectionKey(tab) === inspectionState.active,
    ) ?? null;
  function setInspection(next: ThreadInspection | null) {
    setInspectionState((current) => {
      if (!next) return { ...current, active: null };
      const key = inspectionKey(next);
      const exists = current.tabs.some((tab) => inspectionKey(tab) === key);
      return {
        tabs: exists
          ? current.tabs.map((tab) => (inspectionKey(tab) === key ? next : tab))
          : [...current.tabs, next],
        active: key,
      };
    });
  }
  function closeInspection(key: string) {
    setInspectionState((current) => {
      const index = current.tabs.findIndex((tab) => inspectionKey(tab) === key);
      const tabs = current.tabs.filter((tab) => inspectionKey(tab) !== key);
      return {
        tabs,
        active:
          current.active === key
            ? (tabs[index - 1] ?? tabs[index])
              ? inspectionKey((tabs[index - 1] ?? tabs[index])!)
              : null
            : current.active,
      };
    });
  }
  const [asideDraft, setAsideDraft] = useState<AsideDraftSource | null>(null);
  const [asideChild, setAsideChild] = useState<Thread | null>(null);
  const [asideActivated, setAsideActivated] = useState(false);
  const [planContinuation, setPlanContinuation] =
    useState<PlanContinuation | null>(null);
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
            message_authors: {
              ...current.message_authors,
              ...value.message_authors,
            },
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
    if (!personalChat) heading.current?.focus();
  }, [personalChat]);
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
  const predictionTask = tasks[0];
  const nextPromptSource = useMemo(() => {
    if (!predictionTask?.run_id || taskStatus(predictionTask) !== "completed" || predictionTask.source_read_only || thread.active_task_id || thread.queued_task_ids?.length) return null;
    const output = projection.messages.filter(message => message.task_id === predictionTask.task_id && message.role === "assistant").map(message => message.text).join("\n");
    return output ? { key: JSON.stringify([project, thread.thread_id, predictionTask.run_id]), output } : null;
  }, [project, thread.thread_id, thread.active_task_id, thread.queued_task_ids?.length, predictionTask, projection.messages]);
  const loadPromptPrediction = useCallback((signal: AbortSignal) => predictionTask?.run_id ? loadNextPrompt(`/api/projects/${encodeURIComponent(project)}/tasks/${encodeURIComponent(predictionTask.task_id)}`, predictionTask.run_id, signal) : Promise.resolve(null), [project, predictionTask?.task_id, predictionTask?.run_id]);
  const textScope = JSON.stringify([
    project,
    initial.thread_id,
    initial.node_id,
    initial.session_id,
    initial.owner_user_id,
    personalChat?.owner,
  ]);
  const textScopeRef = useRef(textScope);
  textScopeRef.current = textScope;
  const textMessages = useMemo<ConversationTextMessage[]>(
    () =>
      detail.thread.thread_id !== initial.thread_id ||
      detail.thread.project_id !== project
        ? []
        : projection.turns.flatMap((turn) =>
            turn.messages.flatMap((message) =>
              message.role === "user" || message.role === "assistant"
                ? [
                    {
                      id: message.message_id,
                      role: message.role,
                      author:
                        message.role === "user"
                          ? detail.message_authors?.[message.message_id]
                              ?.display_name || "User"
                          : "Colossus",
                      createdAt: message.created_at,
                      text: message.text,
                    },
                  ]
                : [],
            ),
          ),
    [projection, detail, initial.thread_id, project],
  );
  const emptyFind = () => ({
    scope: textScope,
    open: false,
    query: "",
    selectedId: null as string | null,
  });
  const [findState, setFind] = useState(emptyFind);
  const find = findState.scope === textScope ? findState : emptyFind();
  const matches = useMemo(
    () => findConversationMessages(textMessages, find.query),
    [textMessages, find.query],
  );
  const selectedMatch = matches.includes(find.selectedId ?? "")
    ? find.selectedId
    : (matches[0] ?? null);
  const matchIndex = selectedMatch ? matches.indexOf(selectedMatch) : -1;
  const findTrigger = useRef<HTMLButtonElement>(null);
  const findInput = useRef<HTMLInputElement>(null);
  const textRows = useRef(new Map<string, HTMLDivElement>());
  const findInputId = useId();
  const findScopeId = useId();
  const [textFeedback, setTextFeedback] = useState<{
    scope: string;
    message: string;
    failed?: boolean;
  } | null>(null);
  const [copyingScope, setCopyingScope] = useState<string | null>(null);
  useEffect(() => {
    setFind({ scope: textScope, open: false, query: "", selectedId: null });
    setTextFeedback(null);
  }, [textScope]);
  useEffect(() => {
    if (find.open) findInput.current?.focus();
  }, [find.open, textScope, view]);
  useEffect(() => {
    if (!find.open) return;
    if (find.selectedId !== selectedMatch)
      setFind((current) =>
        current.scope === textScope && current.query === find.query
          ? { ...current, selectedId: selectedMatch }
          : current,
      );
    if (selectedMatch) {
      follow.current = false;
      textRows.current
        .get(selectedMatch)
        ?.scrollIntoView?.({ block: "center" });
    }
  }, [find.open, find.query, find.selectedId, selectedMatch, textScope, view]);
  function closeFind() {
    setFind({ ...find, open: false });
    findTrigger.current?.focus();
  }
  function findKey(event: ReactKeyboardEvent) {
    if (find.open && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeFind();
    }
  }
  function moveMatch(direction: number, focusRow = true) {
    if (!matches.length) return;
    const id =
      matches[(matchIndex + direction + matches.length) % matches.length]!;
    setFind({ ...find, selectedId: id });
    follow.current = false;
    const row = textRows.current.get(id);
    row?.scrollIntoView?.({ block: "center" });
    if (focusRow) row?.focus({ preventScroll: true });
  }
  const target =
    nodes.find((item) => item.node.node_id === thread.node_id) ??
    resolvedTarget;
  const composerNodes = target && !hasListedTarget ? [...nodes, target] : nodes;
  const modeCapabilities = useRunModeCapabilities(
    target ?? undefined,
    !permissions.includes("execute"),
  );
  const canInspectInboxes = Boolean(
    permissions.includes("read") &&
    target?.presence?.capabilities.includes("agent_messages.read.v1"),
  );
  const activeView =
    view === "inboxes" && !canInspectInboxes ? "conversation" : view;
  const plans = useMemo(
    () => selectSessionPlans(sessionRunViews(tasks, events)),
    [tasks, events],
  );
  const active = tasks.filter(
    (task) => !terminalStatuses.has(taskStatus(task)),
  );
  const running = active.find(
    (task) =>
      !task.source_read_only && task.run_id && taskStatus(task) !== "queued",
  );
  const earlierHistoryAvailable = Boolean(
    detail.next_message_cursor || detail.next_task_cursor,
  );
  function textSnapshot() {
    return conversationMarkdown({
      title: thread.title,
      messages: textMessages,
      earlierHistoryAvailable,
      incompleteHistory:
        thread.sync_status !== "current" ||
        tasks.some((task) => task.history_bounded || task.output_limited),
      streaming: active.length > 0,
    });
  }
  async function copyConversation() {
    const scope = textScope;
    const snapshot = textSnapshot();
    setCopyingScope(scope);
    try {
      await navigator.clipboard.writeText(snapshot);
      if (textScopeRef.current === scope)
        setTextFeedback({
          scope,
          message: "Loaded conversation copied as Markdown.",
        });
    } catch {
      if (textScopeRef.current === scope)
        setTextFeedback({
          scope,
          message:
            "Could not copy the conversation. Try downloading it instead.",
          failed: true,
        });
    } finally {
      setCopyingScope((current) => (current === scope ? null : current));
    }
  }
  function downloadConversation() {
    try {
      downloadConversationMarkdown(thread.title, textSnapshot());
      setTextFeedback({
        scope: textScope,
        message: "Loaded conversation downloaded as Markdown.",
      });
    } catch {
      setTextFeedback({
        scope: textScope,
        message: "Could not download the conversation. Try copying it instead.",
        failed: true,
      });
    }
  }
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
        message_authors: {
          ...current.message_authors,
          ...value.message_authors,
        },
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
            target.node.roles.join("\u0000") ||
          (runRequest.plan_action &&
            ![
              "revise" in runRequest.plan_action
                ? runRequest.plan_action.revise
                : runRequest.plan_action.execute,
            ].some((action) => {
              if (
                !action ||
                !personalChat ||
                latest.thread.owner_user_id !== personalChat.owner ||
                latest.thread.node_id !== personalChat.node
              )
                return false;
              return selectSessionPlans(sessionRunViews(latest.tasks, [])).some(
                (plan) =>
                  plan.sourceRunId === action.source_run_id &&
                  plan.revision === action.expected_revision &&
                  plan.status === "draft" &&
                  !plan.continuationPending,
              );
            }))
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
        personalChat && target?.presence?.ready
          ? ""
          : target?.presence?.ready
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
  const canContinuePlan = Boolean(
    canWrite &&
    permissions.includes("control") &&
    personalChat &&
    thread.owner_user_id === personalChat.owner &&
    thread.node_id === personalChat.node &&
    modeCapabilities.plans &&
    !active.length &&
    !thread.active_task_id &&
    !thread.queued_task_ids?.length,
  );
  const selectionAvailable =
    !planContinuation ||
    canContinuePlanFromRun(
      planContinuation.plan.sourceRunId,
      plans,
      canContinuePlan,
    );
  function choosePlan(plan: SessionPlanReference, strategy?: PlanStrategy) {
    if (!canContinuePlanFromRun(plan.sourceRunId, plans, canContinuePlan))
      return;
    const source = tasks.find(
      (task) =>
        task.run_id === plan.sourceRunId &&
        !task.source_read_only &&
        task.snapshot?.run.session_id === thread.session_id,
    );
    if (!source) return;
    setPlanContinuation({
      plan,
      role: source.request.role,
      ...(strategy ? { strategy } : {}),
    });
    setView("conversation");
  }
  const canBranch =
    !embedded &&
    permissions.includes("read") &&
    permissions.includes("execute") &&
    !thread.archived &&
    Boolean(
      personalChat &&
      thread.owner_user_id === personalChat.owner &&
      thread.node_id === personalChat.node,
    ) &&
    Boolean(target && !target.node.revoked);
  const forkSource =
    onFork && personalChat
      ? latestChatForkSource(thread, detail, personalChat.owner)
      : null;
  function inspectPlan(plan: SessionPlanReference) {
    setInspection({
      kind: "plan",
      sourceRunId: plan.sourceRunId,
      planId: plan.planId,
      revision: plan.revision,
    });
  }
  function isAsideSource(source: Task | null | undefined): source is Task {
    return Boolean(
      canBranch &&
      source &&
      !source.source_read_only &&
      source.run_id &&
      source.subject === personalChat?.owner &&
      source.thread_id === thread.thread_id &&
      source.node_id === thread.node_id &&
      source.project_id === project &&
      source.snapshot?.run.session_id === thread.session_id &&
      source.snapshot?.run.run_id === source.run_id &&
      !source.snapshot?.run.archived &&
      taskStatus(source) !== "queued",
    );
  }
  function startAside(source: Task, quote: string, messageId?: string) {
    if (!isAsideSource(source)) return;
    const sourceElement = [
      ...(conversation.current?.querySelectorAll<HTMLElement>(
        "[data-aside-message-id]",
      ) ?? []),
    ].find((element) => element.dataset.asideMessageId === messageId);
    if (sourceElement && !sourceElement.contains(document.activeElement))
      sourceElement.focus({ preventScroll: true });
    setAsideActivated(true);
    setAsideDraft({
      parent: thread,
      source,
      quote: boundedConversationQuote(quote),
      key: crypto.randomUUID(),
      ...(messageId ? { messageId } : {}),
    });
    setAsideChild(null);
    setInspection({ kind: "aside" });
  }
  const inspector = (
    <>
      {inspectionState.tabs.map((tab) => {
        const key = inspectionKey(tab);
        if (tab.kind === "aside") return null;
        const plan =
          tab.kind === "plan"
            ? plans.find(
                (plan) =>
                  plan.planId === tab.planId &&
                  plan.sourceRunId === tab.sourceRunId &&
                  plan.revision === tab.revision,
              )
            : null;
        return (
          <div
            key={key}
            className="shared-inspector-tool"
            hidden={inspectionState.active !== key}
          >
            {tab.kind === "artifact" ? (
              <ThreadArtifactInspection artifact={tab.artifact} />
            ) : plan ? (
              <ThreadPlanInspection
                plan={plan}
                available={
                  canContinuePlan &&
                  plan.status === "draft" &&
                  !plan.continuationPending
                }
                goalAvailable={modeCapabilities.goal}
                busy={busy}
                onRevise={(plan) => {
                  setInspection(null);
                  choosePlan(plan);
                }}
                onExecute={(plan, strategy) => {
                  setInspection(null);
                  choosePlan(plan, strategy);
                }}
              />
            ) : (
              <p className="shared-inspector-document" role="status">
                This plan revision is no longer in the released history. Review
                the latest plan.
              </p>
            )}
          </div>
        );
      })}
      <div
        className="shared-inspector-tool"
        hidden={inspection?.kind !== "aside"}
      >
        {asideActivated ? (
          <ThreadAsidePanel
            project={project}
            parent={thread}
            nodes={composerNodes}
            draft={asideDraft}
            selected={asideChild}
            disabled={!canBranch}
            onSelect={setAsideChild}
            onDiscard={() => setAsideDraft(null)}
            onChanged={() => {
              void refresh();
              onChanged();
            }}
            renderChild={(child) => (
              <ThreadDetail
                embedded
                initial={child}
                project={project}
                nodes={composerNodes}
                permissions={permissions}
                onBack={() => setAsideChild(null)}
                onChanged={onChanged}
                personalChat={personalChat}
                backLabel="Side chats"
              />
            )}
          />
        ) : null}
      </div>
    </>
  );
  const latestTask = running ?? tasks[0];
  const currentStatus = latestTask
    ? WORK_STATUS_PRESENTATIONS[taskStatus(latestTask)]
    : undefined;
  const startedAt =
    latestTask?.snapshot?.run.created_at ?? latestTask?.created_at;
  const startedDate = startedAt ? new Date(startedAt) : null;
  const startedLabel =
    startedDate && Number.isFinite(startedDate.getTime())
      ? startedDate.toLocaleString([], {
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        })
      : null;
  const content = (
    <section
      className={`detail thread-detail${personalChat ? " personal-chat-detail" : ""}${embedded ? " is-aside-detail" : ""}`}
      aria-labelledby={headingId}
    >
      <WorkSurfaceHeader
        variant="compact"
        title={
          thread.title || (personalChat ? "Untitled chat" : "Untitled thread")
        }
        titleId={headingId}
        titleRef={heading}
        statusLabel={
          thread.archived
            ? "Archived"
            : (currentStatus?.label ?? "Conversation")
        }
        status={currentStatus}
        startedLabel={startedLabel}
        modeLabel={
          latestTask
            ? statusLabel(latestTask.request.mode) + " mode"
            : undefined
        }
        details={
          <>
            {personalChat ? (
              <span className="chat-private-cue">
                <IconLock size={14} aria-hidden="true" /> Personal chat
              </span>
            ) : null}
            <span
              className={`thread-runtime-detail ${target?.presence?.ready ? "is-online" : ""}`}
            >
              <span
                className={`dot ${target?.presence?.ready ? "live" : ""}`}
                aria-hidden="true"
              />
              {personalChat || target?.node.execution_kind === "hosted"
                ? target?.presence?.ready
                  ? "Runtime ready"
                  : "Runtime unavailable"
                : target?.presence?.ready
                  ? "Agent online"
                  : "Agent offline"}
            </span>
          </>
        }
        editing={
          editing ? (
            <form
              className="thread-rename"
              onSubmit={(event) => {
                event.preventDefault();
                if (title.trim()) void metadata({ title: title.trim() });
              }}
            >
              <h2 id={headingId} className="sr-only">
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
          ) : undefined
        }
        actions={
          <div className="thread-header-actions">
            {!personalChat && backHref ? (
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
                  <span className="sr-only">{backLabel}</span>
                </a>
              </LinkButton>
            ) : !personalChat ? (
              <Button variant="tertiary" className="back" onClick={onBack}>
                <IconArrowLeft size={16} aria-hidden="true" />
                <span className="sr-only">{backLabel}</span>
              </Button>
            ) : null}
            {!embedded ? (
              <Button
                variant="tertiary"
                aria-label="Open side chats"
                onClick={() => {
                  setAsideActivated(true);
                  setInspection({ kind: "aside" });
                }}
              >
                <IconMessagePlus size={16} aria-hidden="true" />
                <span className="sr-only">Side chats</span>
              </Button>
            ) : null}
            <Button
              ref={findTrigger}
              variant="tertiary"
              aria-label="Find in loaded conversation"
              title="Find in loaded conversation"
              aria-expanded={find.open && activeView === "conversation"}
              onClick={() => {
                setView("conversation");
                setFind({
                  ...find,
                  open: !find.open || activeView !== "conversation",
                });
              }}
            >
              <IconSearch size={16} aria-hidden="true" />
            </Button>
            {permissions.includes("read") ||
            onConnections ||
            permissions.includes("control") ||
            (onFork && !embedded && canBranch) ? (
              <DropdownMenu
                open={actionsMenu.open}
                onOpenChange={actionsMenu.onOpenChange}
                modal={actionsMenu.modal}
              >
                <DropdownMenuTrigger asChild>
                  <LinkButton
                    variant="ghost"
                    size="icon"
                    ref={actionsMenu.triggerRef}
                    aria-label="Conversation actions"
                    disabled={busy}
                  >
                    <IconDots size={18} aria-hidden="true" />
                  </LinkButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="end"
                  ref={actionsMenu.contentRef}
                  container={actionsMenu.portalContainer}
                  onCloseAutoFocus={(event) => {
                    if (skipActionsFocus.current) {
                      event.preventDefault();
                      skipActionsFocus.current = false;
                    }
                  }}
                >
                  <DropdownMenuItem
                    disabled={
                      !textMessages.length || copyingScope === textScope
                    }
                    onSelect={() => void copyConversation()}
                  >
                    <IconCopy size={16} aria-hidden="true" />
                    Copy loaded conversation
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    disabled={!textMessages.length}
                    onSelect={downloadConversation}
                  >
                    <IconDownload size={16} aria-hidden="true" />
                    Download loaded conversation
                  </DropdownMenuItem>
                  {onConnections ? (
                    <DropdownMenuItem onSelect={onConnections}>
                      <IconPlugConnected size={16} aria-hidden="true" />
                      Manage your connections
                    </DropdownMenuItem>
                  ) : null}
                  {onFork && !embedded ? (
                    <DropdownMenuItem
                      disabled={!canBranch || !forkSource}
                      onSelect={() => {
                        if (forkSource) {
                          skipActionsFocus.current = true;
                          onFork(thread, forkSource);
                        }
                      }}
                    >
                      <IconGitFork size={16} aria-hidden="true" />
                      Fork chat
                    </DropdownMenuItem>
                  ) : null}
                  {permissions.includes("control") ? (
                    <>
                      <DropdownMenuItem
                        onSelect={() => {
                          skipActionsFocus.current = true;
                          setTitle(thread.title);
                          setEditing(true);
                        }}
                      >
                        <IconEdit size={16} aria-hidden="true" />
                        Rename thread
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        onSelect={() =>
                          void metadata({ archived: !thread.archived })
                        }
                      >
                        {thread.archived ? (
                          <IconArchiveOff size={16} aria-hidden="true" />
                        ) : (
                          <IconArchive size={16} aria-hidden="true" />
                        )}
                        {thread.archived ? "Restore" : "Archive"}
                      </DropdownMenuItem>
                    </>
                  ) : null}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
          </div>
        }
      />
      <nav className="session-workspace-tabs" aria-label="Session views">
        {SESSION_WORKSPACE_VIEWS.filter(
          (tab) =>
            (!embedded || tab.id === "conversation") &&
            (tab.id !== "inboxes" || canInspectInboxes),
        ).map((tab) => (
          <button
            key={tab.id}
            type="button"
            aria-current={activeView === tab.id ? "page" : undefined}
            onClick={() => setView(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </nav>
      <span className="sr-only" role="status">
        {connected ? "Live updates connected" : "Reconnecting updates"}
      </span>
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
      {textFeedback?.scope === textScope ? (
        <p
          className="sync-notice"
          role={textFeedback.failed ? "alert" : "status"}
        >
          {textFeedback.message}
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
        <span className="sr-only">
          You’re viewing saved history. The conversation remains assigned to its
          original agent.
        </span>
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
      {activeView === "conversation" && find.open ? (
        <div
          className="conversation-find"
          role="search"
          aria-label="Find in loaded conversation"
          onKeyDown={findKey}
        >
          <label htmlFor={findInputId}>Find loaded messages</label>
          <div className="conversation-find-controls">
            <TextInput
              id={findInputId}
              ref={findInput}
              type="search"
              value={find.query}
              aria-describedby={findScopeId}
              onChange={(event) =>
                setFind({
                  ...find,
                  query: event.target.value,
                  selectedId: null,
                })
              }
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  moveMatch(event.shiftKey ? -1 : 1, false);
                }
              }}
            />
            <span
              className="conversation-find-count"
              role="status"
              aria-live="polite"
            >
              {find.query.trim()
                ? matches.length
                  ? `${matchIndex + 1} of ${matches.length} matching messages`
                  : "No matching messages"
                : "Enter text to find"}
            </span>
            <Button
              aria-label="Previous matching message"
              disabled={!matches.length}
              onClick={() => moveMatch(-1)}
            >
              <IconChevronUp size={16} aria-hidden="true" />
            </Button>
            <Button
              aria-label="Next matching message"
              disabled={!matches.length}
              onClick={() => moveMatch(1)}
            >
              <IconChevronDown size={16} aria-hidden="true" />
            </Button>
            <Button aria-label="Close conversation search" onClick={closeFind}>
              <IconX size={16} aria-hidden="true" />
            </Button>
          </div>
          <p id={findScopeId}>
            Loaded user and assistant messages only.
            {earlierHistoryAvailable
              ? " Load earlier history to search more."
              : ""}
          </p>
        </div>
      ) : null}
      {activeView === "conversation" ? (
        <div
          className="thread-conversation"
          ref={conversation}
          tabIndex={0}
          aria-label="Conversation history"
          onKeyDown={findKey}
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
                split =
                  firstResponse < 0 ? turn.messages.length : firstResponse,
                task = turn.task,
                status = task ? taskStatus(task) : "";
              const lastResponse = [...turn.messages]
                .reverse()
                .find((message) => message.role === "assistant");
              const usage = releasedRunUsage(turn.updates);
              const messageEntry = (
                message: (typeof turn.messages)[number],
              ) => (
                <div
                  key={message.message_id}
                  tabIndex={-1}
                  ref={(element) => {
                    if (element)
                      textRows.current.set(message.message_id, element);
                    else textRows.current.delete(message.message_id);
                  }}
                  className={
                    find.open && selectedMatch === message.message_id
                      ? "conversation-find-active"
                      : undefined
                  }
                  aria-current={
                    find.open && selectedMatch === message.message_id
                      ? "true"
                      : undefined
                  }
                  data-aside-source-task-id={
                    isAsideSource(task) &&
                    ["user", "assistant"].includes(message.role)
                      ? task.task_id
                      : undefined
                  }
                  data-aside-message-id={message.message_id}
                >
                  <ConversationEntry
                    role={
                      message.role === "user"
                        ? "user"
                        : message.role === "assistant"
                          ? "assistant"
                          : "system"
                    }
                    author={
                      message.role === "user"
                        ? detail.message_authors?.[message.message_id]
                            ?.display_name || "User"
                        : message.role === "assistant"
                          ? "Colossus"
                          : statusLabel(message.role)
                    }
                    createdAt={message.created_at}
                    content={message.text}
                    streaming={
                      message === lastResponse &&
                      Boolean(task && !terminalStatuses.has(taskStatus(task)))
                    }
                    usage={message === lastResponse ? usage : undefined}
                    usageLabel="Reported run usage"
                    actions={
                      isAsideSource(task) &&
                      ["user", "assistant"].includes(message.role) ? (
                        <Button
                          variant="tertiary"
                          onClick={() =>
                            startAside(task, message.text, message.message_id)
                          }
                        >
                          <IconMessagePlus size={14} aria-hidden="true" /> Ask
                          in side chat
                        </Button>
                      ) : undefined
                    }
                    linkComponent={WebLink}
                    status={
                      message.status ? (
                        <Badge className={`status status-${message.status}`}>
                          {statusLabel(message.status)}
                        </Badge>
                      ) : null
                    }
                  />
                </div>
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
                  {personalChat ? (
                    <HostedFileLinks project={project} task={task} />
                  ) : null}
                  {task ? (
                    <div
                      className="conversation-run"
                      data-task-id={task.task_id}
                    >
                      {!terminalStatuses.has(status) ||
                      task.dispatch_error ||
                      task.output_limited ? (
                        <div className="run-progress" role="status">
                          {!terminalStatuses.has(status) ? (
                            <IconLoader2
                              size={15}
                              className={
                                status === "queued" ? undefined : "spin"
                              }
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
                      {plans
                        .filter((plan) => plan.sourceRunId === task.run_id)
                        .map((plan) => (
                          <ThreadPlanDecision
                            key={`${plan.planId}:${plan.revision}`}
                            plan={plan}
                            onInspect={inspectPlan}
                            available={
                              canContinuePlan &&
                              !task.source_read_only &&
                              task.snapshot?.run.session_id ===
                                thread.session_id
                            }
                            goalAvailable={modeCapabilities.goal}
                            busy={busy}
                            onRevise={(selected) => choosePlan(selected)}
                            onExecute={(selected, strategy) =>
                              choosePlan(selected, strategy)
                            }
                          />
                        ))}
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
      ) : activeView === "inboxes" ? (
        <section
          className="thread-session-inboxes"
          aria-label="Session inboxes"
        >
          {tasks
            .filter((task) => task.run_id && !task.source_read_only)
            .map((task) => (
              <ThreadAgentInboxes
                key={task.task_id}
                project={project}
                task={task}
                available={canInspectInboxes}
              />
            ))}
          {!tasks.some((task) => task.run_id && !task.source_read_only) ? (
            <p>No inspectable runs have been saved in this session.</p>
          ) : null}
        </section>
      ) : (
        <ThreadSessionViews
          view={activeView}
          tasks={tasks}
          updates={events}
          onChangeView={setView}
          onInspectPlan={inspectPlan}
          onInspectArtifact={(artifact) =>
            setInspection({ kind: "artifact", artifact })
          }
          canContinuePlan={canContinuePlan}
          onRevisePlan={(plan) => choosePlan(plan)}
          onExecutePlan={(plan, strategy) => choosePlan(plan, strategy)}
          goalAvailable={modeCapabilities.goal}
          busy={busy}
        />
      )}
      {activeView === "conversation" && below ? (
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
      {!embedded ? (
        <ConversationSelectionLauncher
          containerRef={conversation}
          scopeKey={`${project}:${thread.thread_id}`}
          enabled={canBranch && activeView === "conversation"}
          resolveSource={(element) => {
            const wrapper = element.closest<HTMLElement>(
              "[data-aside-source-task-id]",
            );
            const task = tasks.find(
              (item) =>
                item.task_id === wrapper?.dataset.asideSourceTaskId &&
                isAsideSource(item),
            );
            return task && wrapper?.dataset.asideMessageId
              ? { task, messageId: wrapper.dataset.asideMessageId }
              : null;
          }}
          onOpen={({ source, quote }) =>
            startAside(source.task, quote, source.messageId)
          }
        />
      ) : null}
      <RunComposer
        nextPromptSource={nextPromptSource}
        loadPromptPrediction={loadPromptPrediction}
        onPolicy={onPolicy}
        nodes={composerNodes}
        nodeId={thread.node_id}
        lockedRuntime
        runtimeLabel={
          embedded ? "Side chat" : personalChat ? "Personal chat" : undefined
        }
        hostedProject={personalChat ? project : undefined}
        initialRequest={initialComposerRequest}
        busy={busy}
        running={Boolean(running)}
        queuedTurns={queuedTurns}
        onStop={
          running && permissions.includes("control")
            ? () => void cancel()
            : undefined
        }
        stopping={
          controlling ||
          Boolean(running && taskStatus(running) === "cancelling")
        }
        stopDisabled={!target || target.node.revoked}
        disabled={!canWrite || !selectionAvailable}
        label={embedded ? "Message side chat" : "Message the agent"}
        action={
          active.length || thread.active_task_id
            ? "Queue message"
            : "Send message"
        }
        planContinuation={planContinuation}
        onCancelPlan={() => setPlanContinuation(null)}
        onSubmit={async (request) => {
          if (request.plan_action && (!canContinuePlan || !selectionAvailable))
            return false;
          const submitted = await send(request);
          if (submitted && request.plan_action) setPlanContinuation(null);
          return submitted;
        }}
      />
      {!selectionAvailable ? (
        <p className="composer-offline">
          This Plan revision is no longer available to continue. Cancel this
          action and review the latest plan.
        </p>
      ) : null}
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
  return embedded ? (
    content
  ) : (
    <InspectorLayout
      title={
        inspection?.kind === "plan"
          ? "Plan"
          : inspection?.kind === "artifact"
            ? "Artifact"
            : "Side chats"
      }
      tabs={inspectionState.tabs.map((tab) => ({
        id: inspectionKey(tab),
        label:
          tab.kind === "aside"
            ? "Side chats"
            : tab.kind === "artifact"
              ? tab.artifact.fileName
              : plans.find(
                  (plan) =>
                    plan.planId === tab.planId &&
                    plan.sourceRunId === tab.sourceRunId &&
                    plan.revision === tab.revision,
                )?.sourceRunTitle || `Plan · revision ${tab.revision}`,
      }))}
      activeTab={inspectionState.active ?? undefined}
      onTabChange={(key) =>
        setInspectionState((current) => ({ ...current, active: key }))
      }
      onTabClose={closeInspection}
      inspector={inspector}
      open={inspection !== null}
      onClose={() => setInspection(null)}
    >
      {content}
    </InspectorLayout>
  );
}
