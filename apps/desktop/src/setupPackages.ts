import type { ManagedModelConfiguration, ProviderKind } from "./types";

export interface SetupProvider {
  catalogResourceId?: string | null;
  profile: string;
  displayName: string;
  descriptionMarkdown: string;
  kind: ProviderKind;
  baseUrl: string;
  timeoutMs: number | null;
  credentialRequired: boolean;
  credentialId: string | null;
  icon: string | null;
  darkIcon: string | null;
  models: ManagedModelConfiguration[];
}
export interface SetupPackage {
  id: string;
  name: string;
  version: string;
  sha256: string;
  descriptionMarkdown: string;
  providers: SetupProvider[];
  roles: Record<string, string>;
  certificateFingerprints: string[];
  existingCertificateFingerprints: string[];
  replacesVersion: string | null;
}
export const SETUP_CHANGED_EVENT = "colossus-setup-changed";
export function setupChanged() {
  window.dispatchEvent(new Event(SETUP_CHANGED_EVENT));
}
export function setupCredentialStatus(
  provider: SetupProvider,
  codexSignedIn: boolean,
) {
  if (provider.kind === "open_ai_codex")
    return codexSignedIn ? "Signed in" : "Needs sign-in";
  if (!provider.credentialRequired) return "No key required";
  return provider.credentialId ? "Key saved · not checked" : "Needs API key";
}
export function setupProviderReady(
  provider: SetupProvider,
  codexSignedIn: boolean,
) {
  return provider.kind === "open_ai_codex"
    ? codexSignedIn
    : !provider.credentialRequired || !!provider.credentialId;
}
export function safeSetupIcon(value?: string | null): string | undefined {
  return value &&
    value.length <= 90_000 &&
    /^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(value)
    ? value
    : undefined;
}

/** Apply only the selected connection; the other imported providers remain available. */
export function importedWorkspaceConfiguration(
  provider: SetupProvider,
  selected: import("./types").ManagedModelConfiguration,
  workspaceId: string,
  accessProfile: import("./types").AccessProfile,
  executionBoundary: import("./types").ExecutionBoundary,
  credentialAction: import("./types").CredentialAction,
  credentialId: string | null,
  importedRoles: Record<string, string> = {},
): import("./types").ApplyManagedModelConfigurationRequest {
  const model = { ...selected, providerProfile: provider.profile };
  const models = [
    ...provider.models.filter((entry) => entry.profile !== model.profile),
    model,
  ];
  const roles = Object.fromEntries(
    Object.entries(importedRoles).filter(([, profile]) =>
      models.some((entry) => entry.profile === profile),
    ),
  );
  return {
    workspaceId,
    providers: [
      {
        profile: provider.profile,
        providerKind: provider.kind,
        baseUrl: provider.baseUrl,
        timeoutMs: provider.timeoutMs,
        credentialAction,
        ...(credentialId ? { credentialId } : {}),
      },
    ],
    models,
    roles: { ...roles, primary: model.profile },
    accessProfile,
    executionBoundary,
  };
}
