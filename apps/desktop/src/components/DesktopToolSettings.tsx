import { useEffect, useState } from "react";
import { IconGitBranch, IconWorld, IconTerminal2 } from "@tabler/icons-react";
import { useDesktopPreferences } from "../DesktopPreferencesProvider";
import { validBrowserNewTabUrl } from "../desktop-preferences";
import { DropdownSelect } from "./DropdownSelect";

export function GitSettings() {
  const { gitAutoRefresh, gitDefaultView, updatePreferences } =
    useDesktopPreferences();
  return (
    <section
      className="managed-settings-body desktop-settings"
      aria-labelledby="git-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="git-settings-heading">Git</h3>
          <p className="managed-heading-copy">
            Choose how the read-only Git pane behaves on this device.
          </p>
        </div>
      </div>
      <div className="appearance-settings-card desktop-preferences-card">
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={gitAutoRefresh}
            onChange={(event) =>
              updatePreferences({ gitAutoRefresh: event.target.checked })
            }
          />
          <span>
            <strong>Automatically refresh Git</strong>
            <small>
              Check when the app regains focus and periodically while a
              workspace is open. Manual refresh stays available.
            </small>
          </span>
        </label>
        <label className="desktop-preference-select" htmlFor="git-default-view">
          <IconGitBranch size={18} aria-hidden="true" />
          <span>
            <strong>Opening view</strong>
            <small>Show this view first when you open the Git pane.</small>
          </span>
          <DropdownSelect
            id="git-default-view"
            value={gitDefaultView}
            onChange={(event) =>
              updatePreferences({
                gitDefaultView: event.target.value as "changes" | "history",
              })
            }
          >
            <option value="changes">Changes</option>
            <option value="history">History</option>
          </DropdownSelect>
        </label>
      </div>
    </section>
  );
}

export function BrowserSettings() {
  const { browserNewTabUrl, updatePreferences } = useDesktopPreferences();
  const [draft, setDraft] = useState(browserNewTabUrl);
  const [error, setError] = useState("");
  useEffect(() => setDraft(browserNewTabUrl), [browserNewTabUrl]);
  function save() {
    const url = validBrowserNewTabUrl(draft.trim());
    if (url === null) {
      setError(
        "Enter a full http:// or https:// URL without credentials, or leave the field blank.",
      );
      return;
    }
    updatePreferences({ browserNewTabUrl: url });
    setDraft(url);
    setError("");
  }
  return (
    <section
      className="managed-settings-body desktop-settings"
      aria-labelledby="browser-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="browser-settings-heading">Browser</h3>
          <p className="managed-heading-copy">
            Choose the page opened by the new-tab button in Desktop's temporary
            browser.
          </p>
        </div>
      </div>
      <form
        className="appearance-settings-card desktop-browser-settings"
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <label htmlFor="browser-new-tab-url">
          <IconWorld size={18} aria-hidden="true" />
          <span>
            <strong>New-tab page</strong>
            <small id="browser-new-tab-help">
              Leave blank for an empty tab. Use a full web address, including
              for localhost previews.
            </small>
          </span>
        </label>
        <div className="desktop-browser-url-controls">
          <input
            id="browser-new-tab-url"
            type="url"
            value={draft}
            placeholder="https://example.com/"
            aria-describedby={
              error
                ? "browser-new-tab-help browser-new-tab-error"
                : "browser-new-tab-help"
            }
            aria-invalid={error ? true : undefined}
            onChange={(event) => {
              setDraft(event.target.value);
              setError("");
            }}
          />
          <button className="button secondary" type="submit">
            Save
          </button>
          {browserNewTabUrl ? (
            <button
              className="button secondary"
              type="button"
              onClick={() => {
                updatePreferences({ browserNewTabUrl: "" });
                setDraft("");
                setError("");
              }}
            >
              Use empty tab
            </button>
          ) : null}
        </div>
        {error ? (
          <p id="browser-new-tab-error" role="alert">
            {error}
          </p>
        ) : null}
        <p className="desktop-preference-note">
          The browser session remains temporary. This setting only affects new
          tabs you open; it does not restore browsing history.
        </p>
      </form>
    </section>
  );
}

export function TerminalSettings({
  enabled,
  consentPending,
  disabled,
  shellAvailable,
  onSetEnabled,
}: {
  enabled: boolean;
  consentPending: boolean;
  disabled: boolean;
  shellAvailable: boolean;
  onSetEnabled: (enabled: boolean) => void;
}) {
  const { terminalDefaultSession, updatePreferences } = useDesktopPreferences();
  return (
    <section
      className="managed-settings-body desktop-settings"
      aria-labelledby="terminal-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="terminal-settings-heading">Terminal</h3>
          <p className="managed-heading-copy">
            Control local terminal access and choose what opens from the
            Terminal tool.
          </p>
        </div>
      </div>
      <div className="appearance-settings-card desktop-preferences-card">
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            aria-label="Enable local terminal"
            aria-describedby="local-terminal-help"
            checked={enabled || consentPending}
            disabled={disabled}
            onChange={(event) => onSetEnabled(event.target.checked)}
          />
          <span>
            <strong>Enable local terminal</strong>
            <small id="local-terminal-help">
              Available by default for new workspaces. First use requires native
              confirmation because shells run with your operating-system
              permissions.
            </small>
          </span>
        </label>
        {consentPending ? (
          <div className="desktop-terminal-consent">
            <p>
              Native confirmation is required before the first session opens.
            </p>
            <button
              className="button secondary"
              type="button"
              disabled={disabled}
              onClick={() => onSetEnabled(true)}
            >
              Confirm terminal access
            </button>
          </div>
        ) : null}
        <label
          className="desktop-preference-select"
          htmlFor="terminal-default-session"
        >
          <IconTerminal2 size={18} aria-hidden="true" />
          <span>
            <strong>Default session</strong>
            <small>Choose what opens when you select the Terminal tool.</small>
          </span>
          <DropdownSelect
            id="terminal-default-session"
            value={shellAvailable ? terminalDefaultSession : "automatic"}
            onChange={(event) =>
              updatePreferences({
                terminalDefaultSession: event.target.value as
                  "automatic" | "shell",
              })
            }
          >
            <option value="automatic">Automatic</option>
            <option value="shell" disabled={!shellAvailable}>
              System shell
            </option>
          </DropdownSelect>
        </label>
        <p className="desktop-preference-note">
          Automatic opens the Colossus TUI when the managed runtime is ready,
          and a shell otherwise.
        </p>
      </div>
    </section>
  );
}
