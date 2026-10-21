export type Authentication = "personal" | "shared" | "none";
export type NativeFields = Record<string, unknown>;
export interface ProviderProfile extends NativeFields {
  kind: string;
  baseUrl?: string | null;
  credentialReference?: string | null;
  timeoutMs?: number | null;
}
export interface ModelProfile extends NativeFields {
  providerProfile: string;
  model: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
  reasoningEffort?: string | null;
  capabilities?: NativeFields;
}
export interface McpServer extends NativeFields {
  transport: "streamable_http";
  url: string;
  allowedTools: string[];
  credentialHeaders?: Record<
    string,
    { reference: string; scheme?: string | null }
  >;
  oauth?:
    | (NativeFields & {
        clientId: string;
        callbackPort: number;
        scopes: string[];
      })
    | null;
}
export interface ChatSettingsWrite {
  revision: number;
  labels?: {
    providers: Record<string, string>;
    models: Record<string, string>;
  };
  providers: NativeFields & { profiles: Record<string, ProviderProfile> };
  models: NativeFields & {
    profiles: Record<string, ModelProfile>;
    roles: Record<string, string>;
  };
  mcp: NativeFields & { servers: Record<string, McpServer> };
  provider_authentication: Record<string, Authentication>;
  mcp_authentication: Record<string, Authentication>;
}
export interface ChatSettingsSnapshot extends ChatSettingsWrite {
  credential_status: {
    providers: Record<string, boolean>;
    mcp: Record<string, boolean>;
  };
}
/** Copy only the typed configuration contract, excluding credential status metadata. */
export function settingsWrite(value: ChatSettingsSnapshot): ChatSettingsWrite {
  return structuredClone({
    revision: value.revision,
    ...(value.labels ? { labels: value.labels } : {}),
    providers: value.providers,
    models: value.models,
    mcp: value.mcp,
    provider_authentication: value.provider_authentication,
    mcp_authentication: value.mcp_authentication,
  });
}
export function validProfile(value: string) {
  return (
    /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(value) &&
    !["__proto__", "constructor", "prototype"].includes(value)
  );
}
export function tokens(value: string): string[] {
  return [...new Set(value.split(/[\s,]+/).filter(Boolean))];
}
