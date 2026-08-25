import {
  IconArrowRight,
  IconAlertTriangle,
  IconCloudLock,
  IconFolder,
  IconPlugConnected,
  IconShieldCheck,
} from "@tabler/icons-react";
import { useState } from "react";
import type { FormEvent } from "react";

import type {
  ApplyManagedModelConfigurationRequest,
  ConfigureManagedRuntimeRequest,
  DesktopStatus,
} from "../types";
import {
  INITIAL_MANAGED_SELF_TEST_STATUS,
  managedProviderDefaults,
  runOfflineSelfTest,
  submitManagedRuntimeConfiguration,
} from "../onboarding";
import { DropdownSelect } from "./DropdownSelect";
import { ModelConfigurationEditor } from "./ModelConfigurationEditor";

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

export function OnboardingSurface({
  desktop,
  busy,
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
}: OnboardingSurfaceProps) {
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
  const [accessProfile, setAccessProfile] = useState<
    ConfigureManagedRuntimeRequest["accessProfile"]
  >(desktop.accessProfile);
  const [executionBoundary, setExecutionBoundary] = useState<
    ConfigureManagedRuntimeRequest["executionBoundary"]
  >(desktop.executionBoundary);
  const [replaceCredential, setReplaceCredential] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [selfTest, setSelfTest] = useState(INITIAL_MANAGED_SELF_TEST_STATUS);
  const providerChanged =
    desktop.provider.configured && desktop.provider.kind !== providerKind;
  const providerPromptRequired =
    providerKind !== "open_ai_codex" &&
    (!desktop.provider.configured || providerChanged || replaceCredential);
  const codexReady = desktop.codexAuth.state === "signed_in";
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
        replaceCredential,
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
          <p className="eyebrow">Managed Local</p>
          <h1 id="onboarding-title">
            {dismissible
              ? "Configure Managed Local"
              : "Start Colossus in this app"}
          </h1>
          <p>
            Add a folder-backed Workspace, then choose its provider and
            execution boundary. The app supervises a local runtime while
            credentials remain behind native storage boundaries.
          </p>
        </div>

        <ol className="onboarding-steps" aria-label="Setup progress">
          <li className={desktop.workspace === null ? "is-active" : "is-done"}>
            <span>1</span>
            Workspace
          </li>
          <li className={desktop.workspace === null ? "" : "is-active"}>
            <span>2</span>
            Provider
          </li>
          <li>
            <span>3</span>
            Launch
          </li>
        </ol>

        {desktop.workspace === null ? (
          <>
            <div className="onboarding-workspace-step">
              <span className="setup-icon" aria-hidden="true">
                <IconFolder size={25} stroke={1.5} />
              </span>
              <div>
                <h2>Add your first Workspace</h2>
                <p>
                  This folder is the repository context and relative-path
                  anchor. Full access can reach beyond it; choose an isolated
                  execution boundary next if you want confinement. Runtime state
                  stays in its private Desktop partition under the Colossus
                  home.
                </p>
              </div>
              <button
                className="button primary"
                type="button"
                disabled={busy}
                onClick={() => void onChooseWorkspace()}
              >
                Add Workspace from folder
                <IconArrowRight size={16} stroke={1.8} aria-hidden="true" />
              </button>
            </div>
            {error !== "" ? (
              <p className="page-error" role="alert">
                {error}
              </p>
            ) : null}
          </>
        ) : showAdvanced ? (
          <ModelConfigurationEditor
            desktop={desktop}
            busy={busy}
            onApply={onApplyConfiguration}
            onBack={() => setShowAdvanced(false)}
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
                Add another Workspace
              </button>
            </div>

            <div className="provider-fields">
              <label>
                <span>Provider protocol</span>
                <DropdownSelect
                  value={providerKind}
                  disabled={busy}
                  onChange={(event) => {
                    const kind = event.target
                      .value as ConfigureManagedRuntimeRequest["providerKind"];
                    setProviderKind(kind);
                    setModel(managedProviderDefaults(kind).model);
                    setReplaceCredential(false);
                  }}
                >
                  <option value="openai_compatible">
                    OpenRouter (OpenAI-compatible)
                  </option>
                  <option value="openai_responses">OpenAI Responses</option>
                  <option value="open_ai_codex">
                    ChatGPT subscription (Codex)
                  </option>
                </DropdownSelect>
              </label>
              <label>
                <span>Model</span>
                <input
                  value={model}
                  maxLength={256}
                  required
                  spellCheck={false}
                  disabled={busy}
                  onChange={(event) => setModel(event.target.value)}
                />
              </label>
              <label className="provider-wide-field">
                <span>Access profile</span>
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
                    Pinned — exact tools configured in Settings
                  </option>
                  <option value="development">
                    Development — approval-gated effects
                  </option>
                  <option value="allow_all">
                    Allow all — every declared built-in tool
                  </option>
                </DropdownSelect>
              </label>
              <label className="provider-wide-field">
                <span>Execution boundary</span>
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
                  <strong>Unsafe: Full access.</strong> Commands can use host
                  files, environment variables, and network access without
                  Colossus isolation. Approval mode is a separate setting.
                </p>
              </div>
            ) : null}

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
            ) : dismissible &&
              desktop.provider.configured &&
              !providerChanged ? (
              <label className="provider-credential-toggle">
                <input
                  type="checkbox"
                  checked={replaceCredential}
                  disabled={busy}
                  onChange={(event) =>
                    setReplaceCredential(event.target.checked)
                  }
                />
                <span>
                  <strong>Replace the stored API key</strong>
                  <small>
                    Leave this off to reuse the existing native keychain entry
                    when changing only the model, access profile, or execution
                    boundary.
                  </small>
                </span>
              </label>
            ) : null}

            <div className="provider-security-note">
              <IconCloudLock size={19} stroke={1.6} aria-hidden="true" />
              <p>
                {accessConfirmationRequired
                  ? "A separate native confirmation is required before this access profile can be enabled. "
                  : ""}
                {boundaryConfirmationRequired
                  ? "The execution boundary has its own separate native confirmation. "
                  : ""}
                {providerPromptRequired
                  ? "Continue opens a native secure prompt for the fixed provider origin. The key never enters this WebView or renderer IPC, and native code stores it directly in the OS keychain."
                  : providerKind === "open_ai_codex"
                    ? "The official Codex credential remains file-backed. Only its native path crosses private inherited bootstrap IPC; tokens never enter the WebView."
                    : "The existing provider key remains in the OS keychain. Only the model, access policy, and execution boundary cross this WebView boundary."}
              </p>
            </div>

            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() => setShowAdvanced(true)}
            >
              Configure multiple providers, model limits, capabilities, and role
              routing
            </button>

            {error !== "" ? (
              <p className="page-error" role="alert">
                {error}
              </p>
            ) : null}

            <button
              className="button primary onboarding-launch"
              type="submit"
              disabled={
                busy ||
                model.trim() === "" ||
                (providerKind === "open_ai_codex" && !codexReady)
              }
            >
              {busy ? "Starting Managed Local…" : "Continue securely"}
              <IconArrowRight size={16} stroke={1.8} aria-hidden="true" />
            </button>
          </form>
        )}

        <div className="offline-self-test-row">
          <IconShieldCheck size={19} stroke={1.6} aria-hidden="true" />
          <div>
            <strong>Verify this installation offline</strong>
            <span>
              Starts the bundled runtime with an offline profile. It does not
              configure a provider or enable model runs.
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
              ? "Running self-test…"
              : selfTest.state === "passed"
                ? "Run again"
                : "Run offline self-test"}
          </button>
        </div>

        <div className="external-setup-row">
          <IconPlugConnected size={19} stroke={1.6} aria-hidden="true" />
          <div>
            <strong>Already run a Colossus daemon?</strong>
            <span>
              Import its connection JSON. Pins, paths, and keyring labels stay
              native-only.
            </span>
          </div>
          <button
            className="button secondary"
            type="button"
            disabled={busy}
            onClick={() => void onUseExternal()}
          >
            Import external
          </button>
        </div>
      </section>
    </main>
  );
}
