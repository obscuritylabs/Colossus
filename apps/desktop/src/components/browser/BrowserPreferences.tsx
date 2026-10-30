import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { browserErrorMessage, nativeBrowserApi } from "../../browser-api";
import type {
  BrowserApi,
  BrowserPreferencesAction,
  BrowserPreferencesStatus,
  HumanBrowserPreferences,
} from "../../browser-api";
import "./browser.css";

export function validBrowserUserAgent(value: string): boolean {
  return (
    value.length <= 512 &&
    /^[\x20-\x7e]*$/.test(value) &&
    value.trim() === value
  );
}

export function BrowserPreferences({
  api = nativeBrowserApi,
}: {
  api?: BrowserApi;
}) {
  const [status, setStatus] = useState<BrowserPreferencesStatus | null>(null);
  const [draft, setDraft] = useState<HumanBrowserPreferences>({
    rememberSignIns: false,
    userAgent: "",
  });
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<"clear_data" | "reset" | null>(null);
  const sequence = useRef(0);
  const activeApi = useRef(api);
  useEffect(() => {
    let live = true;
    const operation = ++sequence.current;
    void (async () => {
      try {
        let selectedApi = api;
        if (
          api === nativeBrowserApi &&
          import.meta.env.DEV &&
          new URLSearchParams(window.location.search).has("fixture")
        ) {
          const { browserFixture } = await import("../../dev/browser-fixture");
          selectedApi = browserFixture("fixture-browser-settings");
        }
        activeApi.current = selectedApi;
        if (!selectedApi.preferences)
          throw new Error("Browser profile settings are unavailable.");
        const context = await selectedApi.context();
        const result = await selectedApi.preferences(context.generation, {
          type: "status",
        });
        if (live && operation === sequence.current) {
          setStatus(result);
          setDraft(result.preferences);
        }
      } catch (cause) {
        if (live && operation === sequence.current)
          setError(browserErrorMessage(cause));
      }
    })();
    return () => {
      live = false;
      ++sequence.current;
    };
  }, [api]);

  async function perform(action: BrowserPreferencesAction) {
    const handler = activeApi.current.preferences;
    if (!status || !handler || busy) return;
    if (action.type === "save" && !validBrowserUserAgent(draft.userAgent)) {
      setError(
        "Use at most 512 printable ASCII characters, without surrounding spaces.",
      );
      return;
    }
    const operation = ++sequence.current;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      // Keep the generation captured when this workspace's settings were read.
      // Fetching a new generation here could apply an old draft to another workspace.
      const result = await handler(status.generation, action);
      if (operation !== sequence.current) return;
      setStatus(result);
      setDraft(result.preferences);
      setConfirm(null);
      setMessage(
        action.type === "save"
          ? "Browser settings saved. Open a new tab to use them."
          : action.type === "reset"
            ? "Browsing data cleared and browser profile settings reset."
            : "Browsing data cleared. Saved sign-ins in this workspace were removed.",
      );
    } catch (cause) {
      if (operation === sequence.current) setError(browserErrorMessage(cause));
    } finally {
      if (operation === sequence.current) setBusy(false);
    }
  }

  const disabled = busy || !status?.available;
  return (
    <form
      className="appearance-settings-card desktop-preferences-card browser-profile-settings"
      aria-label="Workspace browser profile"
      onSubmit={(event) => {
        event.preventDefault();
        void perform({ type: "save", preferences: draft });
      }}
    >
      <div>
        <h4>Workspace browser profile</h4>
        <p className="desktop-preference-note">
          {status?.message ||
            "Choose a workspace to configure saved sign-ins and the browser User-Agent."}
        </p>
      </div>
      <label className="compact-switch desktop-preference-toggle">
        <input
          className="switch-input"
          type="checkbox"
          role="switch"
          checked={draft.rememberSignIns}
          disabled={disabled}
          onChange={(event) => {
            setDraft({ ...draft, rememberSignIns: event.target.checked });
            setMessage("");
          }}
        />
        <span>
          <strong>Remember sign-ins</strong>
          <small>
            Opt in to a separate Windows WebView2 profile for human browsing in
            this workspace. Keeps site data and unexpired persistent cookies
            across restarts; websites still control sign-in expiry. Agent
            sessions never receive these cookies.
          </small>
        </span>
      </label>
      <label
        className="browser-profile-user-agent"
        htmlFor="browser-user-agent"
      >
        <strong>User-Agent</strong>
        <small id="browser-user-agent-help">
          Leave blank for the engine default. A custom value explicitly uses
          WebView2, including for temporary browsing. Some sites may still
          require a supported external browser.
        </small>
        <TextInput
          id="browser-user-agent"
          value={draft.userAgent}
          disabled={disabled}
          autoComplete="off"
          spellCheck={false}
          maxLength={512}
          aria-describedby="browser-user-agent-help"
          placeholder="Engine default"
          onChange={(event) => {
            setDraft({ ...draft, userAgent: event.target.value });
            setError("");
            setMessage("");
          }}
        />
      </label>
      <p className="desktop-preference-note">
        Saving closes human browser tabs in this workspace. Password autofill,
        page access to app commands, downloads and file uploads stay disabled.
        Windows protects saved cookie encryption keys; other site data remains
        in the private app profile.
      </p>
      <div className="browser-profile-actions">
        <Button type="submit" disabled={disabled}>
          Save browser settings
        </Button>
        <Button
          type="button"
          disabled={disabled}
          onClick={() => setConfirm("clear_data")}
        >
          Clear browsing data…
        </Button>
        <Button
          type="button"
          disabled={disabled}
          onClick={() => setConfirm("reset")}
        >
          Reset browser profile…
        </Button>
      </div>
      {confirm ? (
        <div
          className="browser-profile-confirm"
          role="group"
          aria-label="Confirm browsing data removal"
        >
          <p>
            This closes human browser tabs and removes this workspace’s saved
            cookies, site data, cache, history and permissions. You will need to
            sign in again.
            {confirm === "reset"
              ? " The User-Agent and Remember sign-ins settings also return to their defaults."
              : ""}
          </p>
          <div className="browser-profile-actions">
            <Button
              type="button"
              disabled={busy}
              onClick={() => void perform({ type: confirm })}
            >
              Clear{confirm === "reset" ? " and reset" : " browsing data"}
            </Button>
            <Button
              type="button"
              disabled={busy}
              onClick={() => setConfirm(null)}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      {message ? <p role="status">{message}</p> : null}
      {status && !status.available ? (
        <p role="status">These profile controls currently require Windows.</p>
      ) : null}
    </form>
  );
}
