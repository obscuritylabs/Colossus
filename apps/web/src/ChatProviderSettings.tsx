import { useEffect, useRef, useState } from "react";
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
import type { CredentialSetup, CredentialTarget } from "./credential-types";

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
  settingsPath,
  saved,
  onApply,
  onCancel,
  onCredentials,
  onPersonalCredentials,
  busy = false,
  instanceScope = false,
  onDirtyChange,
}: {
  busy?: boolean;
  instanceScope?: boolean;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  id: string;
  draft: ChatSettingsWrite;
  project: string;
  settingsPath?: string | undefined;
  saved: ChatSettingsSnapshot;
  onApply: (value: ChatSettingsWrite, setup?: CredentialSetup) => void;
  onCancel: () => void;
  onCredentials?: ((target?: CredentialTarget) => void) | undefined;
  onPersonalCredentials?: ((target?: CredentialTarget) => void) | undefined;
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
  const formValue = JSON.stringify([value, auth]);
  const openingValue = useRef(formValue);
  useEffect(() => {
    onDirtyChange?.(formValue !== openingValue.current);
  }, [formValue, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const presets = useResource<{
    presets: ProviderPreset[];
    availability?: Record<string, PresetAvailability>;
  }>(
    (settingsPath ?? projectPath(project) + "/chat/settings") +
      "/provider-presets",
    0,
    settingsPath ?? project,
  );
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
  const availabilityReason = availability?.reason;
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
  function apply(continueToCredential = false) {
    if (busy) return;
    if (!validProfile(value.profile)) {
      setError("Enter a valid profile ID.");
      return;
    }
    const label = value.label.trim();
    if (
      !label ||
      new TextEncoder().encode(label).length > 96 ||
      /[\u0000-\u001f\u007f-\u009f]/u.test(label)
    ) {
      setError(
        "Display label must contain 1–96 UTF-8 bytes and no control characters.",
      );
      return;
    }
    if (!id && Object.hasOwn(draft.providers.profiles, value.profile)) {
      setError("That profile ID already exists.");
      return;
    }
    if (value.kind !== "echo" && value.kind !== "open_ai_codex") {
      try {
        const endpoint = new URL(value.baseUrl.trim());
        if (
          !["http:", "https:"].includes(endpoint.protocol) ||
          endpoint.username ||
          endpoint.password ||
          value.baseUrl.includes("?") ||
          value.baseUrl.includes("#")
        )
          throw new Error();
      } catch {
        setError(
          "Enter an HTTP or HTTPS endpoint without embedded credentials, a query, or a fragment.",
        );
        return;
      }
      const endpoint = new URL(value.baseUrl.trim());
      const loopback =
        endpoint.hostname === "localhost" ||
        endpoint.hostname === "[::1]" ||
        /^127\./.test(endpoint.hostname);
      if (endpoint.protocol === "http:" && !loopback) {
        setError(
          "Use HTTPS for remote provider endpoints. HTTP is available only for loopback destinations on the Control Plane server.",
        );
        return;
      }
    }
    if (
      value.timeoutMs !== null &&
      (!Number.isSafeInteger(value.timeoutMs) ||
        value.timeoutMs < 1 ||
        value.timeoutMs > 3600000)
    ) {
      setError("Use a request timeout between 1 and 3,600,000 milliseconds.");
      return;
    }
    if (unavailable) {
      setError(
        availabilityReason ??
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
    next.labels.providers[value.profile] = label;
    onApply(
      next,
      continueToCredential && auth !== "none"
        ? {
            target: { kind: "provider", id: value.profile },
            authentication: auth,
          }
        : undefined,
    );
  }
  return (
    <>
      {error ? <p role="alert">{error}</p> : null}
      <ProviderSettingsEditor
        draft={value}
        endpointHelp="Use HTTPS for remote services. HTTP loopback addresses connect to the Control Plane server."
        busy={busy}
        profileReadOnly={Boolean(id)}
        onChange={(next) => {
          if (next.kind !== value.kind || next.baseUrl !== value.baseUrl)
            setError("");
          if (next.kind === "open_ai_codex") setAuth("shared");
          setValue(next);
        }}
        onCancel={onCancel}
        onSave={() => apply()}
        saveDisabled={unavailable}
        preset={
          <>
            <ProviderPresetPicker
              presets={presets.data?.presets ?? []}
              selected={selected?.id ?? ""}
              disabled={busy || presets.loading}
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
                setAuth(
                  preset.protocol === "codex"
                    ? "shared"
                    : preset.credentialEnv
                      ? "personal"
                      : "none",
                );
                setError("");
              }}
            />
            {presets.error ? (
              <Button disabled={busy} onClick={presets.refresh}>
                Reload providers
              </Button>
            ) : null}
            {availabilityReason ? (
              <p className="field-help" role="status">
                {availabilityReason}
              </p>
            ) : null}
          </>
        }
        authentication={
          <div className="provider-auth-grid">
            <div className="provider-credential-control">
              {codex ? (
                <div role="status">
                  <strong>ChatGPT account connection</strong>
                  <p className="field-help">
                    {availability?.status === "existing_only"
                      ? "This provider uses the deployment-managed account. Personal subscription linking requires server support."
                      : "Personal subscription linking is not available on this server. See Connected accounts for setup requirements."}
                  </p>
                  <a
                    href="https://developers.openai.com/siwc/token-sharing-open-source"
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    OpenAI connection requirements
                  </a>
                </div>
              ) : (
                <label>
                  <span>Authentication</span>
                  <DropdownSelect
                    disabled={busy}
                    aria-label="Authentication"
                    value={auth}
                    onChange={(event) =>
                      setAuth(event.target.value as Authentication)
                    }
                  >
                    <option value="personal">
                      Personal account (each user)
                    </option>
                    <option value="shared">
                      Shared credential (explicit assignment)
                    </option>
                    <option value="none">No authentication</option>
                  </DropdownSelect>
                </label>
              )}
              {!codex ? (
                <small>
                  {instanceScope
                    ? "Projects inherit this definition. Each person connects privately, or an administrator explicitly assigns a shared credential."
                    : "Each person connects their own account unless a shared credential is explicitly assigned."}
                </small>
              ) : null}
              {!codex &&
              auth !== "none" &&
              (auth === "shared" ? onCredentials : onPersonalCredentials) ? (
                <Button
                  disabled={
                    busy ||
                    unavailable ||
                    !(auth === "shared" ? onCredentials : onPersonalCredentials)
                  }
                  onClick={(event) => {
                    if (!event.currentTarget.form?.reportValidity()) return;
                    if (!destinationSaved) {
                      apply(true);
                      return;
                    }
                    if (auth === "shared")
                      onCredentials?.({ kind: "provider", id: value.profile });
                    else
                      onPersonalCredentials?.({
                        kind: "provider",
                        id: value.profile,
                      });
                  }}
                >
                  {destinationSaved
                    ? "Choose or add credential"
                    : "Save and choose credential"}
                </Button>
              ) : null}
              {!destinationSaved &&
              !codex &&
              auth !== "none" &&
              (auth === "shared" ? onCredentials : onPersonalCredentials) ? (
                <small>
                  Save this connection and continue to choose an existing
                  credential or create one.
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
                      ? "Explicit shared credential"
                      : auth === "none"
                        ? "No authentication"
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
