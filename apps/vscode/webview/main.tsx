import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import {
  ComposerInput,
  ComposerModeSwitch,
  ComposerSendButton,
  isComposerSendKey,
} from "@colossus/ui";
import type { ViewAction, WorkView } from "../src/model.js";
import { DEFAULT_PREFERENCES, type Preferences } from "../src/settings.js";
import { brandMarks, element, icon, node } from "./ui.js";
import { Conversation } from "./conversation.js";

declare function acquireVsCodeApi(): {
  postMessage(message: ViewAction): void;
  getState(): { draft?: string; mode?: "plan" | "execute" } | undefined;
  setState(state: { draft: string; mode: "plan" | "execute" }): void;
};
const api = acquireVsCodeApi();
const app = element<HTMLDivElement>("app");
app.innerHTML = `<header class="work-header"><div class="work-heading"><img data-brand-mark alt="" width="22" height="22"><h1 id="title">New work</h1></div><div class="header-actions">
<button id="history" class="icon-button" aria-label="Open workspace data" title="Sessions and plans"><span class="icon icon-history" aria-hidden="true"></span></button>
<button id="new" class="icon-button" aria-label="New conversation" title="New conversation"><span class="icon icon-plus" aria-hidden="true"></span></button>
<button id="settings" class="icon-button" aria-label="Open Colossus settings" title="Settings"><span class="icon icon-settings" aria-hidden="true"></span></button></div></header>
<main id="conversation" aria-label="Colossus work"><section id="empty" class="empty"><img data-brand-mark alt="" width="48" height="48"><h2>What would you like to work on?</h2><p>Plan a change, inspect code, or build something with Colossus.</p><button id="connect">Connect worker</button><p id="connect-hint" class="muted">Connect your enrolled local worker to get started.</p></section>
<section id="messages" role="log" aria-label="Conversation messages"></section></main>
<footer class="work-dock"><section id="interactions" aria-label="Pending interactions"></section><div id="error" class="error" role="alert" hidden></div>
<div class="composer"><div id="context" class="context" hidden></div><div id="composer-fields"></div></div>
<div id="context-actions" class="context-actions" hidden><button id="selection" class="text-button">Add selection</button><button id="file" class="text-button">Add file</button><button id="changes" class="text-button">Review changes</button></div>
<div class="workspace-row"><button id="workspace" class="workspace-button" title="Worker connection settings"><span class="icon icon-folder" aria-hidden="true"></span><span id="workspace-name">Select a workspace</span></button><span id="connection-status" class="muted">Disconnected</span></div><div class="status-row"><span id="status" role="status">Connect a Colossus worker to start.</span><button id="resume" class="text-button" hidden>Reconnect feed</button></div></footer>`;
brandMarks(app);
const composerRoot = createRoot(element("composer-fields"));
const persisted = api.getState();
let draft = persisted?.draft ?? "";
const mode = { value: persisted?.mode ?? "plan" };
let preferences = DEFAULT_PREFERENCES;
let initializedPreferences = false;
let view: WorkView | undefined;
let submittedText: string | undefined;
let submittedAfter = new Set<string>();
let contextOpen = false;
function saveDraft() {
  api.setState({ draft, mode: mode.value });
}
function renderComposer() {
  flushSync(() =>
    composerRoot.render(
      <>
        <ComposerInput
          id="prompt"
          aria-label="Task for Colossus"
          value={draft}
          disabled={!view?.connected}
          placeholder={
            view?.busy
              ? "Draft your next task…"
              : mode.value === "plan"
                ? "Describe the work you want Colossus to plan…"
                : "Ask Colossus to work on something…"
          }
          onChange={(event) => {
            draft = event.target.value;
            saveDraft();
            renderComposer();
          }}
          onKeyDown={(event) => {
            if (
              isComposerSendKey(
                { ...event, isComposing: event.nativeEvent.isComposing },
                preferences.sendShortcut,
              )
            ) {
              event.preventDefault();
              send();
            }
          }}
        />
        <div className="composer-actions">
          <div className="composer-controls">
            <button
              id="attach"
              className="icon-button"
              title="Add context"
              aria-label="Add context"
              aria-expanded={contextOpen}
              aria-controls="context-actions"
              disabled={!view?.connected || view.busy}
              onClick={() => {
                contextOpen = !contextOpen;
                element("context-actions").hidden = !contextOpen;
                renderComposer();
              }}
            >
              <span className="icon icon-paperclip" aria-hidden="true" />
            </button>
            <ComposerModeSwitch
              value={mode.value}
              disabled={!view?.connected || view.busy}
              options={[
                { value: "plan", label: "Plan" },
                { value: "execute", label: "Execute" },
              ]}
              onChange={(value) => {
                mode.value = value;
                saveDraft();
                renderComposer();
              }}
            />
          </div>
          <ComposerSendButton
            id="send"
            aria-label="Send"
            title={
              preferences.sendShortcut === "enter"
                ? "Send (Enter)"
                : "Send (Ctrl/Cmd+Enter)"
            }
            hidden={!!view?.busy}
            disabled={!view?.connected || !!view.busy || !draft.trim()}
            onClick={send}
          />
          <ComposerSendButton
            id="stop"
            className="is-stop"
            aria-label="Stop"
            title="Stop"
            hidden={!view?.busy}
            disabled={!view?.connected}
            onClick={() => post({ type: "stop" })}
          >
            <span className="icon icon-square" aria-hidden="true" />
          </ComposerSendButton>
        </div>
      </>,
    ),
  );
}
renderComposer();
function post(action: ViewAction) {
  api.postMessage(action);
}
function button(id: string, type: ViewAction["type"]) {
  element(id).addEventListener("click", () => post({ type } as ViewAction));
}
button("new", "newSession");
button("resume", "resume");
button("selection", "addSelection");
button("file", "addFile");
button("changes", "reviewChanges");
button("settings", "openSettings");
button("workspace", "openSettings");
button("connect", "connect");
button("history", "openWorkspace");
function send() {
  if (!view?.connected || view.busy || !draft.trim()) return;
  submittedText = draft;
  submittedAfter = new Set(
    view.messages.filter((m) => m.role === "user").map((m) => m.id),
  );
  post({
    type: "send",
    text: submittedText,
    mode: mode.value as "plan" | "execute",
  });
}
const conversation = new Conversation(
  element("messages"),
  element("conversation"),
);
let interactionSignature = "";
function render(next: WorkView) {
  const changingSession = view?.sessionId !== next.sessionId;
  if (view?.sessionId && !next.sessionId && !next.busy) {
    mode.value = preferences.defaultMode;
    saveDraft();
  }
  if (next.busy) mode.value = next.mode;
  view = next;
  if (
    submittedText &&
    next.messages.some(
      (m) =>
        m.role === "user" &&
        !submittedAfter.has(m.id) &&
        m.text.startsWith(submittedText!.trim()),
    )
  ) {
    if (draft === submittedText) draft = "";
    submittedText = undefined;
    mode.value = next.mode;
    saveDraft();
  }
  element("title").textContent =
    next.sessions.find((s) => s.id === next.sessionId)?.title ?? "New work";
  element("workspace-name").textContent = next.workspace;
  element("connection-status").textContent = next.connecting
    ? "Connecting…"
    : next.connected
      ? "Agent online"
      : "Disconnected";
  element("status").textContent = next.status;
  element("error").textContent = next.error;
  element("error").hidden = !next.error;
  element("empty").hidden = next.messages.length > 0;
  element("connect").hidden = next.connected;
  element("connect-hint").hidden = next.connected;
  element<HTMLButtonElement>("connect").disabled = next.connecting;
  element("resume").hidden = !next.busy || next.watching;
  for (const id of ["new", "selection", "file"])
    element<HTMLButtonElement>(id).disabled = !next.connected || next.busy;
  renderComposer();
  element<HTMLButtonElement>("changes").disabled = !next.connected;
  conversation.render(next, preferences.showToolActivity, changingSession);
  const interactions = JSON.stringify(next.interactions);
  if (interactions !== interactionSignature) {
    interactionSignature = interactions;
    element("interactions").replaceChildren(
      ...next.interactions.map((interaction) => {
        const card = node("article", "", "interaction");
        card.append(
          node(
            "strong",
            interaction.kind === "approval" ? "Approval required" : "Question",
          ),
          node("p", interaction.title),
        );
        const response = node(
          "button",
          interaction.respondable
            ? "Review and respond"
            : "Waiting for an authorized responder",
        ) as HTMLButtonElement;
        response.disabled = !interaction.respondable;
        response.addEventListener("click", () =>
          post({ type: "respond", id: interaction.id }),
        );
        card.append(response);
        return card;
      }),
    );
  }
  const context = element("context");
  context.hidden = !next.context.length;
  context.replaceChildren(
    ...next.context.map((c) => node("span", c.label, "context-chip")),
  );
  if (next.context.length) {
    const clear = node("button", "Clear", "text-button") as HTMLButtonElement;
    clear.disabled = next.busy;
    clear.addEventListener("click", () => post({ type: "clearContext" }));
    context.append(clear);
  }
}
window.addEventListener(
  "message",
  (
    event: MessageEvent<{
      type?: string;
      view?: WorkView;
      preferences?: Preferences;
      mode?: "plan" | "execute";
    }>,
  ) => {
    if (event.data.type === "newConversation" && event.data.mode) {
      mode.value = event.data.mode;
      saveDraft();
      renderComposer();
      return;
    }
    if (event.data.type !== "state" || !event.data.view) return;
    if (event.data.preferences) {
      const next = event.data.preferences;
      if (
        (!initializedPreferences && !persisted?.mode) ||
        (initializedPreferences &&
          preferences.defaultMode !== next.defaultMode &&
          !view?.busy)
      )
        mode.value = next.defaultMode;
      preferences = next;
      initializedPreferences = true;
    }
    render(event.data.view);
  },
);
post({ type: "ready" });
