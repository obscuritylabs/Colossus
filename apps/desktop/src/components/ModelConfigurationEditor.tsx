import { useState } from "react";
import type { FormEvent } from "react";

import type {
  ApplyManagedModelConfigurationRequest,
  CredentialAction,
  DesktopStatus,
  ManagedModelConfiguration,
  ManagedProviderConfigurationInput,
  ProviderKind,
} from "../types";

const ROLES = [
  "primary",
  "risk_evaluator",
  "context_summarizer",
  "subagent_default",
  "research_planner",
  "research_worker",
  "research_synthesizer",
] as const;

function defaultBaseUrl(kind: ProviderKind): string {
  return kind === "openai_responses"
    ? "https://api.openai.com/v1"
    : "https://openrouter.ai/api/v1";
}

function initialProviders(
  desktop: DesktopStatus,
): ManagedProviderConfigurationInput[] {
  if (desktop.managedModelConfiguration.providers.length === 0) {
    return [
      {
        profile: "primary-provider",
        providerKind: "openai_compatible",
        baseUrl: defaultBaseUrl("openai_compatible"),
        timeoutMs: 120_000,
        credentialAction: "replace",
      },
    ];
  }
  return desktop.managedModelConfiguration.providers.map((provider) => ({
    profile: provider.profile,
    providerKind: provider.providerKind,
    baseUrl: provider.baseUrl,
    timeoutMs: provider.timeoutMs,
    credentialAction: provider.hasCredential ? "reuse" : "none",
  }));
}

function initialModels(desktop: DesktopStatus): ManagedModelConfiguration[] {
  if (desktop.managedModelConfiguration.models.length === 0) {
    return [
      {
        profile: "primary",
        providerProfile: "primary-provider",
        model: "deepseek/deepseek-v4-flash",
        contextWindowTokens: 128_000,
        maxOutputTokens: 16_000,
        capabilities: { toolCalls: true, streaming: true },
      },
    ];
  }
  return desktop.managedModelConfiguration.models;
}

interface ModelConfigurationEditorProps {
  desktop: DesktopStatus;
  busy: boolean;
  onApply: (request: ApplyManagedModelConfigurationRequest) => Promise<boolean>;
  onBack: () => void;
}

export function ModelConfigurationEditor({
  desktop,
  busy,
  onApply,
  onBack,
}: ModelConfigurationEditorProps) {
  const [providers, setProviders] = useState(() => initialProviders(desktop));
  const [models, setModels] = useState(() => initialModels(desktop));
  const [roles, setRoles] = useState<Record<string, string>>(() => {
    const primary =
      desktop.managedModelConfiguration.roles.primary ??
      initialModels(desktop)[0]?.profile ??
      "";
    return Object.fromEntries(
      ROLES.map((role) => [
        role,
        desktop.managedModelConfiguration.roles[role] ?? primary,
      ]),
    );
  });
  const [accessProfile, setAccessProfile] = useState<
    ApplyManagedModelConfigurationRequest["accessProfile"]
  >(desktop.accessProfile);

  function updateProvider(
    index: number,
    update: Partial<ManagedProviderConfigurationInput>,
  ) {
    setProviders((current) =>
      current.map((provider, currentIndex) =>
        currentIndex === index ? { ...provider, ...update } : provider,
      ),
    );
  }

  function updateModel(
    index: number,
    update: Partial<ManagedModelConfiguration>,
  ) {
    setModels((current) =>
      current.map((model, currentIndex) =>
        currentIndex === index ? { ...model, ...update } : model,
      ),
    );
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (desktop.workspace === null) {
      return;
    }
    await onApply({
      workspaceId: desktop.workspace.workspaceId,
      providers,
      models,
      roles,
      accessProfile,
    });
  }

  const providerProfiles = providers.map((provider) => provider.profile);
  const modelProfiles = models.map((model) => model.profile);

  return (
    <form
      className="provider-setup-form"
      onSubmit={(event) => void submit(event)}
    >
      <div className="selected-workspace-row">
        <div>
          <strong>Provider connections</strong>
          <span>
            Credentials are optional and are entered only in a native prompt.
          </span>
        </div>
        <button
          className="text-button"
          type="button"
          disabled={busy}
          onClick={onBack}
        >
          Simple setup
        </button>
      </div>

      {providers.map((provider, index) => (
        <fieldset className="provider-fields" key={`provider-${index}`}>
          <legend>Provider {index + 1}</legend>
          <label>
            <span>Profile</span>
            <input
              value={provider.profile}
              maxLength={64}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) =>
                updateProvider(index, { profile: event.target.value })
              }
            />
          </label>
          <label>
            <span>Protocol</span>
            <select
              value={provider.providerKind}
              disabled={busy}
              onChange={(event) => {
                const providerKind = event.target.value as ProviderKind;
                updateProvider(index, {
                  providerKind,
                  baseUrl: defaultBaseUrl(providerKind),
                });
              }}
            >
              <option value="openai_compatible">OpenAI-compatible</option>
              <option value="openai_responses">OpenAI Responses</option>
            </select>
          </label>
          <label className="provider-wide-field">
            <span>Base URL</span>
            <input
              value={provider.baseUrl}
              maxLength={2048}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) =>
                updateProvider(index, { baseUrl: event.target.value })
              }
            />
          </label>
          <label>
            <span>Timeout (ms)</span>
            <input
              type="number"
              min={1}
              value={provider.timeoutMs}
              required
              disabled={busy}
              onChange={(event) =>
                updateProvider(index, {
                  timeoutMs: Number(event.target.value),
                })
              }
            />
          </label>
          <label>
            <span>Credential</span>
            <select
              value={provider.credentialAction}
              disabled={busy}
              onChange={(event) =>
                updateProvider(index, {
                  credentialAction: event.target.value as CredentialAction,
                })
              }
            >
              <option value="none">No credential</option>
              <option value="reuse">Reuse stored credential</option>
              <option value="replace">Enter or replace natively</option>
            </select>
          </label>
          {providers.length > 1 ? (
            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() =>
                setProviders((current) =>
                  current.filter((_, currentIndex) => currentIndex !== index),
                )
              }
            >
              Remove provider
            </button>
          ) : null}
        </fieldset>
      ))}
      {providers.length < 16 ? (
        <button
          className="button secondary"
          type="button"
          disabled={busy}
          onClick={() =>
            setProviders((current) => [
              ...current,
              {
                profile: `provider-${current.length + 1}`,
                providerKind: "openai_compatible",
                baseUrl: defaultBaseUrl("openai_compatible"),
                timeoutMs: 120_000,
                credentialAction: "none",
              },
            ])
          }
        >
          Add provider
        </button>
      ) : null}

      {models.map((model, index) => (
        <fieldset className="provider-fields" key={`model-${index}`}>
          <legend>Model {index + 1}</legend>
          <label>
            <span>Profile</span>
            <input
              value={model.profile}
              maxLength={64}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, { profile: event.target.value })
              }
            />
          </label>
          <label>
            <span>Provider profile</span>
            <select
              value={model.providerProfile}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, { providerProfile: event.target.value })
              }
            >
              {providerProfiles.map((profile) => (
                <option key={profile} value={profile}>
                  {profile}
                </option>
              ))}
            </select>
          </label>
          <label className="provider-wide-field">
            <span>Provider model ID</span>
            <input
              value={model.model}
              maxLength={256}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, { model: event.target.value })
              }
            />
          </label>
          <label>
            <span>Context window</span>
            <input
              type="number"
              min={1024}
              value={model.contextWindowTokens}
              required
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  contextWindowTokens: Number(event.target.value),
                })
              }
            />
          </label>
          <label>
            <span>Maximum output</span>
            <input
              type="number"
              min={1}
              value={model.maxOutputTokens}
              required
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  maxOutputTokens: Number(event.target.value),
                })
              }
            />
          </label>
          <label>
            <input
              type="checkbox"
              checked={model.capabilities.toolCalls}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  capabilities: {
                    ...model.capabilities,
                    toolCalls: event.target.checked,
                  },
                })
              }
            />
            <span>Tool calls</span>
          </label>
          <label>
            <input
              type="checkbox"
              checked={model.capabilities.streaming}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  capabilities: {
                    ...model.capabilities,
                    streaming: event.target.checked,
                  },
                })
              }
            />
            <span>Streaming</span>
          </label>
          {models.length > 1 ? (
            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() =>
                setModels((current) =>
                  current.filter((_, currentIndex) => currentIndex !== index),
                )
              }
            >
              Remove model
            </button>
          ) : null}
        </fieldset>
      ))}
      {models.length < 64 ? (
        <button
          className="button secondary"
          type="button"
          disabled={busy}
          onClick={() =>
            setModels((current) => [
              ...current,
              {
                profile: `model-${current.length + 1}`,
                providerProfile: providerProfiles[0] ?? "",
                model: "",
                contextWindowTokens: 128_000,
                maxOutputTokens: 16_000,
                capabilities: { toolCalls: true, streaming: true },
              },
            ])
          }
        >
          Add model
        </button>
      ) : null}

      <fieldset className="provider-fields">
        <legend>Role routing</legend>
        {ROLES.map((role) => (
          <label key={role}>
            <span>{role.replaceAll("_", " ")}</span>
            <select
              value={roles[role] ?? ""}
              disabled={busy}
              onChange={(event) =>
                setRoles((current) => ({
                  ...current,
                  [role]: event.target.value,
                }))
              }
            >
              {modelProfiles.map((profile) => (
                <option key={profile} value={profile}>
                  {profile}
                </option>
              ))}
            </select>
          </label>
        ))}
        <label className="provider-wide-field">
          <span>Access profile</span>
          <select
            value={accessProfile}
            disabled={busy}
            onChange={(event) =>
              setAccessProfile(
                event.target
                  .value as ApplyManagedModelConfigurationRequest["accessProfile"],
              )
            }
          >
            <option value="minimal">Minimal — no workspace tools</option>
            <option value="development">
              Development — approval-gated effects
            </option>
          </select>
        </label>
      </fieldset>

      <div className="provider-security-note">
        <p>
          HTTPS endpoints and loopback HTTP are accepted. New or changed
          endpoints require native confirmation. Finish or cancel active Managed
          Local runs before applying changes.
        </p>
      </div>
      <button className="button primary onboarding-launch" disabled={busy}>
        {busy ? "Applying model configuration…" : "Apply model configuration"}
      </button>
    </form>
  );
}
