import type { ViewAction, WorkView } from "../src/model.js";
import { brandMarks, element, icon, node } from "./ui.js";
declare function acquireVsCodeApi(): {
  postMessage(message: ViewAction): void;
  getState(): { tab?: string; query?: string } | undefined;
  setState(state: { tab: string; query: string }): void;
};
const api = acquireVsCodeApi();
const app = element("app");
app.innerHTML = `<header class="workspace-header"><div class="work-heading"><img data-brand-mark width="24" height="24" alt=""><h1>Workspace</h1></div><button id="settings" class="icon-button" aria-label="Open Colossus settings" title="Settings"><span class="icon icon-settings" aria-hidden="true"></span></button></header>
<div class="workspace-toolbar"><button id="new" class="secondary"><span class="icon icon-plus" aria-hidden="true"></span>New conversation</button><button id="refresh" class="icon-button" aria-label="Refresh workspace" title="Refresh workspace"><span class="icon icon-refresh" aria-hidden="true"></span></button></div>
<nav class="workspace-nav" aria-label="Workspace views"><button data-tab="sessions" aria-pressed="true">Sessions</button><button data-tab="plans" aria-pressed="false">Plans</button><button data-tab="runtime" aria-pressed="false">Runtime</button></nav>
<div class="workspace-search" role="search" aria-label="Search workspace"><label class="sr-only" for="search">Search workspace data</label><input id="search" type="search" placeholder="Search sessions and plans…"></div>
<main id="workspace-content" class="workspace-content" aria-label="Workspace data"></main>
<footer class="workspace-footer"><span id="connection" role="status">Disconnected</span><button id="chat" class="text-button">Open chat</button></footer>`;
brandMarks(app);
const saved = api.getState();
let tab = ["sessions", "plans", "runtime"].includes(saved?.tab ?? "")
  ? saved!.tab!
  : "sessions";
const search = element<HTMLInputElement>("search");
search.value = saved?.query ?? "";
let view: WorkView | undefined;
let signature = "";
const post = (action: ViewAction) => api.postMessage(action);
const actionButton = (
  text: string,
  action: ViewAction,
  disabled = false,
  className = "",
) => {
  const button = node("button", text, className) as HTMLButtonElement;
  button.disabled = disabled;
  button.addEventListener("click", () => post(action));
  return button;
};
for (const [id, type] of [
  ["new", "newSession"],
  ["refresh", "refreshSessions"],
  ["settings", "openSettings"],
  ["chat", "openWork"],
] as const)
  element(id).addEventListener("click", () => post({ type }));
for (const button of app.querySelectorAll<HTMLButtonElement>("[data-tab]"))
  button.addEventListener("click", () => {
    tab = button.dataset.tab!;
    save();
    render();
  });
function save() {
  api.setState({ tab, query: search.value });
}
search.addEventListener("input", () => {
  save();
  render();
});
function render() {
  if (!view) return;
  const next = view;
  element("connection").textContent = next.connecting
    ? "Connecting…"
    : next.connected
      ? `Worker ${next.version}`
      : "Disconnected";
  element<HTMLButtonElement>("new").disabled = !next.connected || next.busy;
  element<HTMLButtonElement>("refresh").disabled =
    !next.connected || next.historyLoading;
  for (const button of app.querySelectorAll<HTMLButtonElement>("[data-tab]"))
    button.setAttribute("aria-pressed", String(button.dataset.tab === tab));
  search.placeholder = `Search ${tab}…`;
  const query = search.value.trim().toLowerCase();
  const matches = (...values: string[]) =>
    values.join(" ").toLowerCase().includes(query);
  const nextSignature = JSON.stringify([
    tab,
    query,
    next.connected,
    next.busy,
    next.sessionId,
    next.sessions,
    next.plans,
    next.capabilities,
    next.historyHasMore,
    next.historyLoading,
    next.error,
    next.runs.map(
      ({ id, title, status, mode, sessionId, pendingInteractions }) => ({
        id,
        title,
        status,
        mode,
        sessionId,
        pendingInteractions,
      }),
    ),
  ]);
  if (nextSignature === signature) return;
  signature = nextSignature;
  const content = element("workspace-content");
  content.replaceChildren();
  if (!next.connected) {
    content.append(
      node("h2", "Connect your workspace"),
      node(
        "p",
        "Connect the enrolled local worker to browse saved sessions, plans, and run states.",
        "muted",
      ),
      actionButton("Connect worker", { type: "connect" }, next.connecting),
    );
    if (next.error) content.append(node("p", next.error, "error"));
    return;
  }
  if (tab === "runtime") {
    content.append(
      node("h2", "Worker capabilities"),
      node(
        "p",
        "Availability reflects this worker and your enrollment.",
        "muted",
      ),
    );
    for (const capability of next.capabilities.filter((c) =>
      matches(c.name, c.detail),
    )) {
      const card = node("article", "", "state-card");
      card.append(
        node("strong", capability.name),
        node(
          "span",
          capability.enabled ? "Available" : "Unavailable",
          `state-badge ${capability.enabled ? "good" : ""}`,
        ),
        node("p", capability.detail, "muted"),
      );
      content.append(card);
    }
    content.append(
      node(
        "p",
        "Goals, memory, workflows, and the full session topology currently require the desktop app. Their public inspection APIs are not available here yet.",
        "scope-note",
      ),
    );
    return;
  }
  if (tab === "plans") {
    content.append(
      node("h2", "Saved plans"),
      node("p", "Canonical revisions from the loaded run history.", "muted"),
    );
    const plans = next.plans.filter((p) => matches(p.title, p.status, p.id));
    for (const plan of plans) {
      const button = actionButton(
        "",
        { type: "inspectPlan", id: plan.id },
        false,
        "resource-row",
      );
      button.append(
        icon("list-check"),
        node("strong", plan.title),
        node(
          "span",
          `Revision ${plan.revision} · ${plan.status}`,
          "resource-meta",
        ),
      );
      content.append(button);
    }
    if (!plans.length)
      content.append(
        node(
          "p",
          query
            ? "No matching plans."
            : "Plans appear after a Plan run saves a canonical plan.",
          "empty-resource",
        ),
      );
  } else {
    content.append(node("h2", "Sessions"));
    const sessions = next.sessions.filter((s) =>
      matches(s.title, s.status ?? "", s.id),
    );
    for (const session of sessions) {
      const row = node("div", "", "session-resource");
      const select = actionButton(
        "",
        { type: "selectSession", id: session.id },
        next.busy,
        "resource-row",
      );
      select.setAttribute(
        "aria-current",
        session.id === next.sessionId ? "true" : "false",
      );
      select.append(
        icon("messages"),
        node("strong", session.title),
        node("span", session.status ?? "Saved", "resource-meta"),
      );
      const run = next.runs.find((r) => r.sessionId === session.id);
      row.append(select);
      if (run) {
        const inspect = actionButton(
          "",
          { type: "inspectRun", id: run.id },
          false,
          "icon-button",
        );
        inspect.setAttribute("aria-label", `Inspect session: ${session.title}`);
        inspect.title = "Inspect session state";
        inspect.append(icon("layout-sidebar"));
        row.append(inspect);
      }
      content.append(row);
    }
    if (!sessions.length)
      content.append(
        node(
          "p",
          query
            ? "No matching sessions."
            : "Start a conversation to create a saved session.",
          "empty-resource",
        ),
      );
    const runs = next.runs.filter(
      (r) =>
        (!next.sessionId || r.sessionId === next.sessionId) &&
        matches(r.title, r.status, r.mode),
    );
    content.append(
      node(
        "h2",
        next.sessionId ? "Session runs" : "Recent runs",
        "section-heading",
      ),
    );
    for (const run of runs.slice(0, 20)) {
      const button = actionButton(
        "",
        { type: "inspectRun", id: run.id },
        false,
        "resource-row",
      );
      button.append(
        icon("activity"),
        node("strong", run.title),
        node(
          "span",
          `${run.mode} · ${run.status}${run.pendingInteractions ? ` · ${run.pendingInteractions} pending` : ""}`,
          "resource-meta",
        ),
      );
      content.append(button);
    }
    if (runs.length > 20)
      content.append(
        node("p", "Showing the 20 most recent loaded runs.", "muted"),
      );
  }
  if (next.historyHasMore)
    content.append(
      actionButton(
        next.historyLoading ? "Loading…" : "Load older history",
        { type: "loadMoreSessions" },
        next.historyLoading,
        "secondary load-more",
      ),
    );
  content.append(
    node(
      "p",
      next.error ||
        "Select a session to open it in chat. Inspect a run or plan to view its saved state.",
      "scope-note",
    ),
  );
}
window.addEventListener(
  "message",
  (event: MessageEvent<{ type: string; view?: WorkView }>) => {
    if (event.data.type !== "state" || !event.data.view) return;
    view = event.data.view;
    render();
  },
);
post({ type: "ready" });
