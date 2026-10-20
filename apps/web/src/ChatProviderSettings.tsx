import { useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import { IconShield } from "@tabler/icons-react";
import {
  ProviderSettingsEditor,
  type ProviderSettingsDraft,
} from "@colossus/ui/components/ProviderSettingsEditor";
import { ProviderPresetPicker } from "@colossus/ui/components/ProviderPresetPicker";
import { ProviderIcon } from "@colossus/ui/components/ProviderIcon";
import {
  matchingProviderPreset,
  presetProviderKind,
  type ProviderPreset,
} from "@colossus/ui/lib/provider-catalog";
import { useResource } from "./resources";
import { projectPath } from "./api";
import {
  validProfile,
  settingsWrite,
  type Authentication,
  type ChatSettingsWrite,
  type ChatSettingsSnapshot,
} from "./chat-settings";

interface PresetAvailability {
  status:
    | "available"
    | "configure_endpoint"
    | "existing_only"
    | "deployment_required";
  reason: string;
}
export function ChatProviderSettings({
  id,
  draft,
  project,
  saved,
  onApply,
  onCancel,
  onCredentials,
  onPersonalCredentials,
}: {
  id: string;
  draft: ChatSettingsWrite;
  project: string;
  saved: ChatSettingsSnapshot;
  onApply: (value: ChatSettingsWrite) => void;
  onCancel: () => void;
  onCredentials?: (() => void) | undefined;
  onPersonalCredentials?: (() => void) | undefined;
}) {
  const original = draft.providers.profiles[id];
  const [value, setValue] = useState<ProviderSettingsDraft>(() => ({
    resourceId: id || null,
    label: draft.labels?.providers[id] ?? id,
    profile: id,
    kind: original?.kind ?? "openai_responses",
    baseUrl: original?.baseUrl ?? "",
    timeoutMs: original?.timeoutMs ?? null,
  }));
  const [auth, setAuth] = useState<Authentication>(
    draft.provider_authentication[id] ?? "personal",
  );
  const [error, setError] = useState("");
  const presets = useResource<{
    presets: ProviderPreset[];
    availability?: Record<string, PresetAvailability>;
  }>(projectPath(project) + "/chat/settings/provider-presets", 0, project);
  const selected = matchingProviderPreset(
    presets.data?.presets ?? [],
    value.kind,
    value.baseUrl,
  );
  const availability = selected
    ? presets.data?.availability?.[selected.id]
    : undefined;
  const codex = value.kind === "open_ai_codex";
  const unavailable =
    availability?.status === "deployment_required" ||
    (availability?.status === "existing_only" &&
      (!id ||
        value.kind !== original?.kind ||
        value.baseUrl !== (original?.baseUrl ?? "")));
  // Only saved, unchanged destinations may open a credential write.
  const destinationSaved = Boolean(
    id &&
    original &&
    value.kind === original.kind &&
    value.baseUrl === (original.baseUrl ?? "") &&
    value.profile === id &&
    value.label === (saved.labels?.providers[id] ?? id) &&
    value.timeoutMs === (original.timeoutMs ?? null) &&
    auth === (saved.provider_authentication[id] ?? "personal") &&
    saved.revision === draft.revision &&
    JSON.stringify(draft) === JSON.stringify(settingsWrite(saved)),
  );
  function apply() {
    if (!validProfile(value.profile) || !value.label.trim()) {
      setError("Enter a display label and a valid profile ID.");
      return;
    }
    if (!id && Object.hasOwn(draft.providers.profiles, value.profile)) {
      setError("That profile ID already exists.");
      return;
    }
    if (unavailable) {
      setError(
        availability?.reason ??
          "This connection requires deployment configuration.",
      );
      return;
    }
    const next = structuredClone(draft);
    next.providers.profiles[value.profile] = {
      ...original,
      kind: value.kind,
      baseUrl: value.kind === "echo" ? null : value.baseUrl.trim(),
      credentialReference: original?.credentialReference ?? null,
      timeoutMs: value.timeoutMs,
    };
    if (value.kind !== "openai_compatible")
      delete next.providers.profiles[value.profile]!
        .chatCompletionsOutputTokenParameter;
    next.provider_authentication[value.profile] =
      value.kind === "echo" ? "none" : auth;
    next.labels ??= { providers: {}, models: {} };
    next.labels.providers[value.profile] = value.label.trim();
    onApply(next);
  }
  return (
    <>
      {error ? <p role="alert">{error}</p> : null}
      <ProviderSettingsEditor
        draft={value}
        busy={false}
        profileReadOnly={Boolean(id)}
        onChange={(next) => {
          if (next.kind !== value.kind || next.baseUrl !== value.baseUrl)
            setError("");
          setValue(next);
        }}
        onCancel={onCancel}
        onSave={apply}
        saveDisabled={unavailable}
        preset={
          <>
            <ProviderPresetPicker
              presets={presets.data?.presets ?? []}
              selected={selected?.id ?? ""}
              disabled={presets.loading}
              placeholder={
                presets.error ? "Providers unavailable" : "Choose a provider"
              }
              icon={(presetId) => <ProviderIcon presetId={presetId} />}
              onSelect={(presetId) => {
                const preset = presets.data?.presets.find(
                  (entry) => entry.id === presetId,
                );
                if (!preset) return;
                setValue({
                  ...value,
                  kind: presetProviderKind(preset),
                  baseUrl: preset.baseUrl ?? "",
                  profile: id || value.profile || preset.id,
                  label: value.label || preset.label,
                });
                setAuth(preset.credentialEnv ? "personal" : "none");
                setError("");
              }}
            />
            {presets.error ? (
              <Button onClick={presets.refresh}>Reload providers</Button>
            ) : null}
            {availability?.reason ? (
              <p className="field-help" role="status">
                {availability.reason}
              </p>
            ) : null}
          </>
        }
        authentication={
          <div className="provider-auth-grid">
            <div className="provider-credential-control">
              <label>
                <span>Authentication</span>
                <DropdownSelect
                  aria-label="Authentication"
                  value={auth}
                  disabled={codex}
                  onChange={(event) =>
                    setAuth(event.target.value as Authentication)
                  }
                >
                  <option value="personal">Personal account (each user)</option>
                  <option value="shared">Shared project credential</option>
                  <option value="none">No authentication</option>
                </DropdownSelect>
              </label>
              <small>
                {codex
                  ? "Uses the deployment-managed subscription account."
                  : "Each person connects their own account unless a project credential is explicitly selected."}
              </small>
              {!codex && auth !== "none" ? (
                <Button
                  disabled={!destinationSaved}
                  onClick={() => {
                    onCancel();
                    if (auth === "shared") onCredentials?.();
                    else onPersonalCredentials?.();
                  }}
                >
                  Add API key securely
                </Button>
              ) : null}
              {!destinationSaved && !codex && auth !== "none" ? (
                <small>
                  Save this provider, then add its key in Credentials.
                </small>
              ) : null}
            </div>
            <div className="provider-auth-note">
              <IconShield size={18} aria-hidden="true" />
              <span>
                <strong>
                  {codex
                    ? "Codex account"
                    : auth === "shared"
                      ? "Project credential"
                      : "Personal credential"}
                </strong>
                <small>
                  Credentials are encrypted in SQL storage. Settings contain
                  references and status only.
                </small>
              </span>
            </div>
          </div>
        }
      />
    </>
  );
}
