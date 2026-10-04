import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { SettingsFrame, DropdownSelect } from "@colossus/ui";
import {
  DEFAULT_PREFERENCES,
  type Preferences,
  type SettingsAction,
  type SettingsView,
} from "../src/settings.js";
import { element, applyPalette } from "./ui.js";

type Scope = "global" | "workspace";
type Section = "appearance" | "defaults" | "connections" | "runtime" | "access";
declare function acquireVsCodeApi(): {
  postMessage(message: SettingsAction): void;
  getState(): { scope?: Scope; section?: Section } | undefined;
  setState(state: { scope: Scope; section: Section }): void;
};
const api = acquireVsCodeApi();
const saved = api.getState();
const labels: Record<Section, string> = {
  appearance: "Appearance",
  defaults: "Defaults",
  connections: "Connections",
  runtime: "Runtime",
  access: "Access",
};
const initialView: SettingsView = {
  preferences: DEFAULT_PREFERENCES,
  preferenceError: "",
  workspace: "Select a workspace",
  connected: false,
  connecting: false,
  busy: false,
  hasSavedConnection: false,
  version: "",
  role: "",
  error: "",
};
const post = (action: SettingsAction) => api.postMessage(action);
function Settings() {
  const [view, setView] = useState(initialView);
  const [scope, setScope] = useState<Scope>(
    saved?.scope === "workspace" ? "workspace" : "global",
  );
  const [section, setSection] = useState<Section>(() => {
    const tabs: Section[] =
      saved?.scope === "workspace"
        ? ["connections", "runtime", "access"]
        : ["appearance", "defaults"];
    return saved?.section && tabs.includes(saved.section)
      ? saved.section
      : tabs[0]!;
  });
  const [query, setQuery] = useState("");
  const contentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const receive = (
      event: MessageEvent<{ type?: string; view?: SettingsView }>,
    ) => {
      if (event.data.type === "settings" && event.data.view)
        setView(event.data.view);
    };
    window.addEventListener("message", receive);
    post({ type: "ready" });
    return () => window.removeEventListener("message", receive);
  }, []);
  useEffect(
    () => applyPalette(view.preferences.palette),
    [view.preferences.palette],
  );
  useLayoutEffect(() => {
    api.setState({ scope, section });
    const search = query.trim().toLowerCase();
    let visible = 0;
    for (const page of contentRef.current!.querySelectorAll<HTMLElement>(
      "[data-section]",
    )) {
      let matchingRows = 0;
      for (const row of page.querySelectorAll<HTMLElement>("[data-search]")) {
        row.hidden =
          !!search &&
          !`${page.dataset.section} ${row.dataset.search} ${row.textContent}`
            .toLowerCase()
            .includes(search);
        if (!row.hidden) matchingRows++;
      }
      page.hidden = search ? !matchingRows : page.dataset.section !== section;
      if (!page.hidden) visible++;
    }
    element("no-results").hidden = visible > 0;
  }, [query, section, scope, view]);
  function select(nextScope: Scope, nextSection: Section) {
    setScope(nextScope);
    setSection(nextSection);
    setQuery("");
  }
  function preference(
    action: Extract<SettingsAction, { type: "setPreference" }>,
  ) {
    setView((current) => ({
      ...current,
      preferenceError: "",
      preferences: { ...current.preferences, [action.name]: action.value },
    }));
    post(action);
  }
  const tabs: Section[] =
    scope === "global"
      ? ["appearance", "defaults"]
      : ["connections", "runtime", "access"];
  return (
    <SettingsFrame
      standalone
      scope={scope === "global" ? "global" : "space"}
      onScopeChange={(next) =>
        select(
          next === "global" ? "global" : "workspace",
          next === "global" ? "appearance" : "connections",
        )
      }
      query={query}
      onQueryChange={setQuery}
      tabs={tabs.map((id) => ({
        id,
        label: labels[id],
        group: scope === "global" ? "VS Code" : "Workspace settings",
      }))}
      activeTab={section}
      onTabChange={(tab) => select(scope, tab as Section)}
      workspaceContext={
        <>
          <span className="muted">Workspace</span>
          <strong>{view.workspace}</strong>
        </>
      }
      brandMarkUrl={document.body.dataset.colossusMark ?? ""}
      globalDescription="Preferences for this editor"
      workspaceDescription="Connected worker & access"
      onReturnToWork={() => post({ type: "openWork" })}
    >
      <div ref={contentRef}>
        <p id="breadcrumb" className="surface-breadcrumb">
          {query
            ? "Search settings"
            : `${scope === "global" ? "Global" : "Workspace"} / ${labels[section]}`}
        </p>
        {view.preferenceError && (
          <div className="error" role="alert">
            <p>{view.preferenceError}</p>
            <button
              className="secondary"
              onClick={() => post({ type: "openUserSettings" })}
            >
              Open user settings
            </button>
          </div>
        )}
        <section
          data-section="appearance"
          hidden={!query && section !== "appearance"}
          data-scope="global"
          className="settings-page"
          aria-labelledby="appearance-title"
        >
          <h2 id="appearance-title">Appearance</h2>
          <p className="section-description">
            Make Colossus comfortable to use in your editor.
          </p>
          <div className="settings-card">
            <div className="setting-row" data-search="theme color appearance">
              <div>
                <strong>Color theme</strong>
                <p>Match VS Code’s light, dark, or high contrast theme.</p>
              </div>
              <button
                id="theme"
                onClick={() => post({ type: "openThemeSettings" })}
                className="secondary"
              >
                Change VS Code theme
              </button>
            </div>
            <div
              className="setting-row"
              data-search="palette dark neutral black blue green hacker tui desktop appearance"
            >
              <div>
                <label htmlFor="palette">Surface palette</label>
                <p>
                  Dark palettes: neutral charcoal, Colossus blue, or the TUI’s
                  green Hacker theme. Light and high contrast follow VS Code.
                </p>
              </div>
              <DropdownSelect
                id="palette"
                value={view.preferences.palette}
                onChange={(event) =>
                  preference({
                    type: "setPreference",
                    name: "palette",
                    value: event.target.value as Preferences["palette"],
                  })
                }
              >
                <option value="editor">Editor (Dark+)</option>
                <option value="colossus">Colossus blue</option>
                <option value="hacker">Hacker (TUI)</option>
              </DropdownSelect>
            </div>
            <div
              className="setting-row"
              data-search="tools activity appearance"
            >
              <div>
                <label htmlFor="showToolActivity">Show tool activity</label>
                <p>Show compact tool progress in each conversation turn.</p>
              </div>
              <input
                id="showToolActivity"
                type="checkbox"
                role="switch"
                checked={view.preferences.showToolActivity}
                onChange={(event) =>
                  preference({
                    type: "setPreference",
                    name: "showToolActivity",
                    value: event.target.checked,
                  })
                }
              />
            </div>
          </div>
        </section>
        <section
          data-section="defaults"
          hidden={!query && section !== "defaults"}
          data-scope="global"
          className="settings-page"
          aria-labelledby="defaults-title"
        >
          <h2 id="defaults-title">Defaults</h2>
          <p className="section-description">
            Choose how new work starts in Colossus.
          </p>
          <h3>Composer</h3>
          <div className="settings-card">
            <div
              className="setting-row"
              data-search="default run mode plan execute composer"
            >
              <div>
                <label htmlFor="defaultMode">Default run mode</label>
                <p>Start new conversations in Plan or Execute mode.</p>
              </div>
              <DropdownSelect
                id="defaultMode"
                value={view.preferences.defaultMode}
                onChange={(event) =>
                  preference({
                    type: "setPreference",
                    name: "defaultMode",
                    value: event.target.value as "plan" | "execute",
                  })
                }
              >
                <option value="plan">Plan</option>
                <option value="execute">Execute</option>
              </DropdownSelect>
            </div>
            <div
              className="setting-row"
              data-search="send shortcut enter keyboard composer"
            >
              <div>
                <label htmlFor="sendShortcut">Send shortcut</label>
                <p>Shift+Enter always inserts a new line.</p>
              </div>
              <DropdownSelect
                id="sendShortcut"
                value={view.preferences.sendShortcut}
                onChange={(event) =>
                  preference({
                    type: "setPreference",
                    name: "sendShortcut",
                    value: event.target.value as "enter" | "modEnter",
                  })
                }
              >
                <option value="modEnter">Ctrl / ⌘ + Enter</option>
                <option value="enter">Enter</option>
              </DropdownSelect>
            </div>
          </div>
        </section>
        <section
          data-section="connections"
          hidden={!query && section !== "connections"}
          data-scope="workspace"
          className="settings-page"
          aria-labelledby="connections-title"
        >
          <h2 id="connections-title">Connections</h2>
          <p className="section-description">
            Connect this workspace to your enrolled Colossus worker.
          </p>
          <div className="settings-card">
            <div
              className="setting-row"
              data-search="connect disconnect external worker"
            >
              <div>
                <strong>Local worker</strong>
                <p id="worker-status" role="status">
                  {view.connecting
                    ? "Connecting…"
                    : view.connected
                      ? `Connected to ${view.workspace}`
                      : "Disconnected"}
                </p>
              </div>
              <button
                id="connect"
                disabled={view.connecting || view.busy}
                onClick={() =>
                  post({ type: view.connected ? "disconnect" : "connect" })
                }
              >
                {view.connecting
                  ? "Connecting…"
                  : view.connected
                    ? "Disconnect"
                    : "Connect worker"}
              </button>
            </div>
            <div
              className="setting-row"
              data-search="enrollment credential keyring service account location"
            >
              <div>
                <strong>Enrolled credential</strong>
                <p>
                  Check the OS-keyring service and account used by this
                  connection.
                </p>
              </div>
              <button
                id="credential"
                onClick={() => post({ type: "configureCredential" })}
                disabled={view.connecting || view.busy}
                className="secondary"
              >
                Credential location
              </button>
            </div>
            <div
              className="setting-row"
              data-search="connection diagnostics troubleshooting"
            >
              <div>
                <strong>Connection diagnostics</strong>
                <p>Inspect connection stages in the Colossus output channel.</p>
              </div>
              <button
                id="diagnostics"
                onClick={() => post({ type: "showConnectionLog" })}
                className="secondary"
              >
                Show diagnostics
              </button>
            </div>
            <div
              className="setting-row"
              data-search="forget reset enrollment trust connection"
            >
              <div>
                <strong>Saved connection</strong>
                <p>
                  Forget this workspace’s saved trust settings, then connect
                  again.
                </p>
              </div>
              <button
                id="forget"
                onClick={() => post({ type: "forgetConnection" })}
                disabled={view.connecting || view.busy}
                className="secondary"
              >
                Forget connection
              </button>
            </div>
          </div>
          <p className="settings-note">
            Forgetting a connection leaves the worker’s enrollment and keychain
            credential in place.
          </p>
        </section>
        <section
          data-section="runtime"
          hidden={!query && section !== "runtime"}
          data-scope="workspace"
          className="settings-page"
          aria-labelledby="runtime-title"
        >
          <h2 id="runtime-title">Runtime</h2>
          <p className="section-description">
            The connected worker runs your Colossus tasks.
          </p>
          <div className="settings-card">
            <div
              className="setting-row"
              data-search="runtime cli external worker startup"
            >
              <div>
                <strong>Runtime placement</strong>
                <p>Start the worker with the Colossus CLI before connecting.</p>
              </div>
              <span className="setting-value">External worker</span>
            </div>
            <div className="setting-row" data-search="worker version runtime">
              <div>
                <strong>Worker version</strong>
                <p>Version reported by the connected worker.</p>
              </div>
              <span id="version" className="setting-value">
                {view.version || "Not connected"}
              </span>
            </div>
            <div
              className="setting-row"
              data-search="model role provider runtime"
            >
              <div>
                <strong>Model role</strong>
                <p>The role selected during enrollment setup.</p>
              </div>
              <span id="role" className="setting-value">
                {view.role || "Not configured"}
              </span>
            </div>
          </div>
          <h3>Models, providers &amp; MCP</h3>
          <p className="settings-note">
            Manage these in the connected worker’s configuration. The current
            worker connection supports running tasks; it does not expose runtime
            settings editing.
          </p>
        </section>
        <section
          data-section="access"
          hidden={!query && section !== "access"}
          data-scope="workspace"
          className="settings-page"
          aria-labelledby="access-title"
        >
          <h2 id="access-title">Access</h2>
          <p className="section-description">
            Colossus applies the worker’s policy and the application’s
            enrollment to every task.
          </p>
          <div className="settings-card">
            <div
              className="setting-row"
              data-search="approval security review access"
            >
              <div>
                <strong>Approvals</strong>
                <p>
                  Review requested actions in VS Code’s native confirmation
                  dialogs.
                </p>
              </div>
              <span className="setting-value">Worker policy</span>
            </div>
            <div
              className="setting-row"
              data-search="tools permissions sandbox enrollment access"
            >
              <div>
                <strong>Tools &amp; sandbox</strong>
                <p>
                  Configured by the worker and limited by this application’s
                  enrollment.
                </p>
              </div>
              <span className="setting-value">Worker managed</span>
            </div>
          </div>
          <p className="settings-note">
            Changing a composer preference does not change the worker’s
            permissions.
          </p>
        </section>
        <p id="no-results" className="settings-note" hidden>
          No matching settings.
        </p>
        <div id="error" className="error" role="alert" hidden={!view.error}>
          {view.error}
        </div>
      </div>
    </SettingsFrame>
  );
}
flushSync(() => createRoot(element("app")).render(<Settings />));
