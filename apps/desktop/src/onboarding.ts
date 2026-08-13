import type {
  ConfigureManagedRuntimeRequest,
  DesktopStatus,
  WorkspaceSummary,
} from "./types";

export interface ManagedProviderDraft {
  providerKind: ConfigureManagedRuntimeRequest["providerKind"];
  model: string;
  accessProfile: ConfigureManagedRuntimeRequest["accessProfile"];
  replaceCredential: boolean;
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
  providerKind: ConfigureManagedRuntimeRequest["providerKind"],
): Pick<ManagedProviderDraft, "model"> {
  if (providerKind === "openai_responses") {
    return {
      model: "gpt-5",
    };
  }
  if (providerKind === "open_ai_codex") {
    return {
      model: "",
    };
  }
  return {
    model: "deepseek/deepseek-v4-flash",
  };
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
    replaceCredential: draft.replaceCredential,
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
    message: "Checking the bundled runtime without contacting a model…",
  });
  try {
    await run();
    update({
      state: "passed",
      message: "Offline runtime self-test passed. No provider was contacted.",
    });
  } catch (reason: unknown) {
    update({
      state: "failed",
      message:
        reason instanceof Error
          ? reason.message
          : "The offline runtime self-test failed safely.",
    });
  }
}
