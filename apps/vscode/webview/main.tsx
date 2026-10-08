import { ConversationComposerFrame } from "@colossus/ui/conversation";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import {
  ComposerInput,
  ComposerModeSwitch,
  ComposerSendButton,
  isComposerSendKey,
} from "@colossus/ui";
import {
  isWorkMode,
  isResearchDepth,
  isResearchSources,
  supportsResearch,
  type ResearchDepth,
  type ResearchOptions,
  type ResearchSource,
  type ViewAction,
  type WorkMode,
  type WorkView,
} from "../src/model.js";
import { DEFAULT_PREFERENCES, type Preferences } from "../src/settings.js";
import { brandMarks, element, icon, node } from "./ui.js";
import { Conversation } from "./conversation.js";

declare function acquireVsCodeApi(): {
  postMessage(message: ViewAction): void;
  getState():
    | ({ draft?: string; mode?: WorkMode } & Partial<ResearchOptions>)
    | undefined;
  setState(state: { draft: string; mode: WorkMode } & ResearchOptions): void;
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
<div class="workspace-row"><button id="workspace" class="workspace-button" title="Worker connection settings"><span class="icon icon-folder" aria-hidden="true"></span><span id="workspace-name">Select a workspace</span></button><span id="connection-status" class="muted">Disconnected</span></div><div class="status-row"><span id="status" role="status">Connect a Colossus worker to start.</span><button id="resume" class="text-button" hidden>Reconnect worker</button></div></footer>`;
brandMarks(app);
const composerRoot = createRoot(element("composer-fields"));
const persisted = api.getState();
let draft = persisted?.draft ?? "";
const mode: { value: WorkMode } = {
  value: isWorkMode(persisted?.mode) ? persisted.mode : "plan",
};
let researchDepth: ResearchDepth = isResearchDepth(persisted?.researchDepth)
  ? persisted.researchDepth
  : "standard";
let researchSources: ResearchSource[] = isResearchSources(
  persisted?.researchSources,
)
  ? [...persisted.researchSources]
  : ["repo"];
const sourceOptions = [
  { value: "repo", label: "This Workspace" },
  { value: "web", label: "Web" },
  { value: "mcp", label: "MCP connections" },
] as const;
let preferences = DEFAULT_PREFERENCES;
let initializedPreferences = false;
let view: WorkView | undefined;
let submittedText: string | undefined;
let submittedAfter = new Set<string>();
let contextOpen = false;
function saveDraft() {
  api.setState({ draft, mode: mode.value, researchDepth, researchSources });
}
function canSend() {
  return (
    !!view?.connected &&
    !view.busy &&
    !!draft.trim() &&
    (mode.value !== "research" ||
      (supportsResearch(view.capabilities) && researchSources.length > 0))
  );
}
function renderComposer() {
  flushSync(() =>
    composerRoot.render(
      <ConversationComposerFrame
        aria-label="Task for Colossus"
        onSubmit={(event) => {
          event.preventDefault();
          send();
        }}
        footer={
          <div className="composer-actions shared-vscode-footer">
            <div className="composer-controls">
              <button
                id="attach"
                type="button"
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
              <ComposerModeSwitch<WorkMode>
                value={mode.value}
                disabled={!view?.connected || view.busy}
                options={[
                  { value: "plan", label: "Plan" },
                  { value: "execute", label: "Execute" },
                  {
                    value: "research",
                    label: "Research",
                    disabled: !view || !supportsResearch(view.capabilities),
                    title:
                      view && supportsResearch(view.capabilities)
                        ? "Investigate a question with cited evidence"
                        : "Research is unavailable for this worker connection",
                  },
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
              disabled={!canSend()}
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
        }
      >
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
                : mode.value === "research"
                  ? "Ask a question to investigate with cited evidence…"
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
        {mode.value === "research" ? (
          <section className="research-controls" aria-label="Research settings">
            <fieldset disabled={!view?.connected || view.busy}>
              <legend>Research depth</legend>
              <div className="research-depth-options">
                {(["quick", "standard", "deep"] as const).map((depth) => (
                  <label key={depth}>
                    <input
                      type="radio"
                      name="research-depth"
                      value={depth}
                      checked={researchDepth === depth}
                      onChange={() => {
                        researchDepth = depth;
                        saveDraft();
                        renderComposer();
                      }}
                    />
                    <span>{depth[0]!.toUpperCase() + depth.slice(1)}</span>
                  </label>
                ))}
              </div>
            </fieldset>
            <fieldset disabled={!view?.connected || view.busy}>
              <legend>Evidence sources</legend>
              <div className="research-source-options">
                {sourceOptions.map((source) => (
                  <label key={source.value}>
                    <input
                      type="checkbox"
                      checked={researchSources.includes(source.value)}
                      onChange={(event) => {
                        researchSources = event.target.checked
                          ? [...researchSources, source.value]
                          : researchSources.filter(
                              (value) => value !== source.value,
                            );
                        saveDraft();
                        renderComposer();
                      }}
                    />
                    <span>{source.label}</span>
                  </label>
                ))}
              </div>
            </fieldset>
            <p className="research-help" role="status">
              {view?.connected && !supportsResearch(view.capabilities)
                ? "Research is unavailable for this worker connection."
                : researchSources.length === 0
                  ? "Select at least one evidence source before starting Research."
                  : "Web uses your research search route; MCP uses enabled tools or research projections on your worker."}
            </p>
          </section>
        ) : null}
      </ConversationComposerFrame>,
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
button("resume", "connect");
button("selection", "addSelection");
button("file", "addFile");
button("changes", "reviewChanges");
button("settings", "openSettings");
button("workspace", "openSettings");
button("connect", "connect");
button("history", "openWorkspace");
function send() {
  if (!canSend() || !view) return;
  submittedText = draft;
  submittedAfter = new Set(
    view.messages.filter((m) => m.role === "user").map((m) => m.id),
  );
  post(
    mode.value === "research"
      ? {
          type: "send",
          text: submittedText,
          mode: "research",
          researchDepth,
          researchSources: [...researchSources],
        }
      : { type: "send", text: submittedText, mode: mode.value },
  );
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
  element("resume").hidden = !next.busy || !next.reconnectable;
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
      mode?: WorkMode;
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
