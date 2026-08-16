import {
  IconAlertTriangle,
  IconArrowRight,
  IconBrain,
  IconBooks,
  IconCheck,
  IconChevronDown,
  IconCircle,
  IconEdit,
  IconFile,
  IconInfoCircle,
  IconLoader2,
  IconMessageCircle,
  IconPlayerPlay,
  IconPlayerStop,
  IconSparkles,
  IconTargetArrow,
  IconTerminal2,
} from "@tabler/icons-react";
import { useState } from "react";
import type { ReactNode } from "react";

import colossusMark from "../assets/colossus-mark.svg";
import type { RunView } from "../state";
import type {
  MessageContentPart,
  RunFailure,
  PlanStatus,
  RunTerminal,
  RunUpdate,
  SessionMessage,
  ToolActivity,
} from "../types";
import { MarkdownContent } from "./MarkdownContent";
import { researchSources } from "./ResearchSourcesPanel";

interface RunTimelineProps {
  view: RunView;
  activityPresentation?: ActivityPresentation;
  activityComparison?: boolean;
  planContinuationAvailable?: boolean;
  planWorkflowAvailable?: boolean;
  onOpenPlanWorkflow?: (sessionId: string, planId: string) => void;
  onRevisePlan?: (
    sourceRunId: string,
    planId: string,
    revision: number,
  ) => void;
  onExecutePlan?: (
    sourceRunId: string,
    planId: string,
    revision: number,
    strategy: { type: "direct" } | { type: "goal"; maxIterations: number },
  ) => Promise<void>;
  onOpenResearchSources?: () => void;
}

export type ActivityPresentation = "capsule" | "thread";

type ToolActivityUpdate = RunUpdate & {
  update: Extract<RunUpdate["update"], { type: "tool_activity" }>;
};

interface ToolActivityGroup {
  key: string;
  updates: ToolActivityUpdate[];
}

type TimelineItem =
  | { type: "update"; update: RunUpdate }
  | { type: "tool_activity"; group: ToolActivityGroup };

function readable(value: string): string {
  return value.replaceAll("_", " ");
}

function compactTime(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime())
    ? "Recent"
    : date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function FailureMetadata({ failure }: { failure: RunFailure }) {
  const values = [
    ["Code", failure.reason],
    failure.httpStatus == null
      ? null
      : ["Response", `HTTP ${failure.httpStatus}`],
    failure.retryAfterMs == null
      ? null
      : ["Retry after", `Retry after ${failure.retryAfterMs} ms`],
    [
      "Recovery",
      failure.recoverable === true ? "Recoverable" : "Not recoverable",
    ],
  ].filter((value): value is string[] => value !== null);
  return (
    <dl className="failure-metadata" aria-label="Failure details">
      {values.map(([label, value]) => (
        <div key={label}>
          <dt className="sr-only">{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function formatBytes(size: number): string {
  if (!Number.isFinite(size) || size < 0) {
    return "Unknown size";
  }
  if (size < 1024) {
    return `${size} B`;
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} KB`;
  }
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function ContentPart({ part }: { part: MessageContentPart }) {
  if (part.type === "text") {
    return <span className="preserve-lines">{part.text}</span>;
  }
  return (
    <span className="artifact-chip">
      <IconFile size={16} stroke={1.7} aria-hidden="true" />
      <span>
        {part.artifact.fileName} · {formatBytes(part.artifact.sizeBytes)}
      </span>
    </span>
  );
}

function ResearchResponse({
  output,
  onOpenSources,
}: {
  output: string;
  onOpenSources?: (() => void) | undefined;
}) {
  const sources = researchSources(output);
  const heading = /^## Sources\s*$/m.exec(output);
  if (sources.length === 0 || heading?.index === undefined) {
    return <MarkdownContent content={output} />;
  }
  const report = output.slice(0, heading.index).trimEnd();
  return (
    <>
      {report === "" ? null : <MarkdownContent content={report} />}
      <section className="inline-research-sources" aria-label="Report sources">
        <button type="button" onClick={onOpenSources}>
          <IconBooks size={16} stroke={1.7} aria-hidden="true" />
          <strong>Sources</strong>
          <span>{sources.length}</span>
          <span>View evidence</span>
        </button>
        <ol>
          {sources.map((source) => (
            <li key={`${source.label}:${source.uri}`}>
              <button type="button" onClick={onOpenSources}>
                <span>{source.label}</span>
                <span>{source.title}</span>
              </button>
            </li>
          ))}
        </ol>
      </section>
    </>
  );
}

function Message({ message }: { message: SessionMessage }) {
  if (message.role === "assistant") {
    return null;
  }
  const label = message.role === "user" ? "You" : readable(message.role);
  return (
    <article className={`feed-entry message message-${message.role}`}>
      <div className="feed-marker" aria-hidden="true">
        <IconMessageCircle size={17} stroke={1.7} />
      </div>
      <div className="feed-entry-content">
        <header className="feed-entry-heading">
          <strong>{label}</strong>
          <time dateTime={message.createdAt}>
            {compactTime(message.createdAt)}
          </time>
        </header>
        <div className="message-body" data-aside-selectable="true">
          {message.content.map((part, index) => (
            <ContentPart key={`${message.sequence}-${index}`} part={part} />
          ))}
        </div>
      </div>
    </article>
  );
}

function planSteps(summary: string): string[] {
  const lines = summary.split("\n").map((line) => line.trim());
  if (lines[0]?.toLocaleLowerCase() !== "plan") {
    return [];
  }
  return lines
    .slice(1)
    .map((line) => line.replace(/^\d+[.)]\s*/, ""))
    .filter(Boolean);
}

function ReasoningSummary({ item }: { item: RunUpdate }) {
  if (item.update.type !== "reasoning_summary") {
    return null;
  }
  const steps = planSteps(item.update.summary);
  return (
    <details className="feed-entry reasoning-card" open={steps.length > 0}>
      <summary>
        <span className="feed-marker" aria-hidden="true">
          <IconBrain size={17} stroke={1.7} />
        </span>
        <span>
          <strong>{steps.length > 0 ? "Plan" : "Reasoning summary"}</strong>
          <small>{compactTime(item.createdAt)}</small>
        </span>
      </summary>
      {steps.length > 0 ? (
        <ol className="plan-steps">
          {steps.map((step) => (
            <li key={step}>
              <IconCircle size={14} stroke={1.7} aria-hidden="true" />
              <span>{step}</span>
            </li>
          ))}
        </ol>
      ) : (
        <p className="preserve-lines">{item.update.summary}</p>
      )}
    </details>
  );
}

function isLifecycleNotice(item: RunUpdate): boolean {
  return (
    item.update.type === "notice" &&
    (item.update.reason.startsWith("run.phase.") ||
      item.update.reason === "model.final_output")
  );
}

function compactTimelineItems(updates: readonly RunUpdate[]): TimelineItem[] {
  const toolGroups = new Map<string, ToolActivityGroup>();
  for (const item of updates) {
    if (item.update.type !== "tool_activity") {
      continue;
    }
    const key =
      item.update.activity.callId.trim() === ""
        ? `sequence-${item.sequence}`
        : item.update.activity.callId;
    const group = toolGroups.get(key) ?? { key, updates: [] };
    group.updates.push(item as ToolActivityUpdate);
    toolGroups.set(key, group);
  }

  const emittedTools = new Set<string>();
  const items: TimelineItem[] = [];
  for (const item of updates) {
    if (isLifecycleNotice(item)) {
      continue;
    }
    if (item.update.type !== "tool_activity") {
      items.push({ type: "update", update: item });
      continue;
    }
    const key =
      item.update.activity.callId.trim() === ""
        ? `sequence-${item.sequence}`
        : item.update.activity.callId;
    if (emittedTools.has(key)) {
      continue;
    }
    emittedTools.add(key);
    const group = toolGroups.get(key);
    if (group !== undefined) {
      items.push({ type: "tool_activity", group });
    }
  }
  return items;
}

function formatToolActivityText(value: string): string {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

function toolActivityInput(group: ToolActivityGroup): string | null {
  for (let index = group.updates.length - 1; index >= 0; index -= 1) {
    const input = group.updates[index]?.update.activity.input;
    if (input?.trim()) {
      return formatToolActivityText(input);
    }
  }
  return null;
}

function toolActivityPreview(activity: ToolActivity): string {
  const releasedPreview = activity.preview;
  if (releasedPreview?.trim()) {
    return formatToolActivityText(releasedPreview);
  }
  switch (activity.state) {
    case "requested":
      return "No preview is available until the tool starts.";
    case "waiting_approval":
      return "No preview is available while the tool is waiting for approval.";
    case "started":
      return "No preview is available while the tool is still running.";
    case "completed":
      return "The tool completed, but this activity feed does not include an output preview.";
    case "cancelled":
      return "No preview was generated because the tool was cancelled before it started.";
    case "failed":
      return "No preview was generated because the tool did not complete successfully.";
    case "outcome_unknown":
      return "A preview is unavailable because the tool's final outcome is unknown.";
  }
}

function ToolActivityItem({
  group,
  presentation,
}: {
  group: ToolActivityGroup;
  presentation: ActivityPresentation;
}) {
  const latest = group.updates.at(-1);
  if (latest === undefined || latest.update.type !== "tool_activity") {
    return null;
  }
  const activity = latest.update.activity;
  const complete = activity.state === "completed";
  const input = toolActivityInput(group);
  return (
    <details
      className={`compact-tool-activity activity-tool-${presentation} activity-state-${activity.state}`}
    >
      <summary>
        <span className="feed-marker" aria-hidden="true">
          {complete ? (
            <IconCheck size={16} stroke={2} />
          ) : (
            <IconTerminal2 size={16} stroke={1.7} />
          )}
        </span>
        <span className="compact-tool-copy">
          <span className="compact-tool-heading">
            <strong>{activity.summary}</strong>
            <span className="compact-tool-name">{activity.toolName}</span>
            <span className={`event-state tool-state-${activity.state}`}>
              {readable(activity.state)}
            </span>
            <time dateTime={latest.createdAt}>
              {compactTime(latest.createdAt)}
            </time>
          </span>
        </span>
        <IconChevronDown
          className="compact-tool-chevron"
          size={16}
          stroke={1.8}
          aria-hidden="true"
        />
      </summary>
      <ol className="tool-activity-history">
        {group.updates.map((item) => {
          const update = item.update;
          return (
            <li key={item.sequence}>
              <span
                className={`tool-history-state state-${update.activity.state}`}
              >
                {readable(update.activity.state)}
              </span>
              <span>{update.activity.summary}</span>
              <time dateTime={item.createdAt}>
                {compactTime(item.createdAt)}
              </time>
            </li>
          );
        })}
      </ol>
      {input !== null ? (
        <section className="tool-activity-input" aria-label="Tool input">
          <strong>Input</strong>
          <pre>{input}</pre>
        </section>
      ) : null}
      <section
        className="tool-activity-preview"
        aria-label="Tool output preview"
      >
        <strong>Preview</strong>
        <pre>{toolActivityPreview(activity)}</pre>
      </section>
    </details>
  );
}

function isUserMessageItem(item: TimelineItem): boolean {
  return (
    item.type === "update" &&
    item.update.update.type === "message" &&
    item.update.update.message.role === "user"
  );
}

function isVisibleActivityItem(item: TimelineItem): boolean {
  if (item.type === "tool_activity") {
    return true;
  }
  switch (item.update.update.type) {
    case "reasoning_summary":
    case "notice":
    case "failure":
    case "cancellation":
      return true;
    case "message":
      return (
        item.update.update.message.role === "tool" ||
        item.update.update.message.role === "system"
      );
    case "state":
    case "output_delta":
    case "usage":
    case "interaction":
    case "result":
    case "tool_activity":
      return false;
  }
}

function activityDuration(view: RunView): string {
  const terminalSeconds =
    view.run.terminal?.type === "result"
      ? view.run.terminal.result.elapsedSeconds
      : null;
  if (terminalSeconds !== null && Number.isFinite(terminalSeconds)) {
    return `${Math.max(0, Math.round(terminalSeconds))}s`;
  }
  const start = new Date(view.run.startedAt ?? view.run.createdAt).getTime();
  const finish = new Date(view.run.finishedAt ?? view.run.updatedAt).getTime();
  if (Number.isNaN(start) || Number.isNaN(finish) || finish < start) {
    return "";
  }
  return `${Math.round((finish - start) / 1_000)}s`;
}

function activityStatus(view: RunView): {
  label: string;
  tone: "success" | "warning" | "danger" | "active" | "neutral";
} {
  switch (view.run.status) {
    case "completed":
      return { label: "Completed", tone: "success" };
    case "failed":
    case "outcome_unknown":
      return { label: "Failed", tone: "danger" };
    case "cancelled":
    case "interrupted":
      return { label: "Stopped", tone: "neutral" };
    case "waiting":
      return { label: "Needs input", tone: "warning" };
    case "cancelling":
      return { label: "Stopping", tone: "warning" };
    case "queued":
    case "running":
      return { label: "Working", tone: "active" };
  }
}

function ActivityThought({ item }: { item: RunUpdate }) {
  if (item.update.type !== "reasoning_summary") {
    return null;
  }
  return (
    <article className="activity-thought">
      <span className="activity-thread-marker" aria-hidden="true">
        <IconSparkles size={16} stroke={1.8} />
      </span>
      <p className="preserve-lines">{item.update.summary}</p>
      <time dateTime={item.createdAt}>{compactTime(item.createdAt)}</time>
    </article>
  );
}

function ActivityItem({
  item,
  presentation,
}: {
  item: TimelineItem;
  presentation: ActivityPresentation;
}) {
  if (item.type === "tool_activity") {
    return <ToolActivityItem group={item.group} presentation={presentation} />;
  }
  if (item.update.update.type === "reasoning_summary") {
    return <ActivityThought item={item.update} />;
  }
  return <FeedItem item={item.update} />;
}

function RunActivity({
  view,
  items,
  presentation,
  comparison,
}: {
  view: RunView;
  items: readonly TimelineItem[];
  presentation: ActivityPresentation;
  comparison: boolean;
}) {
  const toolActionCount = items.filter(
    (item) => item.type === "tool_activity",
  ).length;
  const researchStepCount = items.filter(
    (item) =>
      item.type === "update" &&
      item.update.update.type === "notice" &&
      item.update.update.reason.startsWith("research."),
  ).length;
  const actionCount = toolActionCount + researchStepCount;
  const noteCount = items.filter(
    (item) =>
      item.type === "update" && item.update.update.type === "reasoning_summary",
  ).length;
  const failedActionCount = items.filter(
    (item) =>
      item.type === "tool_activity" &&
      ["failed", "outcome_unknown"].includes(
        item.group.updates.at(-1)?.update.activity.state ?? "",
      ),
  ).length;
  const duration = activityDuration(view);
  const status = activityStatus(view);
  const active = ["queued", "running", "waiting", "cancelling"].includes(
    view.run.status,
  );
  const summaryParts = [
    view.run.mode === "research"
      ? `${actionCount} ${actionCount === 1 ? "step" : "steps"}`
      : `${actionCount} ${actionCount === 1 ? "action" : "actions"}`,
    noteCount > 0 ? `${noteCount} ${noteCount === 1 ? "note" : "notes"}` : null,
    duration === "" ? null : duration,
  ].filter((part): part is string => part !== null);

  return (
    <details
      className={`run-activity run-activity-${presentation} run-state-${view.run.status}`}
      open={comparison || active || failedActionCount > 0}
    >
      <summary className="run-activity-summary">
        <span className="run-activity-chevron" aria-hidden="true">
          <IconChevronDown size={16} stroke={1.9} />
        </span>
        <span className="run-activity-mark" aria-hidden="true">
          {presentation === "capsule" ? (
            <IconBrain size={17} stroke={1.7} />
          ) : (
            <img src={colossusMark} alt="" />
          )}
        </span>
        <span className="run-activity-title">
          <strong>
            {presentation === "capsule" ? view.run.title : "Colossus"}
          </strong>
          <small>{summaryParts.join(" · ")}</small>
        </span>
        <span className={`run-activity-status tone-${status.tone}`}>
          {status.tone === "success" ? (
            <IconCheck size={15} stroke={2} aria-hidden="true" />
          ) : status.tone === "danger" ? (
            <IconAlertTriangle size={15} stroke={1.9} aria-hidden="true" />
          ) : null}
          {status.label}
        </span>
        {failedActionCount > 0 ? (
          <span className="run-activity-exceptions">
            <IconAlertTriangle size={14} stroke={1.8} aria-hidden="true" />
            {failedActionCount}
          </span>
        ) : null}
      </summary>
      <div className="run-activity-body">
        {items.map((item) => (
          <ActivityItem
            item={item}
            presentation={presentation}
            key={
              item.type === "tool_activity"
                ? `tool-${item.group.key}`
                : item.update.sequence
            }
          />
        ))}
      </div>
    </details>
  );
}

function liveRunStatus(view: RunView): { label: string; detail: string } {
  const notice = [...view.updates]
    .reverse()
    .find(
      (item) =>
        item.update.type === "notice" &&
        item.update.reason.startsWith("run.phase."),
    );
  if (notice?.update.type !== "notice") {
    return {
      label: view.run.status === "queued" ? "Preparing…" : "Working…",
      detail: "",
    };
  }
  const detailSeparator = notice.update.message.indexOf(": ");
  const detail =
    detailSeparator === -1
      ? ""
      : notice.update.message.slice(detailSeparator + 2);
  switch (notice.update.reason) {
    case "run.phase.preparing":
      return { label: "Preparing…", detail: "" };
    case "run.phase.waiting_for_model":
      return { label: "Waiting for model…", detail };
    case "run.phase.responding":
      return { label: "Responding…", detail: "" };
    case "run.phase.cancelling":
      return { label: "Cancelling…", detail: "" };
    default:
      return { label: "Working…", detail: "" };
  }
}

function LiveRunStatus({ view }: { view: RunView }) {
  const status = liveRunStatus(view);
  return (
    <div className="feed-entry live-run-status">
      <span className="feed-marker" aria-hidden="true">
        <IconLoader2 size={16} stroke={1.8} />
      </span>
      <span className="live-run-status-copy">
        <strong>{status.label}</strong>
        {status.detail === "" ? null : <small>{status.detail}</small>}
      </span>
    </div>
  );
}

function FeedItem({ item }: { item: RunUpdate }): ReactNode {
  const update = item.update;
  switch (update.type) {
    case "message":
      return <Message message={update.message} />;
    case "reasoning_summary":
      return <ReasoningSummary item={item} />;
    case "tool_activity":
      return null;
    case "notice": {
      const handoff = update.reason.includes("handoff");
      return (
        <article className="feed-entry process-row notice-row">
          <div className="feed-marker" aria-hidden="true">
            {handoff ? (
              <IconArrowRight size={17} stroke={1.8} />
            ) : (
              <IconInfoCircle size={17} stroke={1.7} />
            )}
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <strong>{readable(update.reason)}</strong>
              <time dateTime={item.createdAt}>
                {compactTime(item.createdAt)}
              </time>
            </header>
            <p>{update.message}</p>
          </div>
        </article>
      );
    }
    case "failure":
      return (
        <article className="feed-entry process-row failure-row" role="alert">
          <div className="feed-marker" aria-hidden="true">
            <IconAlertTriangle size={17} stroke={1.8} />
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <strong className="failure-title">Run failed</strong>
              <time dateTime={item.createdAt}>
                {compactTime(item.createdAt)}
              </time>
            </header>
            <p>{update.failure.message}</p>
            <FailureMetadata failure={update.failure} />
            {update.failure.outcomeCertainty === "unknown" ? (
              <p className="outcome-warning">
                The external outcome is unknown. Do not retry automatically.
              </p>
            ) : null}
          </div>
        </article>
      );
    case "cancellation":
      return (
        <article className="feed-entry process-row notice-row">
          <div className="feed-marker" aria-hidden="true">
            <IconPlayerStop size={17} stroke={1.7} />
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <strong>Run cancelled</strong>
              <time dateTime={item.createdAt}>
                {compactTime(item.createdAt)}
              </time>
            </header>
            <p>{update.cancellation.message}</p>
          </div>
        </article>
      );
    case "state":
    case "output_delta":
    case "usage":
    case "interaction":
    case "result":
      return null;
  }
}

function TerminalSummary({ terminal }: { terminal: RunTerminal }) {
  if (terminal.type === "failure") {
    return (
      <article className="feed-entry process-row failure-row" role="alert">
        <div className="feed-marker" aria-hidden="true">
          <IconAlertTriangle size={17} stroke={1.8} />
        </div>
        <div className="feed-entry-content">
          <strong className="failure-title">Run failed</strong>
          <p>{terminal.failure.message}</p>
          <FailureMetadata failure={terminal.failure} />
          {terminal.failure.outcomeCertainty === "unknown" ? (
            <p className="outcome-warning">
              The external outcome is unknown. Do not retry automatically.
            </p>
          ) : null}
        </div>
      </article>
    );
  }
  if (terminal.type === "cancellation") {
    return (
      <article className="feed-entry process-row notice-row">
        <div className="feed-marker" aria-hidden="true">
          <IconPlayerStop size={17} stroke={1.7} />
        </div>
        <div className="feed-entry-content">
          <strong>Run cancelled</strong>
          <p>{terminal.cancellation.message}</p>
        </div>
      </article>
    );
  }
  return null;
}

interface PlanReference {
  planId: string;
  revision?: number;
  status?: PlanStatus;
  goalId?: string;
}

function terminalPlanReference(
  terminal: RunTerminal | null,
): PlanReference | null {
  if (terminal?.type === "result") {
    const planId = terminal.result.planId;
    return planId === undefined
      ? null
      : {
          planId,
          ...(terminal.result.planRevision === undefined
            ? {}
            : { revision: terminal.result.planRevision }),
          ...(terminal.result.planStatus === undefined
            ? {}
            : { status: terminal.result.planStatus }),
          ...(terminal.result.goalId === undefined
            ? {}
            : { goalId: terminal.result.goalId }),
        };
  }
  if (terminal?.type === "cancellation") {
    const planId = terminal.cancellation.planId;
    return planId === undefined
      ? null
      : {
          planId,
          ...(terminal.cancellation.planRevision === undefined
            ? {}
            : { revision: terminal.cancellation.planRevision }),
          ...(terminal.cancellation.planStatus === undefined
            ? {}
            : { status: terminal.cancellation.planStatus }),
          ...(terminal.cancellation.goalId === undefined
            ? {}
            : { goalId: terminal.cancellation.goalId }),
        };
  }
  return null;
}

function PlanResultCard({
  sourceRunId,
  plan,
  sessionId,
  cancelled,
  continuationAvailable,
  workflowAvailable,
  onOpenWorkflow,
  onRevise,
  onExecute,
}: {
  sourceRunId: string;
  plan: PlanReference;
  sessionId: string;
  cancelled: boolean;
  continuationAvailable: boolean;
  workflowAvailable: boolean;
  onOpenWorkflow: ((sessionId: string, planId: string) => void) | undefined;
  onRevise:
    | ((sourceRunId: string, planId: string, revision: number) => void)
    | undefined;
  onExecute:
    | ((
        sourceRunId: string,
        planId: string,
        revision: number,
        strategy: { type: "direct" } | { type: "goal"; maxIterations: number },
      ) => Promise<void>)
    | undefined;
}) {
  const [goalIterations, setGoalIterations] = useState(5);
  const [busyAction, setBusyAction] = useState<"direct" | "goal" | null>(null);
  const actionable =
    plan.revision !== undefined &&
    plan.status === "draft" &&
    continuationAvailable &&
    onRevise !== undefined &&
    onExecute !== undefined;
  const executed = plan.status === "executed";

  async function execute(
    strategy: { type: "direct" } | { type: "goal"; maxIterations: number },
  ) {
    if (plan.revision === undefined || onExecute === undefined) {
      return;
    }
    setBusyAction(strategy.type);
    try {
      await onExecute(sourceRunId, plan.planId, plan.revision, strategy);
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <article className="feed-entry plan-result-card">
      <div className="feed-marker" aria-hidden="true">
        <IconCheck size={17} stroke={2} />
      </div>
      <div className="feed-entry-content">
        <header className="feed-entry-heading">
          <strong>
            {executed
              ? plan.goalId === undefined
                ? "Plan executed"
                : "Plan started as a Goal"
              : cancelled
                ? "Draft saved before cancellation"
                : "Plan ready for your decision"}
          </strong>
          <span>
            {plan.revision === undefined
              ? "Durable Plan"
              : `Revision ${plan.revision}`}
          </span>
        </header>
        <p>
          {executed
            ? plan.goalId === undefined
              ? "The approved revision was consumed by a policy-bound execution run."
              : "The approved revision was consumed by bounded Goal Mode."
            : actionable
              ? "Revise it in this chat, run it once, or hand it to bounded Goal Mode."
              : "Open the advanced Plan workflow to inspect or continue this durable Plan."}
        </p>
        {actionable ? (
          <div className="plan-decision-actions">
            <button
              className="button secondary compact"
              type="button"
              disabled={busyAction !== null}
              onClick={() =>
                onRevise?.(sourceRunId, plan.planId, plan.revision ?? 0)
              }
            >
              <IconEdit size={15} stroke={1.8} aria-hidden="true" />
              Revise in chat
            </button>
            <button
              className="button primary compact"
              type="button"
              disabled={busyAction !== null}
              onClick={() => void execute({ type: "direct" })}
            >
              {busyAction === "direct" ? (
                <span className="spinner" aria-hidden="true" />
              ) : (
                <IconPlayerPlay size={15} stroke={1.8} aria-hidden="true" />
              )}
              Run once
            </button>
            <div className="plan-goal-action">
              <label>
                <span className="sr-only">Goal iteration budget</span>
                <select
                  aria-label="Goal iteration budget"
                  value={goalIterations}
                  disabled={busyAction !== null}
                  onChange={(event) =>
                    setGoalIterations(Number(event.target.value))
                  }
                >
                  <option value={3}>3 iterations</option>
                  <option value={5}>5 iterations</option>
                  <option value={10}>10 iterations</option>
                  <option value={20}>20 iterations</option>
                </select>
              </label>
              <button
                className="button secondary compact"
                type="button"
                disabled={busyAction !== null}
                onClick={() =>
                  void execute({
                    type: "goal",
                    maxIterations: goalIterations,
                  })
                }
              >
                {busyAction === "goal" ? (
                  <span className="spinner" aria-hidden="true" />
                ) : (
                  <IconTargetArrow size={15} stroke={1.8} aria-hidden="true" />
                )}
                Run as Goal
              </button>
            </div>
          </div>
        ) : null}
        <div className="plan-workflow-handoff">
          <details>
            <summary>Plan details</summary>
            <dl className="plan-workflow-identifiers">
              <div>
                <dt>Session</dt>
                <dd>{sessionId}</dd>
              </div>
              <div>
                <dt>Plan</dt>
                <dd>{plan.planId}</dd>
              </div>
              {plan.goalId === undefined ? null : (
                <div>
                  <dt>Goal</dt>
                  <dd>{plan.goalId}</dd>
                </div>
              )}
            </dl>
          </details>
          <button
            className="button tertiary compact"
            type="button"
            disabled={!workflowAvailable}
            title={
              workflowAvailable
                ? "Open the authenticated Colossus TUI"
                : "Enable the local Colossus TUI in Settings to continue this plan"
            }
            onClick={() => onOpenWorkflow?.(sessionId, plan.planId)}
          >
            <IconTerminal2 size={15} stroke={1.8} aria-hidden="true" />
            Advanced workflow
          </button>
        </div>
      </div>
    </article>
  );
}

export function RunTimeline({
  view,
  activityPresentation = "thread",
  activityComparison = false,
  planContinuationAvailable = false,
  planWorkflowAvailable = false,
  onOpenPlanWorkflow,
  onRevisePlan,
  onExecutePlan,
  onOpenResearchSources,
}: RunTimelineProps) {
  const hasTerminalFeedItem = view.updates.some(
    ({ update }) => update.type === "failure" || update.type === "cancellation",
  );
  const hasDurableUserMessage = view.updates.some(
    ({ update }) => update.type === "message" && update.message.role === "user",
  );
  const isGenerating =
    view.streamState === "watching" &&
    (view.run.status === "queued" || view.run.status === "running");
  const timelineItems = compactTimelineItems(view.updates);
  const userTimelineItems = timelineItems.filter(isUserMessageItem);
  const activityItems = timelineItems.filter(isVisibleActivityItem);
  const showLiveStatus = isGenerating && view.output === "";
  const partialResponse =
    view.output !== "" &&
    !isGenerating &&
    (view.run.status === "failed" ||
      view.run.status === "outcome_unknown" ||
      view.run.terminal?.type === "failure");
  const plan = terminalPlanReference(view.run.terminal);

  return (
    <div className="timeline">
      <p className="sr-only" role="status" aria-live="polite">
        {view.run.status === "waiting"
          ? "Colossus is waiting for your input."
          : isGenerating
            ? "Colossus is working."
            : `Run ${readable(view.run.status)}.`}
      </p>
      {view.localPrompt !== null && !hasDurableUserMessage ? (
        <article className="feed-entry message message-user">
          <div className="feed-marker" aria-hidden="true">
            <IconMessageCircle size={17} stroke={1.7} />
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <strong>You</strong>
            </header>
            <div
              className="message-body preserve-lines"
              data-aside-selectable="true"
            >
              {view.localPrompt}
            </div>
          </div>
        </article>
      ) : null}
      {userTimelineItems.map((item) =>
        item.type === "update" ? (
          <FeedItem item={item.update} key={item.update.sequence} />
        ) : null,
      )}
      {activityItems.length > 0 ? (
        <RunActivity
          view={view}
          items={activityItems}
          presentation={activityPresentation}
          comparison={activityComparison}
        />
      ) : null}
      {showLiveStatus ? <LiveRunStatus view={view} /> : null}
      {view.output !== "" ? (
        <article className="feed-entry message message-assistant">
          <div className="feed-marker assistant-marker" aria-hidden="true">
            <img src={colossusMark} alt="" />
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <h3 className="feed-entry-title">Colossus</h3>
              <span>
                {isGenerating
                  ? "Working"
                  : partialResponse
                    ? "Partial response"
                    : "Response"}
              </span>
            </header>
            <div
              className={`message-body${isGenerating ? " preserve-lines" : ""}`}
              data-aside-selectable="true"
            >
              {isGenerating ? (
                view.output
              ) : view.run.mode === "research" ? (
                <ResearchResponse
                  output={view.output}
                  onOpenSources={onOpenResearchSources}
                />
              ) : (
                <MarkdownContent content={view.output} />
              )}
              {isGenerating ? (
                <span className="stream-caret" aria-hidden="true" />
              ) : null}
            </div>
          </div>
        </article>
      ) : null}
      {!hasTerminalFeedItem && view.run.terminal !== null ? (
        <TerminalSummary terminal={view.run.terminal} />
      ) : null}
      {plan !== null ? (
        <PlanResultCard
          sourceRunId={view.run.runId}
          plan={plan}
          sessionId={view.run.sessionId}
          cancelled={view.run.terminal?.type === "cancellation"}
          continuationAvailable={planContinuationAvailable}
          workflowAvailable={planWorkflowAvailable}
          onOpenWorkflow={onOpenPlanWorkflow}
          onRevise={onRevisePlan}
          onExecute={onExecutePlan}
        />
      ) : null}
      {view.usage !== null ? (
        <p className="usage-summary">
          {view.usage.totalTokens.toLocaleString()} tokens ·{" "}
          {view.usage.inputTokens.toLocaleString()} in /{" "}
          {view.usage.outputTokens.toLocaleString()} out
        </p>
      ) : null}
    </div>
  );
}
