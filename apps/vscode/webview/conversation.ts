import type { MessageView, ToolView, WorkView } from "../src/model.js";
import { brandMarks, icon, node } from "./ui.js";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { ConversationEntry } from "@colossus/ui/conversation";
import { mcpCallTarget } from "@colossus/ui/lib/mcp-call";

function runId(message: MessageView) {
  return (
    message.runId ??
    /^(?:user|assistant|notice|tools):(.+)$/u.exec(message.id)?.[1] ??
    "legacy"
  );
}

function reconcile(parent: HTMLElement, children: HTMLElement[]) {
  const retained = new Set(children);
  for (const child of Array.from(parent.children)) {
    if (!retained.has(child as HTMLElement)) child.remove();
  }
  // Keep existing details/summary nodes in place so expansion and keyboard focus
  // survive progress updates. Most streaming updates require no DOM moves.
  children.forEach((child, index) => {
    if (parent.children[index] !== child)
      parent.insertBefore(child, parent.children[index] ?? null);
  });
}

const labels: Record<string, string> = {
  requested: "Requested",
  "waiting approval": "Approval needed",
  started: "Running",
  completed: "Completed",
  failed: "Failed",
  "outcome unknown": "Outcome unknown",
  cancelled: "Cancelled",
  unspecified: "Status unavailable",
};
function stateLabel(state: string) {
  return labels[state] ?? "Status unavailable";
}

function toolIcon(name: string) {
  if (name.startsWith("mcp.")) return "plug-connected";
  if (name.startsWith("filesystem.")) return "folder";
  if (name.startsWith("plan.")) return "list-check";
  if (name.startsWith("user.")) return "messages";
  if (name.startsWith("http.") || name.startsWith("web.")) return "world";
  return "terminal-2";
}

function elapsed(start?: string, end?: string) {
  const ms = Date.parse(end ?? "") - Date.parse(start ?? "");
  if (!Number.isFinite(ms) || ms < 1000) return "";
  const seconds = Math.floor(ms / 1000);
  return seconds < 60
    ? `${seconds}s`
    : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

class ToolRow {
  readonly element = node("details", "", "tool-progress") as HTMLDetailsElement;
  private heading = node("summary", "", "tool-progress-heading");
  private symbol = icon("terminal-2");
  private name = node("span", "", "tool-progress-name");
  private state = node("span", "", "tool-progress-state");
  private description = node("span", "", "tool-progress-description");
  private body = node("div", "", "tool-progress-body");
  private signature = "";
  constructor() {
    this.heading.append(
      this.symbol,
      this.name,
      this.state,
      node("span", "", "tool-chevron"),
      this.description,
    );
    this.element.append(this.heading, this.body);
  }
  update(tool: ToolView) {
    const signature = JSON.stringify(tool);
    if (signature === this.signature) return;
    this.signature = signature;
    this.element.dataset.state = tool.state;
    const mcpTarget =
      tool.name === "mcp.call"
        ? (mcpCallTarget(tool.input) ?? mcpCallTarget(tool.preview))
        : null;
    this.name.textContent = mcpTarget
      ? `${tool.name} · ${mcpTarget}`
      : tool.name;
    this.symbol.className = `icon icon-${toolIcon(tool.name)}`;
    this.state.textContent = stateLabel(tool.state);
    this.description.textContent = tool.summary;
    this.description.hidden =
      !tool.summary || ["completed", "cancelled"].includes(tool.state);
    const history = node("ol", "", "tool-lifecycle");
    for (const step of tool.history ?? [
      { state: tool.state, summary: tool.summary, at: tool.updatedAt ?? "" },
    ]) {
      const entry = node("li");
      entry.append(
        node("strong", stateLabel(step.state)),
        node("span", step.summary, "plain-text"),
      );
      if (step.at) {
        const time = node(
          "time",
          new Date(step.at).toLocaleTimeString([], {
            hour: "numeric",
            minute: "2-digit",
            second: "2-digit",
          }),
        ) as HTMLTimeElement;
        time.dateTime = step.at;
        time.title = step.at;
        entry.append(time);
      }
      history.append(entry);
    }
    const content = [history];
    for (const [label, text] of [
      ["Released input", tool.input],
      ["Released output", tool.preview],
    ]) {
      if (text === undefined) continue;
      const section = node("section", "", "tool-released");
      section.append(node("h2", label), node("pre", text, "plain-text"));
      content.push(section);
    }
    if (tool.preview === undefined) {
      const message =
        tool.state === "outcome unknown"
          ? "The tool outcome is unknown; no successful output is confirmed."
          : tool.state === "started"
            ? "Waiting for released output."
            : tool.state === "waiting approval"
              ? "Waiting for approval before execution."
              : "No released output preview is available.";
      content.push(node("p", message, "muted"));
    }
    this.body.replaceChildren(...content);
  }
}

class ToolThread {
  readonly element = node("details", "", "tool-thread") as HTMLDetailsElement;
  private meta = node("span", "", "tool-thread-meta");
  private state = node("span", "", "tool-thread-state");
  private list = node("div", "", "tool-thread-list");
  private rows = new Map<string, ToolRow>();
  constructor(id: string) {
    this.element.open = true;
    this.element.dataset.runId = id;
    const heading = node("summary", "", "tool-thread-heading");
    const mark = node("img") as HTMLImageElement;
    mark.dataset.brandMark = "";
    mark.alt = "";
    mark.width = mark.height = 20;
    heading.append(
      node("span", "", "tool-chevron"),
      mark,
      node("strong", "Colossus"),
      this.meta,
      this.state,
    );
    this.element.append(heading, this.list);
    brandMarks(this.element);
  }
  update(
    tools: ToolView[],
    run: WorkView["runs"][number] | undefined,
    visible: boolean,
  ) {
    this.element.hidden = !visible;
    const duration = elapsed(
      run?.startedAt || run?.createdAt || tools[0]?.startedAt,
      run?.finishedAt ||
        tools
          .map((tool) => tool.updatedAt ?? "")
          .sort()
          .at(-1),
    );
    this.meta.textContent = `${tools.length} ${tools.length === 1 ? "action" : "actions"}${duration ? ` · ${duration}` : ""}`;
    const status = run?.status;
    const terminal = ["completed", "failed", "cancelled"].includes(
      status ?? "",
    );
    const state = terminal
      ? status!
      : tools.some((t) => t.state === "waiting approval")
        ? "waiting approval"
        : status === "queued"
          ? "requested"
          : "started";
    this.element.dataset.state = state;
    this.state.textContent = run ? stateLabel(state) : "Recorded progress";
    const retained = new Set(tools.map((t) => t.id));
    for (const id of this.rows.keys())
      if (!retained.has(id)) this.rows.delete(id);
    reconcile(
      this.list,
      tools.map((tool) => {
        let row = this.rows.get(tool.id);
        if (!row) {
          row = new ToolRow();
          this.rows.set(tool.id, row);
        }
        row.update(tool);
        return row.element;
      }),
    );
  }
}

export class Conversation {
  private messages = new Map<
    string,
    { signature: string; element: HTMLElement; renderer: Root }
  >();
  private threads = new Map<string, ToolThread>();
  constructor(
    private root: HTMLElement,
    private scroll: HTMLElement,
  ) {}
  render(view: WorkView, showTools: boolean, changingSession: boolean) {
    const atBottom =
      this.scroll.scrollTop + this.scroll.clientHeight >=
      this.scroll.scrollHeight - 100;
    const lastRun = [...view.messages].reverse().find((m) => m.role === "user");
    const toolsByRun = new Map<string, ToolView[]>();
    for (const tool of view.tools) {
      const id = tool.runId ?? (lastRun ? runId(lastRun) : "legacy");
      toolsByRun.set(id, [...(toolsByRun.get(id) ?? []), tool]);
    }
    const children: HTMLElement[] = [];
    const rendered = new Set<string>();
    const addThread = (id: string) => {
      const tools = toolsByRun.get(id);
      if (!tools || rendered.has(id)) return;
      let thread = this.threads.get(id);
      if (!thread) {
        thread = new ToolThread(id);
        this.threads.set(id, thread);
      }
      thread.update(
        tools,
        view.runs.find((run) => run.id === id),
        showTools,
      );
      children.push(thread.element);
      rendered.add(id);
    };
    for (const message of view.messages) {
      const id = runId(message);
      if (message.role !== "user") addThread(id);
      const signature = JSON.stringify(message);
      let cached = this.messages.get(message.id);
      if (!cached) {
        const element = node(
          "div",
          "",
          `message ${message.role} shared-message-island`,
        );
        cached = { signature: "", element, renderer: createRoot(element) };
        this.messages.set(message.id, cached);
      }
      if (cached.signature !== signature) {
        cached.signature = signature;
        const label =
          message.role === "user"
            ? message.summary
              ? "Task summary"
              : "You"
            : message.role === "assistant"
              ? "Colossus"
              : "Notice";
        const role =
          message.role === "user"
            ? "user"
            : message.role === "assistant"
              ? "assistant"
              : "system";
        flushSync(() =>
          cached!.renderer.render(
            createElement(ConversationEntry, {
              role,
              author: label,
              content: message.text,
            }),
          ),
        );
      }
      children.push(cached.element);
      if (message.role === "user") addThread(id);
    }
    for (const id of toolsByRun.keys()) addThread(id);
    reconcile(this.root, children);
    const retained = new Set(view.messages.map((m) => m.id));
    for (const id of this.messages.keys())
      if (!retained.has(id)) {
        this.messages.get(id)?.renderer.unmount();
        this.messages.delete(id);
      }
    for (const id of this.threads.keys())
      if (!rendered.has(id)) this.threads.delete(id);
    if (changingSession || atBottom)
      this.scroll.scrollTop = this.scroll.scrollHeight;
  }
}
