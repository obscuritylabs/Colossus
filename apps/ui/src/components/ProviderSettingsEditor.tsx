import { useId, type ReactNode } from "react";
import { IconCheck, IconCloud, IconX } from "@tabler/icons-react";
import { Button, TextInput } from "./Controls.js";
import { DropdownSelect } from "./DropdownSelect.js";
import { ProviderSetupSection } from "./ProviderSetupSection.js";
import "../../styles/provider-settings-editor.css";

export interface ProviderSettingsDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  kind: string;
  baseUrl: string;
  timeoutMs: number | null;
}

/** Hosts retain preset discovery, credential custody, validation and persistence. */
export function ProviderSettingsEditor<T extends ProviderSettingsDraft>({
  draft,
  busy,
  onChange,
  onCancel,
  onSave,
  preset,
  authentication,
  presentation,
  profileReadOnly = false,
  saveDisabled = false,
  saveLabel,
  cancelLabel = "Cancel",
  endpointHelp = "Use HTTPS except when connecting to a service on this device.",
}: {
  draft: T;
  busy: boolean;
  onChange: (draft: T) => void;
  onCancel: () => void;
  onSave: () => void;
  preset: ReactNode;
  authentication: ReactNode;
  presentation?: ReactNode;
  profileReadOnly?: boolean;
  saveDisabled?: boolean;
  saveLabel?: string;
  cancelLabel?: string;
  endpointHelp?: string;
}) {
  const id = useId();
  const codex = draft.kind === "open_ai_codex";
  const title = draft.resourceId
    ? `Edit ${draft.label || "provider"}`
    : "Add provider";
  const adapterDescription =
    draft.kind === "anthropic_messages"
      ? "Use the native Anthropic Messages API."
      : draft.kind === "openai_responses"
        ? "Use for providers that support the OpenAI Responses API."
        : draft.kind === "openai_compatible"
          ? "Use for providers with an OpenAI-compatible API."
          : "Use your Codex subscription; the endpoint and sign-in are managed for you.";
  return (
    <form
      className="mcp-editor catalog-editor provider-editor shared-provider-settings-editor"
      aria-labelledby={`${id}-heading`}
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy && !saveDisabled) onSave();
      }}
    >
      <div className="provider-editor-heading">
        <span className="resource-icon">
          <IconCloud size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id={`${id}-heading`}>{title}</h4>
          <p>
            Connect Colossus to a model provider. You can reuse this connection
            for more than one model.
          </p>
        </div>
        <Button
          className="ui-icon-button"
          aria-label="Close provider editor"
          disabled={busy}
          onClick={onCancel}
        >
          <IconX size={17} />
        </Button>
      </div>
      <ProviderSetupSection
        title="Identity"
        description="Choose a name shown in settings and an ID used in configuration."
      >
        <div className="provider-editor-grid">
          <label>
            <span>Display label</span>
            <TextInput
              disabled={busy}
              value={draft.label}
              placeholder="For example, OpenRouter production"
              required
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small>Shown to people in settings.</small>
          </label>
          <label>
            <span>Profile ID</span>
            <TextInput
              disabled={busy || profileReadOnly}
              value={draft.profile}
              placeholder="openrouter-production"
              required
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
            <small>
              Used to identify this provider in model configuration.
            </small>
          </label>
        </div>
      </ProviderSetupSection>
      <ProviderSetupSection
        title="Connection"
        description="Choose the provider API format and endpoint."
      >
        {preset}
        <div className="provider-editor-grid provider-connection-grid">
          <label>
            <span>Adapter</span>
            <DropdownSelect
              disabled={busy}
              value={draft.kind}
              onChange={(event) =>
                onChange({ ...draft, kind: event.target.value })
              }
            >
              <option value="openai_responses">OpenAI Responses</option>
              <option value="anthropic_messages">Anthropic Messages</option>
              <option value="openai_compatible">OpenAI compatible</option>
              <option value="open_ai_codex">Codex subscription</option>
              {draft.kind === "echo" ? (
                <option value="echo">Echo (local test)</option>
              ) : null}
            </DropdownSelect>
            <small>{adapterDescription}</small>
          </label>
          <label>
            <span>Endpoint URL</span>
            <TextInput
              type="url"
              value={
                codex ? "https://chatgpt.com/backend-api/codex" : draft.baseUrl
              }
              disabled={busy || codex}
              required={!codex && draft.kind !== "echo"}
              onChange={(event) =>
                onChange({ ...draft, baseUrl: event.target.value })
              }
            />
            <small>
              {codex
                ? "This endpoint is managed by the Codex subscription connection."
                : endpointHelp}
            </small>
          </label>
        </div>
      </ProviderSetupSection>
      <ProviderSetupSection
        title="Authentication"
        description="Choose how this provider authenticates its requests."
      >
        {authentication}
      </ProviderSetupSection>
      <ProviderSetupSection
        title="Runtime"
        description="Set a custom request timeout, or leave it blank to use the provider default."
      >
        <div className="provider-runtime-control">
          <label>
            <span>Request timeout (ms)</span>
            <TextInput
              disabled={busy}
              type="number"
              min={1}
              max={3600000}
              value={draft.timeoutMs ?? ""}
              placeholder="Provider default"
              onChange={(event) =>
                onChange({
                  ...draft,
                  timeoutMs:
                    event.target.value === ""
                      ? null
                      : Number(event.target.value),
                })
              }
            />
            <small>
              Leave empty to use the provider default, up to one hour.
            </small>
          </label>
        </div>
      </ProviderSetupSection>
      {presentation}
      <div className="mcp-editor-actions">
        <Button disabled={busy} onClick={onCancel}>
          {cancelLabel}
        </Button>
        <Button variant="primary" type="submit" disabled={busy || saveDisabled}>
          <IconCheck size={16} />
          {saveLabel ?? (draft.resourceId ? "Save changes" : "Add provider")}
        </Button>
      </div>
    </form>
  );
}
