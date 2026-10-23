export interface CredentialTarget {
  kind: "provider" | "mcp" | "search";
  id: string;
}
export interface CredentialSetup {
  target: CredentialTarget;
  authentication: "personal" | "shared";
}
