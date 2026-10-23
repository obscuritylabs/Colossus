import { useId, type ReactNode } from "react";
import { IconSearch, IconX } from "@tabler/icons-react";
import { Button, TextInput } from "./Controls.js";
import { DropdownSelect } from "./DropdownSelect.js";
import "../../styles/service-settings-editor.css";

export interface SearchSettingsDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  kind: "searxng" | "serp_api";
  endpoint: string;
  authHeader: string;
  timeoutMs: number;
  userAgent?: string;
}

/** Controlled service presentation; the host owns credentials, routes and persistence. */
export function SearchSettingsEditor<T extends SearchSettingsDraft>({
  draft,
  busy,
  onChange,
  onSave,
  onCancel,
  authentication,
  profileReadOnly = false,
  saveDisabled = false,
  saveLabel = "Save search service",
  endpointHelp = "Use an origin approved by your administrator.",
}: {
  draft: T;
  busy: boolean;
  onChange: (draft: T) => void;
  onSave: () => void;
  onCancel: () => void;
  authentication?: ReactNode;
  profileReadOnly?: boolean;
  saveDisabled?: boolean;
  saveLabel?: string;
  endpointHelp?: string;
}) {
  const id = useId();
  return (
    <form
      className="service-settings-editor"
      aria-labelledby={`${id}-heading`}
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy && !saveDisabled) onSave();
      }}
    >
      <header>
        <IconSearch size={18} aria-hidden="true" />
        <div>
          <h3 id={`${id}-heading`}>
            {draft.resourceId
              ? `Edit ${draft.label || "search service"}`
              : "Add search service"}
          </h3>
          <p>
            Connect a search service, then choose its agent and research routes.
          </p>
        </div>
        <Button
          type="button"
          variant="tertiary"
          disabled={busy}
          onClick={onCancel}
          aria-label="Close search editor"
        >
          <IconX size={17} aria-hidden="true" />
        </Button>
      </header>
      <fieldset disabled={busy}>
        <legend>Identity</legend>
        <div className="service-settings-grid">
          <label>
            <span>Display label</span>
            <TextInput
              required
              value={draft.label}
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
          </label>
          <label>
            <span>Profile ID</span>
            <TextInput
              required
              readOnly={profileReadOnly}
              value={draft.profile}
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
          </label>
          <label>
            <span>Adapter</span>
            <DropdownSelect
              value={draft.kind}
              onChange={(event) =>
                onChange({ ...draft, kind: event.target.value as T["kind"] })
              }
            >
              <option value="searxng">SearXNG</option>
              <option value="serp_api">SerpAPI</option>
            </DropdownSelect>
          </label>
        </div>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Connection</legend>
        <label>
          <span>Endpoint URL</span>
          <TextInput
            type="url"
            required
            value={draft.endpoint}
            placeholder={
              draft.kind === "searxng"
                ? "https://search.example.com/search"
                : "https://serpapi.com/search"
            }
            onChange={(event) =>
              onChange({ ...draft, endpoint: event.target.value })
            }
          />
          <small>{endpointHelp}</small>
        </label>
        {authentication}
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Request controls</legend>
        <div className="service-settings-grid">
          {draft.kind === "searxng" ? (
            <label>
              <span>Authentication header</span>
              <TextInput
                value={draft.authHeader}
                placeholder="X-Searxng-Key"
                onChange={(event) =>
                  onChange({ ...draft, authHeader: event.target.value })
                }
              />
              <small>Header receiving the attached credential.</small>
            </label>
          ) : null}
          <label>
            <span>Timeout (ms)</span>
            <TextInput
              required
              type="number"
              min={1}
              max={300_000}
              value={draft.timeoutMs}
              onChange={(event) =>
                onChange({ ...draft, timeoutMs: Number(event.target.value) })
              }
            />
          </label>
          {draft.userAgent !== undefined ? (
            <label>
              <span>User agent</span>
              <TextInput
                required
                value={draft.userAgent}
                onChange={(event) =>
                  onChange({ ...draft, userAgent: event.target.value })
                }
              />
            </label>
          ) : null}
        </div>
      </fieldset>
      <footer>
        <Button type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" disabled={busy || saveDisabled}>
          {saveLabel}
        </Button>
      </footer>
    </form>
  );
}
