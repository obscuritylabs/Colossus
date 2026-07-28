import {
  IconArrowRight,
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
  const [replaceCredential, setReplaceCredential] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [selfTest, setSelfTest] = useState(INITIAL_MANAGED_SELF_TEST_STATUS);
  const providerChanged =
    desktop.provider.configured && desktop.provider.kind !== providerKind;
  const providerPromptRequired =
    !desktop.provider.configured || providerChanged || replaceCredential;
  const developmentConfirmationRequired =
    accessProfile === "development" &&
    (!desktop.provider.configured || desktop.accessProfile !== "development");

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await submitManagedRuntimeConfiguration(
      desktop.workspace,
      {
        providerKind,
        model,
        accessProfile,
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
            Pick a workspace and provider. The app supervises an isolated local
            runtime while credentials remain in the native keychain boundary.
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
                <h2>Choose a folder</h2>
                <p>
                  Agent tools are confined to the folder you select. Runtime
                  state is stored separately in Colossus application support.
                </p>
              </div>
              <button
                className="button primary"
                type="button"
                disabled={busy}
                onClick={() => void onChooseWorkspace()}
              >
                Choose workspace
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
                Change
              </button>
            </div>

            <div className="provider-fields">
              <label>
                <span>Provider protocol</span>
                <select
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
                </select>
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
                <select
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
                  <option value="development">
                    Development — approval-gated effects
                  </option>
                </select>
              </label>
            </div>

            {dismissible && desktop.provider.configured && !providerChanged ? (
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
                    when changing only the model or access profile.
                  </small>
                </span>
              </label>
            ) : null}

            <div className="provider-security-note">
              <IconCloudLock size={19} stroke={1.6} aria-hidden="true" />
              <p>
                {developmentConfirmationRequired
                  ? "A separate native confirmation is required before Development access can be enabled. "
                  : ""}
                {providerPromptRequired
                  ? "Continue opens a native secure prompt for the fixed provider origin. The key never enters this WebView or renderer IPC, and native code stores it directly in the OS keychain."
                  : "The existing provider key remains in the OS keychain. Only the model and access policy cross this WebView boundary."}
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
              disabled={busy || model.trim() === ""}
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
            disabled={
              busy || desktop.workspace === null || selfTest.state === "running"
            }
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
