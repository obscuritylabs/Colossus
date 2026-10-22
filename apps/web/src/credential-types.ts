export interface CredentialTarget {
  kind: "provider" | "mcp";
  id: string;
}
export interface CredentialSetup {
  target: CredentialTarget;
  authentication: "personal" | "shared";
}
