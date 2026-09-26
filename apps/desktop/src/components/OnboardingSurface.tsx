import {
  IconArrowRight,
  IconAlertTriangle,
  IconCloudLock,
  IconFolder,
  IconPlugConnected,
  IconShieldCheck,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";

import type {
  ApplyManagedModelConfigurationRequest,
  ConfigureManagedRuntimeRequest,
  DesktopStatus,
} from "../types";
import {
  INITIAL_MANAGED_SELF_TEST_STATUS,
  managedProviderDefaults,
  requiresAdvancedModelSetup,
  runOfflineSelfTest,
  submitManagedRuntimeConfiguration,
} from "../onboarding";
import { DropdownSelect } from "./DropdownSelect";
import { ModelConfigurationEditor } from "./ModelConfigurationEditor";
import type { ManagedSetupDraft } from "./ModelConfigurationEditor";
import { automaticProviderTimeoutMs } from "../providerTimeout";
import { ProviderPresetSelect } from "./ProviderPresetSelect";
import { ProviderModelPicker } from "./ProviderModelPicker";
import { discoverManagedProviderModels } from "../api";
import {
  resetModelMetadata,
  selectCatalogModel,
  presetProviderKind,
} from "../providerCatalog";

interface OnboardingSurfaceProps {
  desktop: DesktopStatus;
  busy: boolean;
  error: string;
  onChooseWorkspace: () => Promise<void>;
  onConfigure: (request: ConfigureManagedRuntimeRequest) => Promise<boolean>;
  onApplyConfiguration: (
    request: ApplyManagedModelConfigurationRequest,
  ) => Promise<boolean>;
  onRunSelfTest: () => Promise<void>;
  onCodexLogin: () => Promise<void>;
  onCodexLogout: () => Promise<void>;
  onUseExternal: () => Promise<void>;
  dismissible: boolean;
  onCancel: () => void;
}

export function OnboardingSurface(props: OnboardingSurfaceProps) {
  const [catalogLoading, setCatalogLoading] = useState(false);
  return (
    <WorkspaceOnboardingForm
      key={props.desktop.workspace?.workspaceId ?? "choose-workspace"}
      {...props}
      catalogLoading={catalogLoading}
      onCatalogLoadingChange={setCatalogLoading}
    />
  );
}

function WorkspaceOnboardingForm({
  desktop,
  busy: runtimeBusy,
  error,
  onChooseWorkspace,
  onConfigure,
  onApplyConfiguration,
  onRunSelfTest,
  onCodexLogin,
  onCodexLogout,
  onUseExternal,
  dismissible,
  onCancel,
  catalogLoading,
  onCatalogLoadingChange,
}: OnboardingSurfaceProps & {
  catalogLoading: boolean;
  onCatalogLoadingChange: (loading: boolean) => void;
}) {
  const initialModel = desktop.managedModelConfiguration.models.find(
    (candidate) =>
      candidate.profile === desktop.managedModelConfiguration.roles.primary,
  );
  const initialProvider = desktop.managedModelConfiguration.providers.find(
    (candidate) => candidate.profile === initialModel?.providerProfile,
  );
  const initialProviderKind =
    desktop.provider.configured && desktop.provider.kind !== null
      ? desktop.provider.kind
      : "openai_compatible";
  const [providerKind, setProviderKind] =
    useState<ConfigureManagedRuntimeRequest["providerKind"]>(
      initialProviderKind,
    );
  const [model, setModel] = useState(
    desktop.provider.configured
      ? desktop.provider.model
      : managedProviderDefaults(initialProviderKind).model,
  );
  const [baseUrl, setBaseUrl] = useState(initialProvider?.baseUrl ?? "");
  const [credentialId, setCredentialId] = useState<string | null>(null);
  const [noCredential, setNoCredential] = useState(
    initialProvider ? !initialProvider.hasCredential : false,
  );
  const [modelConfiguration, setModelConfiguration] = useState({
    profile: "primary",
    providerProfile: "primary-provider",
    model: initialModel?.model ?? "",
    contextWindowTokens: initialModel?.contextWindowTokens ?? 32_768,
    maxOutputTokens: initialModel?.maxOutputTokens ?? 4_096,
    reasoningEffort: initialModel?.reasoningEffort ?? null,
    capabilities: initialModel?.capabilities ?? {
      toolCalls: false,
      streaming: false,
      imageInputs: false,
    },
  });
  const [accessProfile, setAccessProfile] = useState<
    ConfigureManagedRuntimeRequest["accessProfile"]
  >(desktop.accessProfile);
  const [executionBoundary, setExecutionBoundary] = useState<
    ConfigureManagedRuntimeRequest["executionBoundary"]
  >(desktop.executionBoundary);
  const [replaceCredential, setReplaceCredential] = useState(false);
  const [credentialRevision, setCredentialRevision] = useState(0);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const busy = runtimeBusy || catalogLoading;
  const advancedRequired = requiresAdvancedModelSetup(
    desktop.managedModelConfiguration,
  );
  const [selfTest, setSelfTest] = useState(INITIAL_MANAGED_SELF_TEST_STATUS);
  const connectionKey = `${desktop.workspace?.workspaceId}:${providerKind}:${baseUrl}:${noCredential}:${replaceCredential}:${credentialRevision}`;
  const connectionRef = useRef(connectionKey);
  connectionRef.current = connectionKey;
  const errorRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (error === "") return;
    errorRef.current?.focus({ preventScroll: true });
    errorRef.current?.scrollIntoView({ block: "nearest" });
  }, [error]);
  function clearModel() {
    setModel("");
    setModelConfiguration((current) => ({
      ...resetModelMetadata(current),
      model: "",
    }));
  }
  const providerChanged =
    desktop.provider.configured &&
    (desktop.provider.kind !== providerKind ||
      (initialProvider !== undefined && initialProvider.baseUrl !== baseUrl));
  const hasSavedCredential =
    !providerChanged && initialProvider?.hasCredential === true;
  const providerPromptRequired =
    providerKind !== "open_ai_codex" &&
    !noCredential &&
    credentialId === null &&
    (!hasSavedCredential || replaceCredential);
  const codexReady = desktop.codexAuth.state === "signed_in";
  const credentialAction =
    noCredential || providerKind === "open_ai_codex"
      ? "none"
      : credentialId !== null || (hasSavedCredential && !replaceCredential)
        ? "reuse"
        : "replace";

  function returnToSimpleSetup(draft: ManagedSetupDraft) {
    const provider = draft.providers[0];
    const selectedModel = draft.models[0];
    if (!provider || !selectedModel) return;
    setProviderKind(provider.providerKind);
    setBaseUrl(provider.baseUrl);
    setCredentialId(provider.credentialId || null);
    setNoCredential(provider.credentialAction === "none");
    setReplaceCredential(provider.credentialAction === "replace");
    setModel(selectedModel.model);
    setModelConfiguration(selectedModel);
    setAccessProfile(draft.accessProfile);
    setExecutionBoundary(draft.executionBoundary);
    setShowAdvanced(false);
  }
  const accessRank = {
    minimal: 0,
    pinned: 0,
    development: 1,
    allow_all: 2,
  } as const;
  const boundaryRank = {
    offline_isolated: 0,
    workspace_isolated: 1,
    full_access: 2,
  } as const;
  const accessConfirmationRequired =
    (!desktop.provider.configured && accessProfile !== "minimal") ||
    accessRank[accessProfile] > accessRank[desktop.accessProfile];
  const boundaryConfirmationRequired =
    (!desktop.provider.configured &&
      executionBoundary !== "offline_isolated") ||
    boundaryRank[executionBoundary] > boundaryRank[desktop.executionBoundary];

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await submitManagedRuntimeConfiguration(
      desktop.workspace,
      {
        providerKind,
        model,
        accessProfile,
        executionBoundary,
        replaceCredential: credentialId === null && replaceCredential,
        baseUrl,
        ...(credentialId !== null ? { credentialId } : {}),
        noCredential,
        modelMetadata: {
          contextWindowTokens: modelConfiguration.contextWindowTokens,
          maxOutputTokens: modelConfiguration.maxOutputTokens,
          ...modelConfiguration.capabilities,
        },
      },
      onConfigure,
    );
  }

  return (
    <main className="onboarding-surface" id="primary-workspace" tabIndex={-1}>
      <section className="onboarding-card" aria-labelledby="onboarding-title">
        {dismissible ? (
          <div className="onboarding-cancel-row">
            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={onCancel}
            >
              Cancel
            </button>
          </div>
        ) : null}
        <div className="onboarding-heading">
          <span className="onboarding-mark" aria-hidden="true">
            <IconShieldCheck size={28} stroke={1.5} />
          </span>
          <p className="eyebrow">Setup</p>
          <h1 id="onboarding-title">
            {dismissible ? "Edit workspace setup" : "Set up Colossus"}
          </h1>
          <p>
            Choose a folder, connect a model provider, and review tool access.
            Your settings are saved for this workspace.
          </p>
        </div>

        <ol className="onboarding-steps" aria-label="Setup progress">
          <li className={desktop.workspace === null ? "is-active" : "is-done"}>
            <span>1</span>
            Workspace
          </li>
          <li className={desktop.workspace === null ? "" : "is-active"}>
            <span>2</span>
            Model
          </li>
          <li>
            <span>3</span>
            Start
          </li>
        </ol>

        {error !== "" ? (
          <p className="page-error" role="alert" ref={errorRef} tabIndex={-1}>
            {error}
          </p>
        ) : null}

        {desktop.workspace === null ? (
          <>
            <div className="onboarding-workspace-step">
              <span className="setup-icon" aria-hidden="true">
                <IconFolder size={25} stroke={1.5} />
              </span>
              <div>
                <h2>Choose your project folder</h2>
                <p>
                  This folder becomes your workspace. Before you start, you can
                  choose which tools Colossus can use and whether commands can
                  access files outside this folder.
                </p>
              </div>
              <button
                className="button primary"
                type="button"
                disabled={busy}
                onClick={() => void onChooseWorkspace()}
              >
                Choose folder
                <IconArrowRight size={16} stroke={1.8} aria-hidden="true" />
              </button>
            </div>
          </>
        ) : showAdvanced || advancedRequired ? (
          <ModelConfigurationEditor
            desktop={desktop}
            busy={busy}
            onCatalogLoadingChange={onCatalogLoadingChange}
            onApply={onApplyConfiguration}
            {...(!advancedRequired
              ? {
                  onBack: returnToSimpleSetup,
                  initialDraft: {
                    providers: [
                      {
                        profile: "primary-provider",
                        providerKind,
                        baseUrl,
                        timeoutMs: null,
                        effectiveTimeoutMs: automaticProviderTimeoutMs(baseUrl),
                        credentialAction,
                        ...(initialProvider?.hasCredential
                          ? {
                              persistedCredentialProfile:
                                initialProvider.profile,
                            }
                          : {}),
                        ...(credentialId ? { credentialId } : {}),
                      },
                    ],
                    models: [{ ...modelConfiguration, model }],
                    roles: { primary: "primary" },
                    accessProfile,
                    executionBoundary,
                  },
                }
              : {})}
            onCodexLogin={onCodexLogin}
            onCodexLogout={onCodexLogout}
          />
        ) : (
          <form
            className="provider-setup-form"
            onSubmit={(event) => void submit(event)}
          >
            <div className="selected-workspace-row">
              <IconFolder size={18} stroke={1.6} aria-hidden="true" />
              <div>
                <strong>{desktop.workspace.displayName}</strong>
                <span>{desktop.workspace.displayPath}</span>
              </div>
              <button
                className="text-button"
                type="button"
                disabled={busy}
                onClick={() => void onChooseWorkspace()}
              >
                Choose another folder
              </button>
            </div>

            <div className="provider-fields">
              <ProviderPresetSelect
                kind={providerKind}
                baseUrl={baseUrl}
                busy={busy}
                {...(initialProvider === undefined &&
                !desktop.provider.configured
                  ? { defaultPresetId: "openrouter" }
                  : {})}
                onSelect={(preset) => {
                  setProviderKind(presetProviderKind(preset));
                  setBaseUrl(preset.baseUrl ?? "");
                  clearModel();
                  setCredentialId(null);
                  setReplaceCredential(false);
                  setNoCredential(
                    preset.credentialEnv === null && preset.baseUrl !== null,
                  );
                }}
              />
              <label>
                <span>API format</span>
                <DropdownSelect
                  value={providerKind}
                  disabled={busy || providerKind === "open_ai_codex"}
                  onChange={(event) => {
                    const kind = event.target
                      .value as ConfigureManagedRuntimeRequest["providerKind"];
                    if (kind === providerKind) return;
                    setProviderKind(kind);
                    clearModel();
                    setCredentialId(null);
                    setReplaceCredential(false);
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
                  type="url"
                  aria-label="API base URL"
                  value={baseUrl}
                  required
                  disabled={busy || providerKind === "open_ai_codex"}
                  placeholder="https://your-provider.example/v1"
                  onChange={(event) => {
                    setBaseUrl(event.target.value);
                    clearModel();
                    setCredentialId(null);
                  }}
                />
                <small>
                  Filled in for the selected provider. For a custom connection,
                  enter the base URL of a Chat Completions or Responses API.
                </small>
              </label>
              {providerKind !== "open_ai_codex" ? (
                <label className="provider-wide-field provider-credential-toggle">
                  <input
                    type="checkbox"
                    checked={noCredential}
                    disabled={busy}
                    onChange={(event) => {
                      setNoCredential(event.target.checked);
                      setCredentialId(null);
                    }}
                  />
                  <span>Connect without an API key</span>
                </label>
              ) : null}
            </div>
            {providerKind === "open_ai_codex" ? (
              <div className="provider-security-note codex-auth-row">
                <IconCloudLock size={19} stroke={1.6} aria-hidden="true" />
                <p>
                  <strong>
                    {codexReady
                      ? "ChatGPT connected"
                      : "ChatGPT sign-in required"}
                  </strong>{" "}
                  {desktop.codexAuth.message}
                </p>
                <button
                  className="button secondary"
                  type="button"
                  disabled={busy}
                  onClick={() =>
                    void (codexReady ? onCodexLogout() : onCodexLogin())
                  }
                >
                  {codexReady ? "Sign out" : "Sign in with ChatGPT"}
                </button>
              </div>
            ) : dismissible && hasSavedCredential ? (
              <label className="provider-credential-toggle">
                <input
                  type="checkbox"
                  checked={replaceCredential}
                  disabled={busy}
                  onChange={(event) => {
                    setCredentialId(null);
                    setReplaceCredential(event.target.checked);
                  }}
                />
                <span>
                  <strong>Use a different API key</strong>
                  <small>
                    Leave this off to keep using your saved API key.
                  </small>
                </span>
              </label>
            ) : !hasSavedCredential &&
              credentialId !== null &&
              !noCredential ? (
              <button
                className="text-button"
                type="button"
                disabled={busy}
                onClick={() => {
                  setCredentialId(null);
                  setReplaceCredential(true);
                  setCredentialRevision((current) => current + 1);
                }}
              >
                Use a different API key
              </button>
            ) : null}

            <div className="provider-security-note">
              <IconCloudLock size={19} stroke={1.6} aria-hidden="true" />
              <p>
                {providerKind === "open_ai_codex"
                  ? "Connect through Codex using your ChatGPT account. You’ll be asked to confirm the provider address before loading models."
                  : noCredential
                    ? "Use this only if your provider allows connections without an API key, such as a local model server. You’ll be asked to confirm the provider address before loading models."
                    : providerPromptRequired
                      ? "When you load models or save, you’ll confirm the provider address and enter your API key in a separate secure window. Colossus saves the key encrypted on this computer."
                      : "Your saved API key will be reused. You’ll be asked to confirm the provider address before loading models."}
              </p>
            </div>

            <ProviderModelPicker
              connectionKey={connectionKey}
              model={model}
              disabled={
                busy ||
                baseUrl.trim() === "" ||
                (providerKind === "open_ai_codex" && !codexReady)
              }
              onLoad={async () => {
                const requestedConnection = connectionKey;
                onCatalogLoadingChange(true);
                try {
                  const result = await discoverManagedProviderModels({
                    workspaceId: desktop.workspace!.workspaceId,
                    providerKind,
                    baseUrl,
                    ...(initialProvider && !providerChanged
                      ? { providerProfile: initialProvider.profile }
                      : {}),
                    credentialAction,
                    ...(credentialId !== null ? { credentialId } : {}),
                  });
                  if (connectionRef.current === requestedConnection)
                    setCredentialId(result.credentialId);
                  if (result.errorMessage) throw new Error(result.errorMessage);
                  return result.models;
                } finally {
                  onCatalogLoadingChange(false);
                }
              }}
              onSelect={(entry) => {
                setModel(entry.id);
                setModelConfiguration((current) =>
                  selectCatalogModel(current, entry),
                );
              }}
            />
            <div className="provider-fields">
              <label>
                <span>Model ID</span>
                <input
                  value={model}
                  maxLength={256}
                  required
                  spellCheck={false}
                  disabled={busy}
                  onChange={(event) => {
                    setModel(event.target.value);
                    setModelConfiguration((current) => ({
                      ...resetModelMetadata(current),
                      model: event.target.value,
                    }));
                  }}
                />
              </label>
              <details className="provider-wide-field provider-model-overrides">
                <summary>Model limits and capabilities</summary>
                <p>
                  Check these values against your provider’s model details.
                  Enable only the features your model supports.
                </p>
                <div className="provider-fields">
                  <label>
                    <span>Context window (tokens)</span>
                    <input
                      type="number"
                      min={1024}
                      disabled={busy}
                      value={modelConfiguration.contextWindowTokens}
                      onChange={(event) =>
                        setModelConfiguration((current) => ({
                          ...current,
                          contextWindowTokens: Number(event.target.value),
                        }))
                      }
                    />
                  </label>
                  <label>
                    <span>Maximum output (tokens)</span>
                    <input
                      type="number"
                      min={1}
                      disabled={busy}
                      value={modelConfiguration.maxOutputTokens}
                      onChange={(event) =>
                        setModelConfiguration((current) => ({
                          ...current,
                          maxOutputTokens: Number(event.target.value),
                        }))
                      }
                    />
                  </label>
                  {(["toolCalls", "streaming", "imageInputs"] as const).map(
                    (capability) => (
                      <label
                        className="provider-credential-toggle"
                        key={capability}
                      >
                        <input
                          type="checkbox"
                          checked={modelConfiguration.capabilities[capability]}
                          disabled={busy}
                          onChange={(event) =>
                            setModelConfiguration((current) => ({
                              ...current,
                              capabilities: {
                                ...current.capabilities,
                                [capability]: event.target.checked,
                              },
                            }))
                          }
                        />
                        <span>
                          {capability === "toolCalls"
                            ? "Tools"
                            : capability === "imageInputs"
                              ? "Images"
                              : "Streaming"}
                        </span>
                      </label>
                    ),
                  )}
                </div>
              </details>
              <label className="provider-wide-field">
                <span>Tool access</span>
                <DropdownSelect
                  value={accessProfile}
                  disabled={busy}
                  onChange={(event) =>
                    setAccessProfile(
                      event.target
                        .value as ConfigureManagedRuntimeRequest["accessProfile"],
                    )
                  }
                >
                  <option value="minimal">Minimal — no workspace tools</option>
                  <option value="pinned">
                    Custom — tools selected in Settings
                  </option>
                  <option value="development">
                    Development — tools with approval checks
                  </option>
                  <option value="allow_all">
                    Allow all — all built-in tools
                  </option>
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
                        .value as ConfigureManagedRuntimeRequest["executionBoundary"],
                    )
                  }
                >
                  <option value="full_access">Full access — unsafe</option>
                  <option value="workspace_isolated">Workspace isolated</option>
                  <option value="offline_isolated">Offline isolated</option>
                </DropdownSelect>
              </label>
            </div>

            {executionBoundary === "full_access" ? (
              <div className="unsafe-execution-note" role="alert">
                <IconAlertTriangle size={20} stroke={1.8} aria-hidden="true" />
                <p>
                  <strong>Full access is unsafe.</strong> Commands can access
                  files on your computer, environment variables, and the network
                  without isolation. Approval settings still apply.
                </p>
              </div>
            ) : null}

            {accessConfirmationRequired || boundaryConfirmationRequired ? (
              <div className="provider-security-note">
                <IconShieldCheck size={19} stroke={1.6} aria-hidden="true" />
                <p>
                  {accessConfirmationRequired
                    ? "You’ll be asked to confirm this level of tool access before it takes effect. "
                    : ""}
                  {boundaryConfirmationRequired
                    ? "You’ll confirm the command isolation setting in a separate window."
                    : ""}
                </p>
              </div>
            ) : null}

            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() => setShowAdvanced(true)}
            >
              Advanced model setup
            </button>

            <button
              className="button primary onboarding-launch"
              type="submit"
              disabled={
                busy ||
                baseUrl.trim() === "" ||
                model.trim() === "" ||
                (providerKind === "open_ai_codex" && !codexReady)
              }
            >
              {runtimeBusy ? "Please wait…" : "Save and start"}
              <IconArrowRight size={16} stroke={1.8} aria-hidden="true" />
            </button>
          </form>
        )}

        <div className="offline-self-test-row">
          <IconShieldCheck size={19} stroke={1.6} aria-hidden="true" />
          <div>
            <strong>Check your installation</strong>
            <span>
              Runs an offline check without setting up a provider or running a
              model. No API key is needed.
            </span>
            {selfTest.message !== "" ? (
              <span
                className={`offline-self-test-status is-${selfTest.state}`}
                role={selfTest.state === "failed" ? "alert" : "status"}
              >
                {selfTest.message}
              </span>
            ) : null}
          </div>
          <button
            className="button secondary"
            type="button"
            disabled={busy || selfTest.state === "running"}
            onClick={() => void runOfflineSelfTest(onRunSelfTest, setSelfTest)}
          >
            {selfTest.state === "running"
              ? "Checking…"
              : selfTest.state === "passed"
                ? "Run again"
                : "Run check"}
          </button>
        </div>

        <div className="external-setup-row">
          <IconPlugConnected size={19} stroke={1.6} aria-hidden="true" />
          <div>
            <strong>Already have Colossus running elsewhere?</strong>
            <span>Import its connection file to connect from this app.</span>
          </div>
          <button
            className="button secondary"
            type="button"
            disabled={busy}
            onClick={() => void onUseExternal()}
          >
            Import connection
          </button>
        </div>
      </section>
    </main>
  );
}
