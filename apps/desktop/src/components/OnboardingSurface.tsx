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
import colossusMark from "../assets/colossus-mark.svg";
import { useSetupPackages } from "./setup/useSetupPackages";
import {
  ImportedProviderPicker,
  type ImportedProviderSelection,
} from "./setup/ImportedProviderPicker";
import { ImportedModelPicker } from "./setup/ImportedModelPicker";
import { importedWorkspaceConfiguration, setupChanged } from "../setupPackages";
import type { ManagedModelConfiguration } from "../types";
import { SetupPackagesPanel } from "./setup/SetupPackagesPanel";
import { SetupDesktopStep } from "./setup/SetupDesktopStep";
import { setupSteps, validProviderUrl } from "./setup/setupSteps";
import type { SetupStep } from "./setup/setupSteps";
import "./setup/setup.css";

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
import {
  configureSetupCredential,
  discoverManagedProviderModels,
  listSetupPackages,
} from "../api";
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
  onImportCaBundle: () => Promise<void>;
  onSetupStatus?: (status: DesktopStatus) => void | Promise<void>;
  dismissible: boolean;
  onCancel: () => void;
}

export function OnboardingSurface(props: OnboardingSurfaceProps) {
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [step, setStep] = useState<SetupStep>(
    props.desktop.workspace ? "provider" : "desktop",
  );
  const workspaceId = props.desktop.workspace?.workspaceId;
  const previousWorkspace = useRef(workspaceId);
  useEffect(() => {
    if (workspaceId !== previousWorkspace.current) {
      previousWorkspace.current = workspaceId;
      setStep(workspaceId ? "provider" : "workspace");
    }
  }, [workspaceId]);
  return (
    <WorkspaceOnboardingForm
      key={props.desktop.workspace?.workspaceId ?? "choose-workspace"}
      {...props}
      catalogLoading={catalogLoading}
      onCatalogLoadingChange={setCatalogLoading}
      step={step}
      onStepChange={setStep}
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
  onImportCaBundle,
  onSetupStatus,
  dismissible,
  onCancel,
  catalogLoading,
  onCatalogLoadingChange,
  step,
  onStepChange,
}: OnboardingSurfaceProps & {
  catalogLoading: boolean;
  onCatalogLoadingChange: (loading: boolean) => void;
  step: SetupStep;
  onStepChange: (step: SetupStep) => void;
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
  const { packages, loaded: packagesLoaded } = useSetupPackages();
  const [importedKey, setImportedKey] = useState<string | null>(null);
  const [credentialBusy, setCredentialBusy] = useState(false);
  const [setupError, setSetupError] = useState("");
  const importedDigest = useRef<string | null>(null);
  const initializedImport = useRef(false);
  const selectedSetup =
    packages
      .flatMap((item) =>
        item.providers.map((provider) => ({ package: item, provider })),
      )
      .find(
        (entry) =>
          entry.package.id + ":" + entry.provider.profile === importedKey,
      ) ?? null;
  function selectImportedModel(entry: ManagedModelConfiguration) {
    setModel(entry.model);
    setModelConfiguration(entry);
  }
  function selectImported(selection: ImportedProviderSelection) {
    const provider = selection.provider;
    importedDigest.current = selection.package.sha256;
    setSetupError("");
    setImportedKey(selection.package.id + ":" + provider.profile);
    setProviderKind(provider.kind);
    setBaseUrl(provider.baseUrl);
    setCredentialId(provider.credentialId);
    setNoCredential(!provider.credentialRequired);
    setReplaceCredential(false);
    const suggested =
      provider.models.find(
        (entry) => entry.profile === selection.package.roles.primary,
      ) ?? provider.models[0];
    if (suggested) selectImportedModel(suggested);
    else {
      clearModel();
      setModelConfiguration((current) => ({
        ...current,
        providerProfile: provider.profile,
      }));
    }
  }
  useEffect(() => {
    if (!packagesLoaded || initializedImport.current) return;
    // Never replace an existing workspace configuration with imported defaults.
    if (packages.length || step === "provider")
      initializedImport.current = true;
    if (desktop.provider.configured) return;
    const candidates = packages.flatMap((item) =>
      item.providers.map((provider) => ({ package: item, provider })),
    );
    const suggested =
      candidates.find((entry) =>
        entry.provider.models.some(
          (candidate) => candidate.profile === entry.package.roles.primary,
        ),
      ) ?? candidates[0];
    if (suggested) selectImported(suggested);
  }, [packagesLoaded, packages, step]);
  useEffect(() => {
    if (selectedSetup) setCredentialId(selectedSetup.provider.credentialId);
  }, [selectedSetup?.provider.credentialId, importedKey]);
  useEffect(() => {
    if (!importedKey || !packagesLoaded) return;
    if (
      selectedSetup &&
      selectedSetup.package.sha256 !== importedDigest.current
    ) {
      selectImported(selectedSetup);
      if (step !== "desktop" && step !== "workspace") onStepChange("provider");
    } else if (!selectedSetup) {
      setImportedKey(null);
      setCredentialId(null);
      setBaseUrl("");
      clearModel();
      if (step !== "desktop" && step !== "workspace") onStepChange("provider");
    }
  }, [packages, importedKey, packagesLoaded]);
  const busy = runtimeBusy || catalogLoading || credentialBusy;
  const advancedRequired = requiresAdvancedModelSetup(
    desktop.managedModelConfiguration,
  );
  const guided = !dismissible && !showAdvanced && !advancedRequired;
  const stepIndex = setupSteps.findIndex((entry) => entry.id === step);
  const currentStep = setupSteps[stepIndex]!;
  const headingRef = useRef<HTMLHeadingElement>(null);
  const previousStep = useRef(step);
  useEffect(() => {
    if (previousStep.current !== step) {
      previousStep.current = step;
      headingRef.current?.focus({ preventScroll: true });
      headingRef.current?.scrollIntoView({ block: "nearest" });
    }
  }, [step]);
  const [selfTest, setSelfTest] = useState(INITIAL_MANAGED_SELF_TEST_STATUS);
  const connectionKey = `${desktop.workspace?.workspaceId}:${providerKind}:${baseUrl}:${noCredential}:${replaceCredential}:${credentialRevision}`;
  const connectionRef = useRef(connectionKey);
  connectionRef.current = connectionKey;
  const errorRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (error === "" && setupError === "") return;
    errorRef.current?.focus({ preventScroll: true });
    errorRef.current?.scrollIntoView({ block: "nearest" });
  }, [error, setupError]);
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
  const providerReady =
    validProviderUrl(baseUrl) &&
    (providerKind !== "open_ai_codex" || codexReady);
  const modelReady =
    model.trim() !== "" &&
    Number.isInteger(modelConfiguration.contextWindowTokens) &&
    modelConfiguration.contextWindowTokens >= 1024 &&
    Number.isInteger(modelConfiguration.maxOutputTokens) &&
    modelConfiguration.maxOutputTokens >= 1;
  const canContinue =
    step === "desktop" ||
    (step === "workspace"
      ? desktop.workspace !== null
      : step === "provider"
        ? providerReady
        : providerReady && modelReady);

  function advance() {
    if (!busy && canContinue && stepIndex < setupSteps.length - 1)
      onStepChange(setupSteps[stepIndex + 1]!.id);
  }
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
    setImportedKey(null);
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
    if (busy) return;
    if (guided && step !== "start") {
      advance();
      return;
    }
    if (guided && (!providerReady || !modelReady)) return;
    if (selectedSetup && desktop.workspace) {
      setSetupError("");
      setCredentialBusy(true);
      try {
        let savedCredentialId = credentialId;
        if (selectedSetup.provider.credentialRequired && !savedCredentialId) {
          await configureSetupCredential({
            id: selectedSetup.package.id,
            sha256: selectedSetup.package.sha256,
            profile: selectedSetup.provider.profile,
          });
          const refreshed = await listSetupPackages();
          savedCredentialId =
            refreshed
              .find((entry) => entry.sha256 === selectedSetup.package.sha256)
              ?.providers.find(
                (entry) => entry.profile === selectedSetup.provider.profile,
              )?.credentialId ?? null;
          setupChanged();
          if (!savedCredentialId)
            throw new Error("The setup provider changed. Review it and retry.");
          setCredentialId(savedCredentialId);
        }
        await onApplyConfiguration(
          importedWorkspaceConfiguration(
            selectedSetup.provider,
            { ...modelConfiguration, model },
            desktop.workspace.workspaceId,
            accessProfile,
            executionBoundary,
            savedCredentialId ? "reuse" : credentialAction,
            savedCredentialId,
            selectedSetup.package.roles,
          ),
        );
      } catch (failure) {
        setSetupError(
          failure instanceof Error
            ? failure.message
            : "Could not finish provider setup. Retry when ready.",
        );
      } finally {
        setCredentialBusy(false);
      }
      return;
    }
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

  const choosingWorkspace = desktop.workspace === null;
  const installationCheck = (
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
  );

  return (
    <main
      className={`onboarding-surface${guided ? ` setup-wizard setup-wizard--${step}` : choosingWorkspace ? " onboarding-surface--welcome" : ""}`}
      id="primary-workspace"
      tabIndex={-1}
    >
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
          <img className="onboarding-mark" src={colossusMark} alt="" />
          <h1 id="onboarding-title" ref={headingRef} tabIndex={-1}>
            {guided
              ? currentStep.title
              : dismissible
                ? "Edit workspace setup"
                : choosingWorkspace
                  ? "Welcome to Colossus"
                  : "Connect a model"}
          </h1>
          <p>
            {guided
              ? currentStep.description
              : choosingWorkspace
                ? "Start with a project folder. Your models and tool settings will be saved to this workspace."
                : "Choose your provider and model, then review tool access before you start."}
          </p>
        </div>

        {guided ? (
          <ol className="setup-progress" aria-label="Setup progress">
            {setupSteps.map((entry, index) => (
              <li
                key={entry.id}
                className={
                  index === stepIndex
                    ? "is-active"
                    : index < stepIndex
                      ? "is-done"
                      : ""
                }
              >
                <button
                  type="button"
                  disabled={busy || index >= stepIndex}
                  aria-current={index === stepIndex ? "step" : undefined}
                  onClick={() => onStepChange(entry.id)}
                >
                  <span>{index + 1}</span>
                  {entry.label}
                </button>
              </li>
            ))}
          </ol>
        ) : null}

        {error !== "" ? (
          <p className="page-error" role="alert" ref={errorRef} tabIndex={-1}>
            {error || setupError}
          </p>
        ) : null}

        {(guided && step === "desktop") || !guided ? (
          <SetupPackagesPanel
            desktop={desktop}
            busy={busy}
            onStatusChange={onSetupStatus}
            onSignIn={onCodexLogin}
            compact={guided}
            onChooseProvider={(provider) => {
              setProviderKind(provider.kind);
              setBaseUrl(provider.baseUrl);
              clearModel();
              setCredentialId(provider.credentialId);
              setNoCredential(!provider.credentialRequired);
              setReplaceCredential(false);
              onStepChange("provider");
            }}
          />
        ) : null}
        {guided && step === "desktop" ? (
          <SetupDesktopStep
            busy={busy}
            certificates={desktop.additionalCaBundle}
            onImportCaBundle={onImportCaBundle}
          />
        ) : null}

        {(guided ? step === "workspace" : choosingWorkspace) ? (
          <div className="onboarding-workspace-step">
            {desktop.workspace ? (
              <div className="selected-workspace-row">
                <IconFolder size={18} aria-hidden="true" />
                <div>
                  <strong>{desktop.workspace.displayName}</strong>
                  <span>{desktop.workspace.displayPath}</span>
                </div>
              </div>
            ) : null}
            <button
              className="button primary"
              type="button"
              disabled={busy}
              aria-describedby="workspace-access-note"
              onClick={() => void onChooseWorkspace()}
            >
              <IconFolder size={19} stroke={1.6} aria-hidden="true" />
              {desktop.workspace ? "Choose another folder" : "Choose folder"}
              <IconArrowRight size={16} stroke={1.8} aria-hidden="true" />
            </button>
            <p id="workspace-access-note">
              You’ll review tool and file access before starting.
            </p>
          </div>
        ) : null}
        {desktop.workspace !== null && (showAdvanced || advancedRequired) ? (
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
                        profile:
                          selectedSetup?.provider.profile ?? "primary-provider",
                        providerKind,
                        baseUrl,
                        timeoutMs: selectedSetup?.provider.timeoutMs ?? null,
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
                    roles: { primary: modelConfiguration.profile },
                    accessProfile,
                    executionBoundary,
                  },
                }
              : {})}
            onCodexLogin={onCodexLogin}
            onCodexLogout={onCodexLogout}
          />
        ) : desktop.workspace !== null ? (
          <form
            className="provider-setup-form"
            id="setup-provider-form"
            noValidate={guided}
            hidden={guided && (step === "desktop" || step === "workspace")}
            onSubmit={(event) => void submit(event)}
          >
            {!guided ? (
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
            ) : null}

            <div
              className="setup-step-panel"
              hidden={guided && step !== "provider"}
            >
              {selectedSetup ? (
                <ImportedProviderPicker
                  packages={packages}
                  selected={selectedSetup}
                  busy={busy}
                  signedIn={codexReady}
                  onSelect={selectImported}
                  onBusyChange={setCredentialBusy}
                  onOther={() => {
                    setImportedKey(null);
                    clearModel();
                    setBaseUrl("");
                    setCredentialId(null);
                    setNoCredential(false);
                  }}
                />
              ) : null}
              {!selectedSetup && packages.length ? (
                <button
                  type="button"
                  className="text-button"
                  disabled={busy}
                  onClick={() => {
                    const item = packages[0]!;
                    selectImported({
                      package: item,
                      provider: item.providers[0]!,
                    });
                  }}
                >
                  Back to imported providers
                </button>
              ) : null}
              <div className="provider-fields" hidden={!!selectedSetup}>
                {packagesLoaded && !selectedSetup ? (
                  <ProviderPresetSelect
                    kind={providerKind}
                    baseUrl={baseUrl}
                    busy={busy}
                    {...(initialProvider === undefined &&
                    !desktop.provider.configured &&
                    packages.length === 0
                      ? { defaultPresetId: "openrouter" }
                      : {})}
                    onSelect={(preset) => {
                      if (preset.setup) {
                        const item = packages.find(
                          (item) => item.id === preset.setup!.packageId,
                        );
                        if (item) {
                          selectImported({
                            package: item,
                            provider: preset.setup.provider,
                          });
                          return;
                        }
                      }
                      setProviderKind(presetProviderKind(preset));
                      setBaseUrl(preset.baseUrl ?? "");
                      clearModel();
                      setCredentialId(
                        preset.setup?.provider.credentialId ?? null,
                      );
                      setReplaceCredential(false);
                      setNoCredential(
                        preset.credentialEnv === null &&
                          preset.baseUrl !== null,
                      );
                    }}
                  />
                ) : null}
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
                    Filled in for the selected provider. For a custom
                    connection, enter the base URL of a Chat Completions or
                    Responses API.
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
              ) : !selectedSetup && dismissible && hasSavedCredential ? (
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
              ) : !selectedSetup &&
                !hasSavedCredential &&
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

              <div className="provider-security-note" hidden={!!selectedSetup}>
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
            </div>
            <div
              className="setup-step-panel"
              hidden={guided && step !== "model"}
            >
              {selectedSetup?.provider.models.length ? (
                <ImportedModelPicker
                  selection={selectedSetup}
                  selectedProfile={
                    model.trim() !== "" && modelConfiguration.model === model
                      ? modelConfiguration.profile
                      : ""
                  }
                  busy={busy}
                  onSelect={selectImportedModel}
                />
              ) : null}
              <details
                className="imported-model-custom"
                open={!selectedSetup?.provider.models.length}
              >
                {selectedSetup?.provider.models.length ? (
                  <summary>
                    Choose a different model or edit its settings
                  </summary>
                ) : null}
                <ProviderModelPicker
                  headingLevel={2}
                  connectionKey={connectionKey}
                  model={model}
                  disabled={
                    busy ||
                    !providerReady ||
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
                      if (result.errorMessage)
                        throw new Error(result.errorMessage);
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
                              checked={
                                modelConfiguration.capabilities[capability]
                              }
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
                </div>
              </details>
            </div>
            <div
              className="setup-step-panel"
              hidden={guided && step !== "start"}
            >
              {guided ? (
                <dl className="setup-review">
                  <div>
                    <dt>Workspace</dt>
                    <dd>{desktop.workspace.displayName}</dd>
                  </div>
                  <div>
                    <dt>Provider</dt>
                    <dd>
                      {providerKind === "open_ai_codex"
                        ? "Codex · ChatGPT subscription"
                        : baseUrl}
                    </dd>
                  </div>
                  <div>
                    <dt>Model</dt>
                    <dd>{model}</dd>
                  </div>
                </dl>
              ) : null}
              <div className="provider-fields">
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
                    <option value="minimal">
                      Minimal — no workspace tools
                    </option>
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
                    <option value="workspace_isolated">
                      Workspace isolated
                    </option>
                    <option value="offline_isolated">Offline isolated</option>
                  </DropdownSelect>
                </label>
              </div>

              {executionBoundary === "full_access" ? (
                <div className="unsafe-execution-note" role="alert">
                  <IconAlertTriangle
                    size={20}
                    stroke={1.8}
                    aria-hidden="true"
                  />
                  <p>
                    <strong>Full access is unsafe.</strong> Commands can access
                    files on your computer, environment variables, and the
                    network without isolation. Approval settings still apply.
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

              {!guided ? (
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
              ) : null}
            </div>
          </form>
        ) : null}

        {guided ? (
          <div className="setup-navigation">
            {stepIndex > 0 ? (
              <button
                className="button secondary"
                type="button"
                disabled={busy}
                onClick={() => onStepChange(setupSteps[stepIndex - 1]!.id)}
              >
                Back
              </button>
            ) : null}
            {step === "start" ? (
              <button
                key="start"
                className="button primary"
                type="submit"
                form="setup-provider-form"
                disabled={busy || !canContinue}
              >
                {runtimeBusy ? "Starting…" : "Save and start"}
                <IconArrowRight size={16} aria-hidden="true" />
              </button>
            ) : (
              <button
                key="continue"
                className="button primary"
                type="button"
                disabled={busy || !canContinue}
                onClick={(event) => {
                  event.preventDefault();
                  advance();
                }}
              >
                Continue
                <IconArrowRight size={16} aria-hidden="true" />
              </button>
            )}
          </div>
        ) : null}

        {!guided && !choosingWorkspace ? installationCheck : null}

        {!guided || step === "desktop" || step === "workspace" ? (
          <>
            <div className="external-setup-row">
              <IconPlugConnected size={19} stroke={1.6} aria-hidden="true" />
              <div>
                <strong>Already have Colossus running elsewhere?</strong>
                {!guided && !choosingWorkspace ? (
                  <span>
                    Import its connection file to connect from this app.
                  </span>
                ) : null}
              </div>
              <button
                className={
                  guided || choosingWorkspace
                    ? "text-button"
                    : "button secondary"
                }
                type="button"
                disabled={busy}
                onClick={() => void onUseExternal()}
              >
                Import connection
              </button>
            </div>
            {guided || choosingWorkspace ? (
              <details className="onboarding-diagnostics">
                <summary>Check installation</summary>
                {installationCheck}
              </details>
            ) : null}
          </>
        ) : null}
      </section>
    </main>
  );
}
