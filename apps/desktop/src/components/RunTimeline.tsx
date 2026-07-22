import {
  IconAlertTriangle,
  IconArrowRight,
  IconBrain,
  IconCheck,
  IconCircle,
  IconFile,
  IconInfoCircle,
  IconMessageCircle,
  IconPlayerStop,
  IconTerminal2,
} from "@tabler/icons-react";
import type { ReactNode } from "react";

import colossusMark from "../assets/colossus-mark.svg";
import type { RunView } from "../state";
import type {
  MessageContentPart,
  RunTerminal,
  RunUpdate,
  SessionMessage,
} from "../types";
import { MarkdownContent } from "./MarkdownContent";

interface RunTimelineProps {
  view: RunView;
}

function readable(value: string): string {
  return value.replaceAll("_", " ");
}

function compactTime(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime())
    ? "Recent"
    : date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
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

function FeedItem({ item }: { item: RunUpdate }): ReactNode {
  const update = item.update;
  switch (update.type) {
    case "message":
      return <Message message={update.message} />;
    case "reasoning_summary":
      return <ReasoningSummary item={item} />;
    case "tool_activity": {
      const complete = update.activity.state === "completed";
      return (
        <article className="feed-entry process-row">
          <div className="feed-marker" aria-hidden="true">
            {complete ? (
              <IconCheck size={17} stroke={2} />
            ) : (
              <IconTerminal2 size={17} stroke={1.7} />
            )}
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <strong>{update.activity.toolName}</strong>
              <time dateTime={item.createdAt}>
                {compactTime(item.createdAt)}
              </time>
            </header>
            <p>{update.activity.summary}</p>
            <span className={`event-state tool-state-${update.activity.state}`}>
              {readable(update.activity.state)}
            </span>
          </div>
        </article>
      );
    }
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

  return (
    <div className="timeline" id="work-activity">
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
      {view.updates.map((item) => (
        <FeedItem item={item} key={item.sequence} />
      ))}
      {view.output !== "" || isGenerating ? (
        <article className="feed-entry message message-assistant">
          <div className="feed-marker assistant-marker" aria-hidden="true">
            <img src={colossusMark} alt="" />
          </div>
          <div className="feed-entry-content">
            <header className="feed-entry-heading">
              <h3 className="feed-entry-title">Colossus</h3>
              <span>{isGenerating ? "Working" : "Response"}</span>
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
