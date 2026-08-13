import { useState } from "react";
import type { FormEvent } from "react";

import type {
  ApplyManagedModelConfigurationRequest,
  CredentialAction,
  DesktopStatus,
  ManagedModelConfiguration,
  ManagedProviderConfigurationInput,
  ProviderKind,
  ReasoningEffort,
} from "../types";
import {
  REMOTE_PROVIDER_TIMEOUT_MS,
  automaticProviderTimeoutMs,
} from "../providerTimeout";

const ROLES = [
  "primary",
  "risk_evaluator",
  "context_summarizer",
  "subagent_default",
  "research_planner",
  "research_worker",
  "research_synthesizer",
] as const;

const REASONING_EFFORTS: readonly ReasoningEffort[] = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
];

export type EditableProvider = ManagedProviderConfigurationInput & {
  effectiveTimeoutMs: number;
};

interface EditableConfiguration {
  providers: EditableProvider[];
  models: ManagedModelConfiguration[];
}

function defaultBaseUrl(kind: ProviderKind): string {
  if (kind === "open_ai_codex") {
    return "https://chatgpt.com/backend-api/codex";
  }
  if (kind === "openai_responses") {
    return "https://api.openai.com/v1";
  }
  return "https://openrouter.ai/api/v1";
}

function timeoutLabel(timeoutMs: number): string {
  const minutes = timeoutMs / 60_000;
  return `${Number.isInteger(minutes) ? minutes : minutes.toFixed(1)} minutes`;
}

function initialProviders(desktop: DesktopStatus): EditableProvider[] {
  if (desktop.managedModelConfiguration.providers.length === 0) {
    return [
      {
        profile: "primary-provider",
        providerKind: "openai_compatible",
        baseUrl: defaultBaseUrl("openai_compatible"),
        timeoutMs: null,
        effectiveTimeoutMs: REMOTE_PROVIDER_TIMEOUT_MS,
        credentialAction: "replace",
      },
    ];
  }
  return desktop.managedModelConfiguration.providers.map((provider) => ({
    profile: provider.profile,
    providerKind: provider.providerKind,
    baseUrl: provider.baseUrl,
    timeoutMs: provider.timeoutMs,
    effectiveTimeoutMs:
      provider.timeoutMs === null
        ? provider.effectiveTimeoutMs
        : automaticProviderTimeoutMs(provider.baseUrl),
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
        reasoningEffort: null,
        capabilities: { toolCalls: true, streaming: true },
      },
    ];
  }
  return desktop.managedModelConfiguration.models;
}

export function changeProviderProtocol(
  providers: EditableProvider[],
  models: ManagedModelConfiguration[],
  index: number,
  providerKind: ProviderKind,
): EditableConfiguration {
  const previous = providers[index];
  if (previous === undefined || previous.providerKind === providerKind) {
    return { providers, models };
  }

  return {
    providers: providers.map((provider, currentIndex) =>
      currentIndex === index
        ? {
            ...provider,
            providerKind,
            baseUrl: defaultBaseUrl(providerKind),
            credentialAction:
              providerKind === "open_ai_codex"
                ? "none"
                : provider.credentialAction,
            effectiveTimeoutMs: automaticProviderTimeoutMs(
              defaultBaseUrl(providerKind),
            ),
          }
        : provider,
    ),
    models: models.map((model) =>
      model.providerProfile === previous.profile
        ? { ...model, model: "" }
        : model,
    ),
  };
}

export async function submitModelConfiguration(
  workspaceId: string | null,
  providers: EditableProvider[],
  models: ManagedModelConfiguration[],
  roles: Record<string, string>,
  accessProfile: ApplyManagedModelConfigurationRequest["accessProfile"],
  executionBoundary: ApplyManagedModelConfigurationRequest["executionBoundary"],
  apply: (request: ApplyManagedModelConfigurationRequest) => Promise<boolean>,
): Promise<boolean> {
  if (
    workspaceId === null ||
    models.some((model) => model.model.trim() === "")
  ) {
    return false;
  }

  return apply({
    workspaceId,
    providers: providers.map(
      ({ effectiveTimeoutMs: _, ...provider }) => provider,
    ),
    models,
    roles,
    accessProfile,
    executionBoundary,
  });
}

interface ModelConfigurationEditorProps {
  desktop: DesktopStatus;
  busy: boolean;
  onApply: (request: ApplyManagedModelConfigurationRequest) => Promise<boolean>;
  onCodexLogin: () => Promise<void>;
  onCodexLogout: () => Promise<void>;
  onBack: () => void;
}

export function ModelConfigurationEditor({
  desktop,
  busy,
  onApply,
  onCodexLogin,
  onCodexLogout,
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
  const [executionBoundary, setExecutionBoundary] = useState<
    ApplyManagedModelConfigurationRequest["executionBoundary"]
  >(desktop.executionBoundary);

  function updateProvider(index: number, update: Partial<EditableProvider>) {
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
    await submitModelConfiguration(
      desktop.workspace?.workspaceId ?? null,
      providers,
      models,
      roles,
      accessProfile,
      executionBoundary,
      onApply,
    );
  }

  const providerProfiles = providers.map((provider) => provider.profile);
  const modelProfiles = models.map((model) => model.profile);
  const requiresCodexAuth = providers.some(
    (provider) => provider.providerKind === "open_ai_codex",
  );
  const codexSignedIn = desktop.codexAuth.state === "signed_in";

  return (
    <form
      className="provider-setup-form"
      onSubmit={(event) => void submit(event)}
    >
      <div className="selected-workspace-row">
        <div>
          <strong>Provider connections</strong>
          <span>
            API keys are entered only in a native prompt. ChatGPT authorization
            uses the installed Codex CLI.
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
                const next = changeProviderProtocol(
                  providers,
                  models,
                  index,
                  providerKind,
                );
                setProviders(next.providers);
                setModels(next.models);
              }}
            >
              <option value="openai_compatible">OpenAI-compatible</option>
              <option value="openai_responses">OpenAI Responses</option>
              <option value="open_ai_codex">
                ChatGPT subscription (Codex)
              </option>
            </select>
          </label>
          <label className="provider-wide-field">
            <span>Base URL</span>
            <input
              value={provider.baseUrl}
              maxLength={2048}
              required
              spellCheck={false}
              disabled={busy || provider.providerKind === "open_ai_codex"}
              onChange={(event) => {
                const baseUrl = event.target.value;
                updateProvider(index, {
                  baseUrl,
                  effectiveTimeoutMs: automaticProviderTimeoutMs(baseUrl),
                });
              }}
            />
          </label>
          <label>
            <span>Timeout</span>
            <select
              value={provider.timeoutMs === null ? "automatic" : "custom"}
              disabled={busy}
              onChange={(event) =>
                updateProvider(index, {
                  timeoutMs:
                    event.target.value === "automatic"
                      ? null
                      : provider.effectiveTimeoutMs,
                })
              }
            >
              <option value="automatic">
                Automatic · {timeoutLabel(provider.effectiveTimeoutMs)}
              </option>
              <option value="custom">Custom</option>
            </select>
          </label>
          {provider.timeoutMs !== null ? (
            <label>
              <span>Custom timeout (ms)</span>
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
          ) : null}
          {provider.providerKind === "open_ai_codex" ? (
            <div className="codex-provider-auth">
              <span>
                {desktop.codexAuth.state === "signed_in"
                  ? "ChatGPT account connected"
                  : desktop.codexAuth.message}
              </span>
              <button
                className="button secondary"
                type="button"
                disabled={busy}
                onClick={() =>
                  void (codexSignedIn ? onCodexLogout() : onCodexLogin())
                }
              >
                {codexSignedIn ? "Sign out" : "Sign in with ChatGPT"}
              </button>
            </div>
          ) : (
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
          )}
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
                timeoutMs: null,
                effectiveTimeoutMs: REMOTE_PROVIDER_TIMEOUT_MS,
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
            <span>Reasoning effort</span>
            <select
              value={model.reasoningEffort ?? "provider-default"}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  reasoningEffort:
                    event.target.value === "provider-default"
                      ? null
                      : (event.target.value as ReasoningEffort),
                })
              }
            >
              <option value="provider-default">Provider default</option>
              {REASONING_EFFORTS.map((effort) => (
                <option key={effort} value={effort}>
                  {effort === "xhigh" ? "Extra high" : effort}
                </option>
              ))}
            </select>
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
                reasoningEffort: null,
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
            <option value="allow_all">
              Allow all — every declared built-in tool
            </option>
          </select>
        </label>
        <label className="provider-wide-field">
          <span>Execution boundary</span>
          <select
            value={executionBoundary}
            disabled={busy}
            onChange={(event) =>
              setExecutionBoundary(
                event.target
                  .value as ApplyManagedModelConfigurationRequest["executionBoundary"],
              )
            }
          >
            <option value="full_access">Full access — unsafe</option>
            <option value="workspace_isolated">Workspace isolated</option>
            <option value="offline_isolated">Offline isolated</option>
          </select>
        </label>
      </fieldset>

      {executionBoundary === "full_access" ? (
        <div className="unsafe-execution-note" role="alert">
          <p>
            <strong>Unsafe: Full access.</strong> Commands can use host files,
            environment variables, and network access without Colossus
            isolation. Approval mode is configured separately.
          </p>
        </div>
      ) : null}

      <div className="provider-security-note">
        <p>
          HTTPS endpoints and loopback HTTP are accepted. New or changed
          endpoints require native confirmation. Finish or cancel active Managed
          Local runs before applying changes.
        </p>
      </div>
      <button
        className="button primary onboarding-launch"
        disabled={
          busy ||
          models.some((model) => model.model.trim() === "") ||
          (requiresCodexAuth && !codexSignedIn)
        }
      >
        {busy ? "Applying model configuration…" : "Apply model configuration"}
      </button>
    </form>
  );
}
