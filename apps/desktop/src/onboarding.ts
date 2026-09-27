import type {
  CommandError,
  ConfigureManagedRuntimeRequest,
  DesktopStatus,
  ManagedModelConfiguration,
  ManagedProviderConfiguration,
  WorkspaceSummary,
} from "./types";

export function managedSetupLaunchFailure(
  status: Pick<DesktopStatus, "connection">,
): CommandError | null {
  if (status.connection.state === "connected") return null;
  return {
    code: "managed_setup_not_connected",
    message: `Your settings were saved, but Colossus could not connect. ${status.connection.message} Review your settings and try again.`,
    retryable: true,
    outcomeUnknown: false,
    violations: [],
  };
}

export interface ManagedProviderDraft {
  providerKind: ConfigureManagedRuntimeRequest["providerKind"];
  model: string;
  accessProfile: ConfigureManagedRuntimeRequest["accessProfile"];
  executionBoundary: ConfigureManagedRuntimeRequest["executionBoundary"];
  replaceCredential: boolean;
  baseUrl?: ConfigureManagedRuntimeRequest["baseUrl"];
  credentialId?: ConfigureManagedRuntimeRequest["credentialId"];
  noCredential?: ConfigureManagedRuntimeRequest["noCredential"];
  modelMetadata?: ConfigureManagedRuntimeRequest["modelMetadata"];
}

export type ManagedSelfTestStatus =
  | { state: "idle" | "running" | "passed"; message: string }
  | { state: "failed"; message: string };

export const INITIAL_MANAGED_SELF_TEST_STATUS: ManagedSelfTestStatus = {
  state: "idle",
  message: "",
};

export function managedOnboardingRequired(desktop: DesktopStatus): boolean {
  if (desktop.workspace !== null && desktop.provider.configured) {
    return false;
  }

  const selectedExternal = desktop.targets.some(
    (target) =>
      target.targetId === desktop.selectedTargetId &&
      target.kind === "external_daemon",
  );
  return !selectedExternal;
}

export function managedProviderDefaults(
  _providerKind: ConfigureManagedRuntimeRequest["providerKind"],
): Pick<ManagedProviderDraft, "model"> {
  return { model: "" };
}

/** Simple setup can only save one default route without extra connection options. */
export function requiresAdvancedModelSetup(configuration: {
  providers: Pick<ManagedProviderConfiguration, "profile" | "timeoutMs">[];
  models: Pick<
    ManagedModelConfiguration,
    "profile" | "providerProfile" | "reasoningEffort"
  >[];
  roles: Record<string, string>;
}): boolean {
  const { providers, models, roles } = configuration;
  return (
    providers.length > 1 ||
    models.length > 1 ||
    providers.some(
      (provider) =>
        provider.profile !== "primary-provider" || provider.timeoutMs !== null,
    ) ||
    models.some(
      (model) =>
        model.profile !== "primary" ||
        model.providerProfile !== "primary-provider" ||
        model.reasoningEffort !== null,
    ) ||
    Object.entries(roles).some(
      ([role, profile]) => role !== "primary" || profile !== "primary",
    )
  );
}

export function buildManagedRuntimeRequest(
  workspace: WorkspaceSummary | null,
  draft: ManagedProviderDraft,
): ConfigureManagedRuntimeRequest | null {
  const model = draft.model.trim();
  if (workspace === null || model === "") {
    return null;
  }

  return {
    workspaceId: workspace.workspaceId,
    providerKind: draft.providerKind,
    model,
    accessProfile: draft.accessProfile,
    executionBoundary: draft.executionBoundary,
    replaceCredential: draft.replaceCredential,
    ...(draft.baseUrl !== undefined ? { baseUrl: draft.baseUrl } : {}),
    ...(draft.credentialId !== undefined
      ? { credentialId: draft.credentialId }
      : {}),
    ...(draft.noCredential !== undefined
      ? { noCredential: draft.noCredential }
      : {}),
    ...(draft.modelMetadata !== undefined
      ? { modelMetadata: draft.modelMetadata }
      : {}),
  };
}

export async function submitManagedRuntimeConfiguration(
  workspace: WorkspaceSummary | null,
  draft: ManagedProviderDraft,
  configure: (request: ConfigureManagedRuntimeRequest) => Promise<boolean>,
): Promise<boolean> {
  const request = buildManagedRuntimeRequest(workspace, draft);
  if (request === null) {
    return false;
  }

  return configure(request);
}

export async function runOfflineSelfTest(
  run: () => Promise<void>,
  update: (status: ManagedSelfTestStatus) => void,
): Promise<void> {
  update({
    state: "running",
    message: "Checking that Colossus can run on this computer…",
  });
  try {
    await run();
    update({
      state: "passed",
      message: "Installation check passed. No model provider was contacted.",
    });
  } catch (reason: unknown) {
    update({
      state: "failed",
      message:
        reason instanceof Error
          ? reason.message
          : "The installation check failed. Try again to check whether Colossus can start.",
    });
  }
}
