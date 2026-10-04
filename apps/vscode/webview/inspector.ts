import type { InspectionView } from "../src/model.js";
import { brandMarks, element, node } from "./ui.js";
import { markdown } from "./markdown.js";
declare function acquireVsCodeApi(): {
  postMessage(message: { type: "ready" | "refreshInspection" }): void;
};
const api = acquireVsCodeApi();
const app = element("app");
app.innerHTML = `<header class="inspection-header"><div class="work-heading"><img data-brand-mark width="24" height="24" alt=""><h1 id="title">State inspector</h1></div><button id="refresh" class="secondary"><span class="icon icon-refresh" aria-hidden="true"></span>Refresh state</button></header>
<nav class="inspection-nav workspace-nav" aria-label="Inspection views"><button data-tab="overview" aria-pressed="true">Overview</button><button data-tab="output" aria-pressed="false">Output</button><button data-tab="activity" aria-pressed="false">Session activity</button></nav><main id="inspection-content" class="inspection-content" aria-label="State details"></main>`;
brandMarks(app);
let tab = "overview";
let view: InspectionView | undefined;
let loading = false;
let error = "";
let connected = false;
element("refresh").addEventListener("click", () =>
  api.postMessage({ type: "refreshInspection" }),
);
for (const button of app.querySelectorAll<HTMLButtonElement>("[data-tab]"))
  button.addEventListener("click", () => {
    tab = button.dataset.tab!;
    render();
  });
function fields(rows: [string, string][]) {
  const list = node("dl", "", "inspection-fields");
  for (const [key, value] of rows) {
    list.append(node("dt", key), node("dd", value || "—"));
  }
  return list;
}
function render() {
  element<HTMLButtonElement>("refresh").disabled = !connected || loading;
  for (const button of app.querySelectorAll<HTMLButtonElement>("[data-tab]"))
    button.setAttribute("aria-pressed", String(button.dataset.tab === tab));
  const content = element("inspection-content");
  content.replaceChildren();
  if (!view) {
    element("title").textContent = "State inspector";
    const notice = node(
      "p",
      loading
        ? "Reading saved state…"
        : error || "Select a run or plan from Colossus Workspace.",
      "muted",
    );
    notice.setAttribute("role", "status");
    content.append(notice);
    return;
  }
  const run = view.run;
  element("title").textContent = view.plan
    ? `Plan · ${view.plan.title}`
    : run.title;
  const status = node("div", "", "inspection-status");
  status.append(
    node("span", run.status, "state-badge"),
    node("span", `${run.mode} · ${run.role}`, "muted"),
    node(
      "span",
      `Read ${new Date(view.observedAt).toLocaleTimeString()}`,
      "muted",
    ),
  );
  content.append(status);
  if (tab === "overview") {
    content.append(
      node("h2", "Run state"),
      fields([
        ["Run", run.id],
        ["Session", run.sessionId],
        ["Created", run.createdAt],
        ["Updated", run.updatedAt],
        ["Started", run.startedAt],
        ["Finished", run.finishedAt],
        ["Feed sequence", run.sequence],
        ["Pending interactions", String(run.pendingInteractions)],
        ["Model", view.model],
        ["Provider", view.provider],
      ]),
    );
    if (view.plan)
      content.append(
        node("h2", "Plan state"),
        fields([
          ["Plan", view.plan.id],
          ["Revision", view.plan.revision],
          ["Status", view.plan.status],
          ["Goal", view.plan.goalId],
        ]),
      );
    content.append(
      node(
        "p",
        "This is a saved snapshot. Refresh to read the latest worker state. Respond to pending approvals and questions in chat.",
        "scope-note",
      ),
    );
  } else if (tab === "output") {
    content.append(
      node("h2", view.plan ? "Saved plan / run output" : "Released output"),
    );
    content.append(
      view.output
        ? markdown(view.output)
        : node("p", "No terminal output has been saved for this run.", "muted"),
    );
  } else {
    content.append(
      node("h2", "Session activity"),
      node("p", view.activityState, "muted"),
    );
    for (const activity of view.activities) {
      const card = node("article", "", "activity-card");
      card.append(
        node("h3", activity.title),
        node(
          "p",
          `${activity.kind} · ${activity.lane}${activity.status ? ` · ${activity.status}` : ""}`,
          "resource-meta",
        ),
        node("p", activity.summary, "plain-text"),
      );
      if (activity.startedAt)
        card.append(
          node("p", new Date(activity.startedAt).toLocaleString(), "muted"),
        );
      if (activity.result) {
        const details = node("details");
        details.append(
          node("summary", "Released result"),
          node("pre", activity.result, "released-result"),
        );
        card.append(details);
      }
      content.append(card);
    }
    if (view.activityHasMore)
      content.append(
        node(
          "p",
          "Showing the latest 25 activities. Older activity is available in the desktop app.",
          "scope-note",
        ),
      );
    if (!view.activities.length)
      content.append(node("p", "No activity to display.", "muted"));
  }
}
window.addEventListener(
  "message",
  (
    event: MessageEvent<{
      type: string;
      view?: InspectionView;
      loading: boolean;
      connected: boolean;
      error: string;
    }>,
  ) => {
    if (event.data.type !== "inspection") return;
    view = event.data.view;
    loading = event.data.loading;
    connected = event.data.connected;
    error = event.data.error;
    render();
  },
);
api.postMessage({ type: "ready" });
