import { useRef, useState } from "react";
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
import { DropdownSelect } from "./DropdownSelect";
import { ProviderPresetSelect } from "./ProviderPresetSelect";
import { ProviderModelPicker } from "./ProviderModelPicker";
import {
  resetModelMetadata,
  selectCatalogModel,
  presetProviderKind,
} from "../providerCatalog";
import { discoverManagedProviderModels } from "../api";
import { requiresAdvancedModelSetup } from "../onboarding";

const ROLES = [
  "primary",
  "risk_evaluator",
  "context_summarizer",
  "subagent_default",
  "research_planner",
  "research_worker",
  "research_synthesizer",
] as const;

const ROLE_LABELS: Record<(typeof ROLES)[number], string> = {
  primary: "Primary",
  risk_evaluator: "Risk evaluator",
  context_summarizer: "Context summarizer",
  subagent_default: "Default subagent",
  research_planner: "Research planner",
  research_worker: "Research worker",
  research_synthesizer: "Research synthesizer",
};

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
  credentialRevision?: number;
  persistedCredentialProfile?: string;
};

interface EditableConfiguration {
  providers: EditableProvider[];
  models: ManagedModelConfiguration[];
}

export interface ManagedSetupDraft extends EditableConfiguration {
  roles: Record<string, string>;
  accessProfile: ApplyManagedModelConfigurationRequest["accessProfile"];
  executionBoundary: ApplyManagedModelConfigurationRequest["executionBoundary"];
}

export function renameProviderProfile(
  configuration: EditableConfiguration,
  index: number,
  profile: string,
): EditableConfiguration {
  if (
    configuration.providers.some(
      (provider, currentIndex) =>
        currentIndex !== index && provider.profile === profile,
    )
  ) {
    return configuration;
  }
  const previous = configuration.providers[index]?.profile;
  return {
    providers: configuration.providers.map((provider, currentIndex) =>
      currentIndex === index ? { ...provider, profile } : provider,
    ),
    models: configuration.models.map((model) =>
      model.providerProfile === previous
        ? { ...model, providerProfile: profile }
        : model,
    ),
  };
}

export function renameModelProfile(
  models: ManagedModelConfiguration[],
  roles: Record<string, string>,
  index: number,
  profile: string,
) {
  if (
    models.some(
      (model, currentIndex) =>
        currentIndex !== index && model.profile === profile,
    )
  ) {
    return { models, roles };
  }
  const previous = models[index]?.profile;
  return {
    models: models.map((model, currentIndex) =>
      currentIndex === index ? { ...model, profile } : model,
    ),
    roles: Object.fromEntries(
      Object.entries(roles).map(([role, model]) => [
        role,
        model === previous ? profile : model,
      ]),
    ),
  };
}

function nextProfile(prefix: string, profiles: string[]): string {
  let index = 1;
  while (profiles.includes(`${prefix}-${index}`)) index += 1;
  return `${prefix}-${index}`;
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
        baseUrl: "",
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
    ...(provider.hasCredential
      ? { persistedCredentialProfile: provider.profile }
      : {}),
  }));
}

function initialModels(desktop: DesktopStatus): ManagedModelConfiguration[] {
  if (desktop.managedModelConfiguration.models.length === 0) {
    return [
      {
        profile: "primary",
        providerProfile: "primary-provider",
        model: "",
        contextWindowTokens: 32_768,
        maxOutputTokens: 4_096,
        reasoningEffort: null,
        capabilities: {
          toolCalls: false,
          streaming: false,
          imageInputs: false,
        },
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
            baseUrl: "",
            credentialAction:
              providerKind === "open_ai_codex"
                ? "none"
                : provider.credentialAction === "reuse"
                  ? "replace"
                  : provider.credentialAction,
            effectiveTimeoutMs: REMOTE_PROVIDER_TIMEOUT_MS,
            credentialId: "",
          }
        : provider,
    ),
    models: models.map((model) =>
      model.providerProfile === previous.profile
        ? { ...resetModelMetadata(model), model: "" }
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
      ({
        effectiveTimeoutMs: _,
        credentialRevision: __,
        persistedCredentialProfile: ___,
        credentialId,
        ...provider
      }) => ({ ...provider, ...(credentialId ? { credentialId } : {}) }),
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
  onBack?: (draft: ManagedSetupDraft) => void;
  initialDraft?: ManagedSetupDraft;
  onCatalogLoadingChange?: (loading: boolean) => void;
}

export function ModelConfigurationEditor({
  desktop,
  busy: runtimeBusy,
  onApply,
  onCodexLogin,
  onCodexLogout,
  onBack,
  initialDraft,
  onCatalogLoadingChange,
}: ModelConfigurationEditorProps) {
  const [providers, setProviders] = useState(
    () => initialDraft?.providers ?? initialProviders(desktop),
  );
  const [models, setModels] = useState(
    () => initialDraft?.models ?? initialModels(desktop),
  );
  const [roles, setRoles] = useState<Record<string, string>>(() => {
    if (initialDraft) return initialDraft.roles;
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
  >(initialDraft?.accessProfile ?? desktop.accessProfile);
  const [executionBoundary, setExecutionBoundary] = useState<
    ApplyManagedModelConfigurationRequest["executionBoundary"]
  >(initialDraft?.executionBoundary ?? desktop.executionBoundary);
  const [loadingModelIndex, setLoadingModelIndex] = useState<number | null>(
    null,
  );
  const busy = runtimeBusy || loadingModelIndex !== null;
  const workspaceRef = useRef(desktop.workspace?.workspaceId);
  workspaceRef.current = desktop.workspace?.workspaceId;

  function updateProvider(index: number, update: Partial<EditableProvider>) {
    setProviders((current) =>
      current.map((provider, currentIndex) =>
        currentIndex === index
          ? {
              ...provider,
              ...update,
              credentialRevision:
                (provider.credentialRevision ?? 0) +
                ("credentialAction" in update ? 1 : 0),
            }
          : provider,
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
          <strong>Advanced model setup</strong>
          <span>
            Connect providers and choose models for different tasks. API keys
            are entered in a secure dialog and saved encrypted. Connect Codex
            with your ChatGPT account.
          </span>
        </div>
        {onBack && !requiresAdvancedModelSetup({ providers, models, roles }) ? (
          <button
            className="text-button"
            type="button"
            disabled={busy}
            onClick={() =>
              onBack({
                providers,
                models,
                roles,
                accessProfile,
                executionBoundary,
              })
            }
          >
            Back to basic setup
          </button>
        ) : null}
      </div>

      {providers.map((provider, index) => (
        <fieldset className="provider-fields" key={`provider-${index}`}>
          <legend>Provider {index + 1}</legend>
          <ProviderPresetSelect
            kind={provider.providerKind}
            baseUrl={provider.baseUrl}
            busy={busy}
            {...(index === 0 &&
            desktop.managedModelConfiguration.providers.length === 0
              ? { defaultPresetId: "openrouter" }
              : {})}
            onSelect={(preset) => {
              updateProvider(index, {
                providerKind: presetProviderKind(preset),
                baseUrl: preset.baseUrl ?? "",
                credentialAction:
                  preset.credentialEnv || preset.baseUrl === null
                    ? "replace"
                    : "none",
                credentialId: "",
                effectiveTimeoutMs: automaticProviderTimeoutMs(
                  preset.baseUrl ?? "",
                ),
              });
              setModels((current) =>
                current.map((model) =>
                  model.providerProfile === provider.profile
                    ? { ...resetModelMetadata(model), model: "" }
                    : model,
                ),
              );
            }}
          />
          <label>
            <span>Connection ID</span>
            <input
              value={provider.profile}
              maxLength={64}
              required
              spellCheck={false}
              disabled={busy}
              readOnly={
                provider.credentialAction === "reuse" &&
                provider.persistedCredentialProfile !== undefined &&
                !provider.credentialId
              }
              onChange={(event) => {
                const next = renameProviderProfile(
                  { providers, models },
                  index,
                  event.target.value,
                );
                setProviders(next.providers);
                setModels(next.models);
              }}
            />
            {provider.credentialAction === "reuse" &&
            provider.persistedCredentialProfile !== undefined &&
            !provider.credentialId ? (
              <small>
                Load models before renaming this saved connection, or choose to
                replace its API key.
              </small>
            ) : null}
          </label>
          <label>
            <span>API format</span>
            <DropdownSelect
              value={provider.providerKind}
              disabled={busy || provider.providerKind === "open_ai_codex"}
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
              <option value="openai_compatible">
                Chat Completions (OpenAI-compatible)
              </option>
              <option value="openai_responses">OpenAI Responses</option>
              <option value="open_ai_codex" disabled>
                ChatGPT subscription (Codex)
              </option>
            </DropdownSelect>
          </label>
          <label className="provider-wide-field">
            <span>API base URL</span>
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
                  credentialId: "",
                  credentialAction:
                    provider.credentialAction === "reuse"
                      ? "replace"
                      : provider.credentialAction,
                  effectiveTimeoutMs: automaticProviderTimeoutMs(baseUrl),
                });
                setModels((current) =>
                  current.map((model) =>
                    model.providerProfile === provider.profile
                      ? { ...resetModelMetadata(model), model: "" }
                      : model,
                  ),
                );
              }}
            />
          </label>
          <label>
            <span>Request timeout</span>
            <DropdownSelect
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
            </DropdownSelect>
          </label>
          {provider.timeoutMs !== null ? (
            <label>
              <span>Custom timeout (milliseconds)</span>
              <input
                type="number"
                min={1}
                value={provider.timeoutMs}
                required
                disabled={busy}
                onChange={(event) => {
                  updateProvider(index, {
                    timeoutMs: Number(event.target.value),
                  });
                }}
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
              <span>API key</span>
              <DropdownSelect
                value={provider.credentialAction}
                disabled={busy}
                onChange={(event) => {
                  if (event.target.value === provider.credentialAction) return;
                  updateProvider(index, {
                    credentialAction: event.target.value as CredentialAction,
                    credentialId: "",
                  });
                }}
              >
                <option value="none">No API key required</option>
                <option
                  value="reuse"
                  disabled={
                    provider.persistedCredentialProfile !== undefined &&
                    provider.profile !== provider.persistedCredentialProfile &&
                    !provider.credentialId
                  }
                >
                  Use saved API key
                </option>
                <option value="replace">Enter or replace API key</option>
              </DropdownSelect>
            </label>
          )}
          {providers.length > 1 ? (
            <>
              {models.some(
                (model) => model.providerProfile === provider.profile,
              ) ? (
                <p className="provider-wide-field">
                  Choose another provider for this connection’s models before
                  removing it.
                </p>
              ) : null}
              <button
                className="text-button"
                type="button"
                disabled={
                  busy ||
                  models.some(
                    (model) => model.providerProfile === provider.profile,
                  )
                }
                onClick={() =>
                  setProviders((current) =>
                    current.filter((_, currentIndex) => currentIndex !== index),
                  )
                }
              >
                Remove provider
              </button>
            </>
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
                profile: nextProfile(
                  "provider",
                  current.map((provider) => provider.profile),
                ),
                providerKind: "openai_compatible",
                baseUrl: "",
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
            <span>Model configuration ID</span>
            <input
              value={model.profile}
              maxLength={64}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) => {
                const next = renameModelProfile(
                  models,
                  roles,
                  index,
                  event.target.value,
                );
                setModels(next.models);
                setRoles(next.roles);
              }}
            />
          </label>
          <label>
            <span>Provider connection</span>
            <DropdownSelect
              value={model.providerProfile}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  ...resetModelMetadata(model),
                  providerProfile: event.target.value,
                  model: "",
                })
              }
            >
              {providerProfiles.map((profile) => (
                <option key={profile} value={profile}>
                  {profile}
                </option>
              ))}
            </DropdownSelect>
          </label>
          <div className="provider-wide-field">
            <ProviderModelPicker
              connectionKey={JSON.stringify([
                desktop.workspace?.workspaceId,
                providers
                  .filter(
                    (provider) => provider.profile === model.providerProfile,
                  )
                  .map((provider) => [
                    provider.profile,
                    provider.providerKind,
                    provider.baseUrl,
                    provider.credentialRevision,
                  ]),
              ])}
              model={model.model}
              disabled={
                busy ||
                desktop.workspace === null ||
                !providers.some(
                  (provider) =>
                    provider.profile === model.providerProfile &&
                    provider.baseUrl.trim() !== "",
                )
              }
              onLoad={async () => {
                const provider = providers.find(
                  (candidate) => candidate.profile === model.providerProfile,
                );
                if (!provider || desktop.workspace === null) return [];
                const workspaceId = desktop.workspace.workspaceId;
                setLoadingModelIndex(index);
                onCatalogLoadingChange?.(true);
                try {
                  const result = await discoverManagedProviderModels({
                    workspaceId,
                    providerProfile: provider.profile,
                    providerKind: provider.providerKind,
                    baseUrl: provider.baseUrl,
                    credentialAction: provider.credentialAction,
                    ...(provider.credentialId
                      ? { credentialId: provider.credentialId }
                      : {}),
                  });
                  if (
                    result.credentialId &&
                    workspaceRef.current === workspaceId
                  )
                    setProviders((current) =>
                      current.map((candidate) =>
                        candidate.profile === provider.profile &&
                        candidate.providerKind === provider.providerKind &&
                        candidate.baseUrl === provider.baseUrl &&
                        candidate.credentialRevision ===
                          provider.credentialRevision &&
                        candidate.credentialAction ===
                          provider.credentialAction &&
                        candidate.credentialId === provider.credentialId
                          ? {
                              ...candidate,
                              credentialId: result.credentialId!,
                              credentialAction: "reuse",
                            }
                          : candidate,
                      ),
                    );
                  if (result.errorMessage) throw new Error(result.errorMessage);
                  return result.models;
                } finally {
                  setLoadingModelIndex(null);
                  onCatalogLoadingChange?.(false);
                }
              }}
              onSelect={(entry) =>
                updateModel(index, selectCatalogModel(model, entry))
              }
            />
          </div>
          <label className="provider-wide-field">
            <span>Provider model ID</span>
            <input
              value={model.model}
              maxLength={256}
              required
              spellCheck={false}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  ...resetModelMetadata(model),
                  model: event.target.value,
                })
              }
            />
          </label>
          <label>
            <span>Context window (tokens)</span>
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
            <span>Maximum output (tokens)</span>
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
            <DropdownSelect
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
            </DropdownSelect>
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
            <span>Tool use</span>
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
          <label>
            <input
              type="checkbox"
              checked={model.capabilities.imageInputs}
              disabled={busy}
              onChange={(event) =>
                updateModel(index, {
                  capabilities: {
                    ...model.capabilities,
                    imageInputs: event.target.checked,
                  },
                })
              }
            />
            <span>Images</span>
          </label>
          {models.length > 1 ? (
            <>
              {Object.values(roles).includes(model.profile) ? (
                <p className="provider-wide-field">
                  Assign this model’s roles to another model before removing it.
                </p>
              ) : null}
              <button
                className="text-button"
                type="button"
                disabled={busy || Object.values(roles).includes(model.profile)}
                onClick={() =>
                  setModels((current) =>
                    current.filter((_, currentIndex) => currentIndex !== index),
                  )
                }
              >
                Remove model
              </button>
            </>
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
                profile: nextProfile(
                  "model",
                  current.map((model) => model.profile),
                ),
                providerProfile: providerProfiles[0] ?? "",
                model: "",
                contextWindowTokens: 32_768,
                maxOutputTokens: 4_096,
                reasoningEffort: null,
                capabilities: {
                  toolCalls: false,
                  streaming: false,
                  imageInputs: false,
                },
              },
            ])
          }
        >
          Add model
        </button>
      ) : null}

      <fieldset className="provider-fields">
        <legend>Models for each role</legend>
        {ROLES.map((role) => (
          <label key={role}>
            <span>{ROLE_LABELS[role]}</span>
            <DropdownSelect
              value={roles[role] ?? roles.primary ?? ""}
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
            </DropdownSelect>
          </label>
        ))}
        <label className="provider-wide-field">
          <span>Tool access</span>
          <DropdownSelect
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
            <option value="pinned">Custom — tools selected in Settings</option>
            <option value="development">
              Development — tools with approval checks
            </option>
            <option value="allow_all">Allow all — all built-in tools</option>
          </DropdownSelect>
        </label>
        <label className="provider-wide-field">
          <span>Command isolation</span>
          <DropdownSelect
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
          </DropdownSelect>
        </label>
      </fieldset>

      {executionBoundary === "full_access" ? (
        <div className="unsafe-execution-note" role="alert">
          <p>
            <strong>Full access is unsafe.</strong> Commands can access files on
            your computer, environment variables, and the network without
            isolation. Approval settings still apply.
          </p>
        </div>
      ) : null}

      <div className="provider-security-note">
        <p>
          Use HTTPS for remote providers. HTTP is allowed only for local servers
          on this computer. You’ll be asked to approve new or changed
          connections. Finish or cancel active tasks before saving.
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
        {runtimeBusy && loadingModelIndex === null
          ? "Please wait…"
          : "Save and start"}
      </button>
    </form>
  );
}
