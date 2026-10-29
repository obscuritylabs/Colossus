import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button, TextInput } from "@colossus/ui";
import { DropdownSelect } from "./DropdownSelect";
import {
  featureError,
  getDesktopFeatures,
  saveDesktopFeatures,
} from "../desktop-extras";
import type {
  DesktopFeatureContext,
  DesktopFeaturePreferences,
} from "../desktop-extras";
import "./desktop-extras.css";
import { useDesktopPreferences } from "../DesktopPreferencesProvider";

export function DesktopExtrasSettings() {
  const { nextPromptMode, updatePreferences } = useDesktopPreferences();
  const [context, setContext] = useState<DesktopFeatureContext | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [shortcut, setShortcut] = useState("");
  const latest = useRef<DesktopFeatureContext | null>(null);
  const mounted = useRef(false);
  const saving = useRef(false);
  const requestSequence = useRef(0);
  const lastAppliedRequest = useRef(0);
  function applyContext(value: DesktopFeatureContext, request: number) {
    const previous = latest.current;
    if (
      !mounted.current ||
      (previous !== null &&
        (value.revision < previous.revision ||
          (value.revision === previous.revision &&
            request < lastAppliedRequest.current)))
    )
      return;
    latest.current = value;
    lastAppliedRequest.current = Math.max(lastAppliedRequest.current, request);
    setContext(value);
    setShortcut(value.preferences.appshotShortcut);
    setError("");
  }
  function reportFailure(
    failure: unknown,
    request: number,
    revision: number | null,
  ) {
    if (
      mounted.current &&
      request >= lastAppliedRequest.current &&
      (latest.current?.revision ?? null) === revision
    )
      setError(featureError(failure));
  }
  useEffect(() => {
    let live = true;
    mounted.current = true;
    const load = () => {
      const request = ++requestSequence.current;
      const revision = latest.current?.revision ?? null;
      void getDesktopFeatures().then(
        (value) => {
          if (live) applyContext(value, request);
        },
        (failure) => {
          if (live) reportFailure(failure, request, revision);
        },
      );
    };
    load();
    const subscription =
      "__TAURI_INTERNALS__" in window
        ? listen("desktop-features-changed", load)
        : null;
    return () => {
      if (subscription)
        void subscription.then(
          (unlisten) => unlisten(),
          () => {},
        );
      live = false;
      mounted.current = false;
    };
  }, []);
  async function update(patch: Partial<DesktopFeaturePreferences>) {
    const current = latest.current;
    if (current === null || saving.current) return;
    saving.current = true;
    const request = ++requestSequence.current;
    setBusy(true);
    setError("");
    try {
      const value = await saveDesktopFeatures(
        {
          ...current.preferences,
          ...patch,
        },
        current.revision,
      );
      applyContext(value, request);
    } catch (failure) {
      reportFailure(failure, request, current.revision);
    } finally {
      saving.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  if (context === null)
    return (
      <section className="managed-settings-body desktop-settings">
        <p role={error ? "alert" : "status"}>
          {error || "Loading Desktop features…"}
        </p>
      </section>
    );
  const p = context.preferences;
  return (
    <section
      className="managed-settings-body desktop-settings desktop-features"
      aria-labelledby="desktop-features-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="desktop-features-heading">App Shots & companion</h3>
          <p className="managed-heading-copy">
            Capture context into your draft and keep a small companion beside
            your work.
          </p>
        </div>
      </div>
      {error && <p role="alert">{error}</p>}
      <div className="appearance-settings-card desktop-preferences-card">
        <label className="desktop-preference-select" htmlFor="next-prompt-mode">
          <span>
            <strong>Suggest the next prompt</strong>
            <small>
              Tab or click adds an editable draft. Local uses the completed
              response. Model sends recent released conversation prose to that
              run’s model and uses extra provider tokens; it falls back to Local
              when unavailable. Suggestions never send a message.
            </small>
          </span>
          <DropdownSelect
            id="next-prompt-mode"
            value={nextPromptMode}
            onChange={(e) =>
              updatePreferences({
                nextPromptMode: e.target.value as "off" | "local" | "model",
              })
            }
          >
            <option value="off">Off</option>
            <option value="local">Local · no extra model call</option>
            <option value="model">Model · extra token usage</option>
          </DropdownSelect>
        </label>
      </div>
      <div className="appearance-settings-card desktop-preferences-card">
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={p.appshotsEnabled}
            disabled={busy || !context.appshotsAvailable}
            onChange={(e) => void update({ appshotsEnabled: e.target.checked })}
          />
          <span>
            <strong>App Shots</strong>
            <small>
              {context.appshotsAvailable
                ? "Use the shortcut from another app. Review its screenshot and visible text before attaching to the current chat draft."
                : "App Shots currently require Windows."}
            </small>
          </span>
        </label>
        <form
          className="desktop-feature-shortcut"
          onSubmit={(e) => {
            e.preventDefault();
            void update({ appshotShortcut: shortcut.trim() });
          }}
        >
          <label htmlFor="appshot-shortcut">
            <strong>Capture shortcut</strong>
            <small>
              Use Control, Alt and a letter; Shift and Super are optional.
              Shortcut conflicts are reported.
            </small>
          </label>
          <TextInput
            id="appshot-shortcut"
            value={shortcut}
            disabled={busy || !context.appshotsAvailable}
            onChange={(e) => setShortcut(e.target.value)}
            maxLength={64}
          />
          <Button
            type="submit"
            disabled={
              busy ||
              !context.appshotsAvailable ||
              shortcut === p.appshotShortcut
            }
          >
            Save shortcut
          </Button>
        </form>
        {context.shortcutError && <p role="alert">{context.shortcutError}</p>}
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={p.appshotIncludeText}
            disabled={busy || !context.appshotsAvailable}
            onChange={(e) =>
              void update({ appshotIncludeText: e.target.checked })
            }
          />
          <span>
            <strong>Include available text</strong>
            <small>
              Read bounded visible accessibility labels from the captured
              window. Password-marked fields are excluded. You can omit text
              when attaching.
            </small>
          </span>
        </label>
      </div>
      <div className="managed-section-heading">
        <div>
          <h3>Developer companion</h3>
          <p className="managed-heading-copy">
            A quiet mini hacker that shows your current chat’s activity. Drag it
            to a convenient spot.
          </p>
        </div>
      </div>
      <div className="appearance-settings-card desktop-preferences-card">
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={p.petEnabled}
            disabled={busy}
            onChange={(e) => void update({ petEnabled: e.target.checked })}
          />
          <span>
            <strong>Show mini hacker</strong>
            <small>
              Idle, working, needs input, and complete states follow the
              selected chat. Open Colossus or hide the companion from its
              controls.
            </small>
          </span>
        </label>
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={p.petAlwaysOnTop}
            disabled={busy}
            onChange={(e) => void update({ petAlwaysOnTop: e.target.checked })}
          />
          <span>
            <strong>Keep companion on top</strong>
            <small>Show the small floating window above other apps.</small>
          </span>
        </label>
        <label className="compact-switch desktop-preference-toggle">
          <input
            className="switch-input"
            type="checkbox"
            role="switch"
            checked={p.petMotion}
            disabled={busy}
            onChange={(e) => void update({ petMotion: e.target.checked })}
          />
          <span>
            <strong>Animate companion</strong>
            <small>
              Subtle blinking and activity effects. Your system’s reduced-motion
              preference is always respected.
            </small>
          </span>
        </label>
        <label className="desktop-preference-select" htmlFor="pet-size">
          <span>
            <strong>Companion size</strong>
            <small>
              Choose the character’s size inside its compact window.
            </small>
          </span>
          <DropdownSelect
            id="pet-size"
            value={p.petSize}
            disabled={busy}
            onChange={(e) =>
              void update({ petSize: e.target.value as "small" | "large" })
            }
          >
            <option value="small">Mini</option>
            <option value="large">Comfortable</option>
          </DropdownSelect>
        </label>
      </div>
    </section>
  );
}
