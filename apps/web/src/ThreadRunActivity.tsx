import {
  ConversationActivity,
  type ConversationActivityTone,
} from "@colossus/ui/conversation";
import { mcpCallTarget } from "@colossus/ui/lib/mcp-call";
import {
  IconChevronDown,
  IconPlug,
  IconSparkles,
  IconTool,
} from "@tabler/icons-react";
import {
  statusLabel,
  taskStatus,
  terminalStatuses,
  type Task,
  type Update,
} from "./api";

type ReleasedTool = {
  call_id: string;
  tool_name: string;
  state: string;
  summary: string;
  input?: string;
  preview?: string;
};
type ActivityItem =
  | {
      key: string;
      type: "tool";
      history: { activity: ReleasedTool; update: Update }[];
    }
  | {
      key: string;
      type: "note";
      text: string;
      createdAt: string;
      reasoning: boolean;
    };

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
function toolsAndNotes(updates: Update[]): ActivityItem[] {
  const items: ActivityItem[] = [],
    tools = new Map<string, Extract<ActivityItem, { type: "tool" }>>();
  for (const update of updates) {
    const value = update.update,
      raw = record(value.tool_activity);
    if (
      raw &&
      typeof raw.tool_name === "string" &&
      typeof raw.state === "string"
    ) {
      const activity: ReleasedTool = {
        call_id: typeof raw.call_id === "string" ? raw.call_id : "",
        tool_name: raw.tool_name,
        state: raw.state,
        summary: typeof raw.summary === "string" ? raw.summary : "",
        ...(typeof raw.input === "string" ? { input: raw.input } : {}),
        ...(typeof raw.preview === "string" ? { preview: raw.preview } : {}),
      };
      const key = `${update.run_id}:${activity.call_id || `sequence-${update.sequence}`}`;
      let item = tools.get(key);
      if (!item) {
        item = { key, type: "tool", history: [] };
        tools.set(key, item);
        items.push(item);
      }
      item.history.push({ activity, update });
      continue;
    }
    let text: string | null =
      typeof value.reasoning_summary === "string"
        ? value.reasoning_summary
        : null;
    const notice = record(value.notice);
    if (
      notice &&
      typeof notice.message === "string" &&
      !(
        typeof notice.reason === "string" &&
        (notice.reason.startsWith("run.phase.") ||
          notice.reason === "model.final_output")
      )
    )
      text = notice.message;
    const failed = record(value.failure),
      failure = record(failed?.failure);
    if (typeof failure?.message === "string") text = failure.message;
    const cancellation = record(value.cancellation);
    if (typeof cancellation?.message === "string") text = cancellation.message;
    if (text)
      items.push({
        key: `${update.run_id}:${update.sequence}`,
        type: "note",
        text,
        createdAt: update.created_at,
        reasoning: typeof value.reasoning_summary === "string",
      });
  }
  return items;
}
function time(value: string) {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
    : "";
}
function preview(activity: ReleasedTool) {
  if (activity.preview) return activity.preview;
  if (activity.state === "completed")
    return "The tool completed, but this activity feed does not include an output preview.";
  if (activity.state === "cancelled")
    return "The tool was stopped before an output preview was released.";
  if (activity.state === "failed" || activity.state === "outcome_unknown")
    return activity.summary;
  return "A released output preview will appear after the tool completes.";
}

export function ThreadRunActivity({
  task,
  updates,
  label,
}: {
  task: Task;
  updates: Update[];
  label: string;
}) {
  const items = toolsAndNotes(updates),
    status = taskStatus(task);
  const terminal = record(task.snapshot?.run.terminal),
    result = record(terminal?.result),
    failure = record(terminal?.failure);
  if (!items.length && typeof failure?.message === "string")
    items.push({
      key: "snapshot-failure",
      type: "note",
      text: failure.message,
      createdAt: task.snapshot?.run.updated_at ?? "",
      reasoning: false,
    });
  if (!items.length) return null;
  const toolItems = items.filter(
      (item): item is Extract<ActivityItem, { type: "tool" }> =>
        item.type === "tool",
    ),
    actions = toolItems.length,
    notes = items.filter(
      (item) => item.type === "note" && item.reasoning,
    ).length,
    exceptions = toolItems.filter((item) =>
      ["failed", "outcome_unknown"].includes(
        item.history.at(-1)?.activity.state ?? "",
      ),
    ).length,
    duration =
      typeof result?.elapsed_seconds === "number" &&
      Number.isFinite(result.elapsed_seconds)
        ? `${Math.max(0, Math.round(result.elapsed_seconds))}s`
        : null,
    tone: ConversationActivityTone =
      status === "completed"
        ? "success"
        : ["failed", "outcome_unknown"].includes(status)
          ? "danger"
          : ["waiting", "cancelling"].includes(status)
            ? "warning"
            : terminalStatuses.has(status)
              ? "neutral"
              : "active",
    humanStatus =
      status === "waiting"
        ? "Needs input"
        : ["queued", "running"].includes(status)
          ? "Working"
          : status === "cancelling"
            ? "Stopping"
            : ["cancelled", "interrupted"].includes(status)
              ? "Stopped"
              : statusLabel(status);
  return (
    <ConversationActivity
      ariaLabel={label}
      className={`run-state-${status}`}
      description={[
        `${actions} ${actions === 1 ? "action" : "actions"}`,
        notes ? `${notes} ${notes === 1 ? "note" : "notes"}` : null,
        duration,
      ]
        .filter(Boolean)
        .join(" · ")}
      statusLabel={humanStatus}
      tone={tone}
      exceptionCount={exceptions}
      open={
        !terminalStatuses.has(status) || exceptions > 0 || tone === "danger"
      }
    >
      {items.map((item) => {
        if (item.type === "note")
          return (
            <div className="shared-activity-item" key={item.key}>
              {item.reasoning ? (
                <IconSparkles size={16} aria-hidden="true" />
              ) : null}
              <p className="shared-preserve-lines">{item.text}</p>
            </div>
          );
        const latest = item.history.at(-1)!,
          activity = latest.activity,
          input = [...item.history]
            .reverse()
            .find((entry) => entry.activity.input)?.activity.input,
          isMcpCall = activity.tool_name === "mcp.call",
          mcpTarget = isMcpCall
            ? (mcpCallTarget(input) ?? mcpCallTarget(activity.preview))
            : null;
        return (
          <details
            className="shared-activity-item shared-activity-tool"
            data-state={activity.state}
            key={item.key}
          >
            <summary>
              {isMcpCall ? (
                <IconPlug size={16} aria-hidden="true" />
              ) : (
                <IconTool size={16} aria-hidden="true" />
              )}
              <strong>
                {mcpTarget
                  ? `${activity.tool_name} · ${mcpTarget}`
                  : activity.tool_name}
              </strong>
              <span className="shared-activity-meta">
                {statusLabel(activity.state)}
              </span>
              <time
                className="shared-activity-meta"
                dateTime={latest.update.created_at}
              >
                {time(latest.update.created_at)}
              </time>
              <IconChevronDown size={15} aria-hidden="true" />
            </summary>
            <ol className="shared-activity-history">
              {item.history.map(({ activity, update }) => (
                <li key={`${update.run_id}:${update.sequence}`}>
                  <span className="shared-activity-meta">
                    {statusLabel(activity.state)}
                  </span>
                  <span>{activity.summary}</span>
                </li>
              ))}
            </ol>
            {input ? (
              <section
                className="shared-activity-detail"
                aria-label="Tool input"
              >
                <strong>Input</strong>
                <pre>{input}</pre>
              </section>
            ) : null}
            <section
              className="shared-activity-detail"
              aria-label="Tool output preview"
            >
              <strong>Preview</strong>
              <pre>{preview(activity)}</pre>
            </section>
          </details>
        );
      })}
    </ConversationActivity>
  );
}
