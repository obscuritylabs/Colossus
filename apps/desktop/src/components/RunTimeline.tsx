import {
  IconAlertTriangle,
  IconArrowRight,
  IconBrain,
  IconCheck,
  IconChevronDown,
  IconCircle,
  IconFile,
  IconInfoCircle,
  IconLoader2,
  IconMessageCircle,
  IconPlayerStop,
  IconTerminal2,
} from "@tabler/icons-react";
import type { ReactNode } from "react";

import colossusMark from "../assets/colossus-mark.svg";
import type { RunView } from "../state";
import type {
  MessageContentPart,
  RunFailure,
  RunTerminal,
  RunUpdate,
  SessionMessage,
} from "../types";
import { MarkdownContent } from "./MarkdownContent";

interface RunTimelineProps {
  view: RunView;
}

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
      : ["Retry after", `${failure.retryAfterMs} ms`],
    ["Recoverable", failure.recoverable === true ? "Yes" : "No"],
  ].filter((value): value is string[] => value !== null);
  return (
    <dl className="failure-metadata">
      {values.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
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
        <div className="message-body">
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

function ToolActivityItem({ group }: { group: ToolActivityGroup }) {
  const latest = group.updates.at(-1);
  if (latest === undefined || latest.update.type !== "tool_activity") {
    return null;
  }
  const activity = latest.update.activity;
  const complete = activity.state === "completed";
  return (
    <details className="compact-tool-activity">
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
            <strong>{activity.toolName}</strong>
            <span className={`event-state tool-state-${activity.state}`}>
              {readable(activity.state)}
            </span>
            <time dateTime={latest.createdAt}>
              {compactTime(latest.createdAt)}
            </time>
          </span>
          <small>{activity.summary}</small>
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
              <strong>Run failed</strong>
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
          <strong>Run failed</strong>
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

export function RunTimeline({ view }: RunTimelineProps) {
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
  const showLiveStatus = isGenerating && view.output === "";
  const partialResponse =
    view.output !== "" &&
    !isGenerating &&
    (view.run.status === "failed" ||
      view.run.status === "outcome_unknown" ||
      view.run.terminal?.type === "failure");

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
            <div className="message-body preserve-lines">
              {view.localPrompt}
            </div>
          </div>
        </article>
      ) : null}
      {timelineItems.map((item) =>
        item.type === "tool_activity" ? (
          <ToolActivityItem group={item.group} key={`tool-${item.group.key}`} />
        ) : (
          <FeedItem item={item.update} key={item.update.sequence} />
        ),
      )}
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
            >
              {isGenerating ? (
                view.output
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
