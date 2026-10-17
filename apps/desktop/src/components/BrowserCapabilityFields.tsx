/** @jsxRuntime classic */
/** @jsx element */
/** @jsxFrag Fragment */
import { element, Fragment } from "./browser/browser-jsx";
import { Button, TextInput } from "@colossus/ui";
import type { ManagedFieldDescriptor } from "../types";
import "./browser-settings.css";

export function BrowserCapabilityFields({
  descriptors,
  values,
  effective,
  scope,
  focusedFieldId,
  fieldElementId,
  onChange,
  onInherit,
}: {
  descriptors: ManagedFieldDescriptor[];
  values: Record<string, unknown>;
  effective: Map<string, { value: unknown; source: string }>;
  scope: "global" | "space";
  focusedFieldId?: string | null;
  fieldElementId: (id: string) => string;
  onChange: (id: string, value: unknown) => void;
  onInherit: (id: string) => void;
}) {
  function field(descriptor: ManagedFieldDescriptor) {
    const overridden = Object.hasOwn(values, descriptor.id);
    const inherited = effective.get(descriptor.id);
    const value = overridden
      ? values[descriptor.id]
      : (inherited?.value ?? descriptor.defaultValue);
    const source = overridden ? scope : (inherited?.source ?? "built_in");
    const id = fieldElementId(descriptor.id);
    const toggle = descriptor.control === "toggle";
    return (
      <div
        key={descriptor.id}
        id={id}
        role="group"
        aria-label={descriptor.title}
        className={`browser-setting-row${toggle ? " browser-setting-access" : ""}${focusedFieldId === descriptor.id ? " is-search-target" : ""}`}
        tabIndex={focusedFieldId === descriptor.id ? -1 : undefined}
      >
        <div className="browser-setting-copy">
          <label htmlFor={`${id}-control`}>{descriptor.title}</label>
          <p id={`${id}-help`}>
            {descriptor.id === "browser.enabled"
              ? "Let agents browse and interact with websites using a verified browser."
              : descriptor.description}
          </p>
          <div className="browser-setting-source">
            <span>
              {source === "built_in"
                ? "Built-in default"
                : source === "global"
                  ? "Global default"
                  : "Workspace override"}
            </span>
            {overridden ? (
              <Button
                variant="tertiary"
                onClick={() => {
                  onInherit(descriptor.id);
                  document.getElementById(`${id}-control`)?.focus();
                }}
              >
                Inherit
              </Button>
            ) : null}
          </div>
        </div>
        <div className="browser-setting-control">
          {toggle ? (
            <>
              <span aria-hidden="true">{value ? "On" : "Off"}</span>
              <input
                id={`${id}-control`}
                className="switch-input"
                type="checkbox"
                role="switch"
                checked={Boolean(value)}
                aria-describedby={`${id}-help`}
                onChange={(event) =>
                  onChange(descriptor.id, event.target.checked)
                }
              />
            </>
          ) : (
            <TextInput
              id={`${id}-control`}
              type="number"
              value={Number(value)}
              min={descriptor.minimum ?? undefined}
              max={descriptor.maximum ?? undefined}
              aria-describedby={`${id}-help`}
              onChange={(event) =>
                onChange(descriptor.id, Number(event.target.value))
              }
            />
          )}
        </div>
      </div>
    );
  }
  return (
    <div className="browser-capability-settings">
      <div className="browser-settings-group">
        {descriptors.filter(({ control }) => control === "toggle").map(field)}
      </div>
      <section className="browser-settings-group" aria-label="Resource limits">
        <header>
          <h4>Resource limits</h4>
          <p>Keep each workspace’s browser usage within these bounds.</p>
        </header>
        {descriptors.filter(({ control }) => control !== "toggle").map(field)}
      </section>
      <aside className="browser-skill-note" aria-label="Browser skill">
        <code>@browser</code>
        <div>
          <strong>Built-in browser skill</strong>
          <p>
            Guidance for navigation, forms, and page inspection. Available in
            the composer when browser tools are enabled and permitted.
          </p>
        </div>
      </aside>
      <p className="browser-settings-apply-note">
        {scope === "global"
          ? "Save changes below. Active work finishes before updates apply. Re-enabling access requires Apply in Workspace settings."
          : "Apply changes below to restart this workspace’s runtime and close temporary browser sessions. Inherit restores its accepted global defaults."}
      </p>
    </div>
  );
}
