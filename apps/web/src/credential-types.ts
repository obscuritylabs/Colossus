export interface CredentialTarget {
  kind: "provider" | "mcp" | "search";
  id: string;
}
export interface CredentialSetup {
  target: CredentialTarget;
  authentication: "personal" | "shared";
}

/** Read-only connection metadata may exist before an operator enables execution. */
export interface CredentialExecution {
  enabled: boolean;
  settings_editable: boolean;
  reason?: string;
}
