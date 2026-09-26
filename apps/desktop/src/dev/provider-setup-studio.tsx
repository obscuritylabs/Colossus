/** Browser-only fixture around production setup and native API adapters. */
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { OnboardingSurface } from "../components/OnboardingSurface";
import {
  configureManagedRuntime,
  applyManagedModelConfiguration,
} from "../api";
import type { DesktopStatus } from "../types";
import { AppearanceProvider } from "../theme/AppearanceProvider";

const desktop: DesktopStatus = {
  releaseChannel: "development",
  connection: {
    state: "not_configured",
    message: "Choose a provider.",
    targetId: null,
  },
  targets: [],
  spaces: [],
  selectedTargetId: null,
  selectedSpaceId: null,
  managedState: "needs_provider",
  workspace: {
    workspaceId: "019b98f6-fd27-7413-bbe4-a44d97c0ff68",
    displayName: "Setup test",
    displayPath: "~/setup-test",
  },
  provider: { configured: false, kind: null, model: "" },
  codexAuth: { state: "signed_in", message: "ChatGPT connected." },
  managedModelConfiguration: { providers: [], models: [], roles: {} },
  accessProfile: "allow_all",
  executionBoundary: "full_access",
  approvalMode: "ask",
  terminalEnabled: false,
  additionalCaBundle: {
    configured: false,
    certificateCount: 0,
    fingerprintsSha256: [],
  },
  capabilities: {
    delegation: false,
    plugins: false,
    tui: false,
    shellTerminal: false,
    files: false,
    artifacts: false,
    planContinuation: false,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
};

export default function ProviderSetupStudio({
  configured,
  workspaceSelected,
  hasCredential,
  showControls = true,
  guided = true,
}: {
  configured: boolean;
  workspaceSelected: boolean;
  hasCredential: boolean;
  showControls?: boolean;
  guided?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [visible, setVisible] = useState(true);
  const [saved, setSaved] = useState(false);
  const [signedIn, setSignedIn] = useState(true);
  const [workspaceNumber, setWorkspaceNumber] = useState(
    workspaceSelected ? 1 : 0,
  );
  const [multipleProviders, setMultipleProviders] = useState(false);
  const [failure, setFailure] = useState("");
  const [certificatesImported, setCertificatesImported] = useState(false);
  const [cancelFolder, setCancelFolder] = useState(false);
  const currentDesktop: DesktopStatus = structuredClone(
    (configured || multipleProviders) && workspaceNumber <= 1
      ? {
          ...desktop,
          provider: {
            configured: true,
            kind: "openai_compatible",
            model: "vendor/previous",
          },
          managedModelConfiguration: {
            providers: [
              {
                profile: "primary-provider",
                providerKind: "openai_compatible",
                baseUrl: "https://openrouter.ai/api/v1",
                hasCredential,
                timeoutMs: null,
                effectiveTimeoutMs: 300000,
              },
            ],
            models: [
              {
                profile: "primary",
                providerProfile: "primary-provider",
                model: "vendor/previous",
                contextWindowTokens: 32768,
                maxOutputTokens: 4096,
                capabilities: {
                  toolCalls: false,
                  imageInputs: false,
                  streaming: false,
                },
                reasoningEffort: null,
              },
            ],
            roles: { primary: "primary" },
          },
        }
      : desktop,
  );
  currentDesktop.workspace =
    workspaceNumber === 0
      ? null
      : {
          ...desktop.workspace!,
          workspaceId:
            workspaceNumber === 1
              ? desktop.workspace!.workspaceId
              : "019b98f6-fd27-7413-bbe4-a44d97c0ff69",
          displayName: `Setup test ${workspaceNumber}`,
        };
  if (multipleProviders && workspaceNumber <= 1) {
    const configuration = currentDesktop.managedModelConfiguration;
    configuration.providers.push({
      ...configuration.providers[0]!,
      profile: "secondary-provider",
      baseUrl: "https://secondary.example.test/v1",
    });
    configuration.models.push({
      ...configuration.models[0]!,
      profile: "secondary",
      providerProfile: "secondary-provider",
      model: "vendor/secondary",
      reasoningEffort: "high",
    });
    configuration.roles.research_worker = "secondary";
  }

  async function save(action: () => Promise<unknown>) {
    setBusy(true);
    setFailure("");
    try {
      await action();
      setSaved(true);
      return true;
    } catch (error) {
      setFailure(
        error instanceof Error ? error.message : "Configuration failed.",
      );
      return false;
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      {showControls ? (
        <div>
          <button onClick={() => setBusy((value) => !value)}>
            Toggle unrelated busy state
          </button>
          <button onClick={() => setVisible((value) => !value)}>
            {visible ? "Close setup fixture" : "Open setup fixture"}
          </button>
          <button onClick={() => setWorkspaceNumber((value) => value + 1)}>
            Switch workspace fixture
          </button>
          <button onClick={() => setMultipleProviders(true)}>
            Load saved multiple providers fixture
          </button>
          <button onClick={() => setCancelFolder(true)}>
            Cancel next folder selection
          </button>
        </div>
      ) : null}
      {saved ? (
        <main className="setup-preview-complete">
          <h1>Setup complete</h1>
          <p role="status">Configuration saved</p>
          <p>
            This browser preview uses sample data. No workspace or provider was
            changed.
          </p>
        </main>
      ) : visible ? (
        <div className="app-shell" style={{ minHeight: "100%" }}>
          <OnboardingSurface
            desktop={{
              ...currentDesktop,
              additionalCaBundle: certificatesImported
                ? {
                    configured: true,
                    certificateCount: 1,
                    fingerprintsSha256: [],
                  }
                : currentDesktop.additionalCaBundle,
              codexAuth: {
                state: signedIn ? "signed_in" : "signed_out",
                message: signedIn
                  ? "ChatGPT connected."
                  : "Sign in to load your Codex models.",
              },
            }}
            busy={busy}
            error={failure}
            onChooseWorkspace={async () => {
              if (cancelFolder) {
                setCancelFolder(false);
                return;
              }
              setWorkspaceNumber((value) => value + 1);
            }}
            onImportCaBundle={async () => {
              setCertificatesImported(true);
            }}
            onConfigure={(request) =>
              save(() => configureManagedRuntime(request))
            }
            onApplyConfiguration={(request) =>
              save(() => applyManagedModelConfiguration(request))
            }
            onRunSelfTest={async () => {}}
            onCodexLogin={async () => {
              setSignedIn(true);
            }}
            onCodexLogout={async () => {
              setSignedIn(false);
            }}
            onUseExternal={async () => {}}
            dismissible={configured || !guided}
            onCancel={() => {}}
          />
        </div>
      ) : null}
    </>
  );
}

export function mountProviderSetupStudio(
  configured = false,
  workspaceSelected = true,
  hasCredential = true,
  guided = false,
) {
  if (!import.meta.env.DEV)
    throw new Error("Setup fixture requires development mode.");
  document.getElementById("root")!.style.display = "none";
  const host = document.createElement("div");
  host.style.height = "100%";
  host.style.overflow = "auto";
  document.body.append(host);
  createRoot(host).render(
    <AppearanceProvider>
      <ProviderSetupStudio
        configured={configured}
        workspaceSelected={workspaceSelected}
        hasCredential={hasCredential}
        guided={guided}
      />
    </AppearanceProvider>,
  );
}
