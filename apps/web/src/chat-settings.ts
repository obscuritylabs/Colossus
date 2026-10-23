export type Authentication = "personal" | "shared" | "none";
export type NativeFields = Record<string, unknown>;
export type SettingsOrigin = "global" | "project";
export type InheritedSection =
  "providers" | "models" | "mcp" | "search" | "variables" | "roles";
export interface SettingsInheritance {
  global_revision: number;
  reset?: Partial<Record<InheritedSection, string[]>>;
}
export interface SettingsInheritanceSnapshot extends SettingsInheritance {
  overrides: Record<InheritedSection, string[]>;
  provenance: Record<InheritedSection, Record<string, SettingsOrigin>>;
  global_entries?: Record<InheritedSection, string[]>;
  disabled?: Record<InheritedSection, string[]>;
}
export interface SearchProfile extends NativeFields {
  kind: "searxng" | "serp_api";
  endpoint?: string | null;
  credentialReference?: string | null;
  authHeader?: string | null;
  userAgent?: string | null;
  timeoutMs?: number | null;
}
export interface ObservabilitySettings {
  enabled: boolean;
  serviceName: string;
  resourceAttributes: Record<string, string>;
  traces: { enabled: boolean; sampleRatio: number };
  metrics: { enabled: boolean; exportIntervalMs: number };
  logs: {
    otlp: boolean;
    stdoutJson: boolean;
    journalPayloads: "disabled" | "metadata" | "full";
    acknowledgeSensitiveContent: boolean;
  };
  otlp: {
    endpoint: string | null;
    protocol: "grpc" | "http_protobuf";
    timeoutMs: number;
    acknowledgeInsecureTransport: boolean;
  };
}
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
    search?: Record<string, string>;
  };
  providers: NativeFields & { profiles: Record<string, ProviderProfile> };
  models: NativeFields & {
    profiles: Record<string, ModelProfile>;
    roles: Record<string, string>;
  };
  mcp: NativeFields & { servers: Record<string, McpServer> };
  provider_authentication: Record<string, Authentication>;
  mcp_authentication: Record<string, Authentication>;
  search?: NativeFields & {
    profiles: Record<string, SearchProfile>;
    roles: Record<string, string>;
  };
  search_authentication?: Record<string, Authentication>;
  variables?: Record<string, string>;
  observability?: ObservabilitySettings;
  inheritance?: SettingsInheritance;
}
export interface ChatSettingsSnapshot extends ChatSettingsWrite {
  execution?: {
    enabled: boolean;
    settings_editable: boolean;
    reason?: string;
  };
  inheritance?: SettingsInheritanceSnapshot;
  telemetry?: {
    state: "active" | "disabled" | "restart_required";
    configured_revision: number;
    applied_revision: number;
  };
  credential_status: {
    providers: Record<string, boolean>;
    mcp: Record<string, boolean>;
    search?: Record<string, boolean>;
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
    ...(value.search ? { search: value.search } : {}),
    ...(value.search_authentication
      ? { search_authentication: value.search_authentication }
      : {}),
    ...(value.variables ? { variables: value.variables } : {}),
    ...(value.observability ? { observability: value.observability } : {}),
    ...(value.inheritance
      ? { inheritance: { global_revision: value.inheritance.global_revision } }
      : {}),
  });
}
export function validProfile(value: string) {
  return (
    /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(value) &&
    !["__proto__", "constructor", "prototype"].includes(value)
  );
}

export function validVariableName(value: string) {
  return /^[A-Za-z0-9_]{1,128}$/.test(value);
}
export function tokens(value: string): string[] {
  return [...new Set(value.split(/[\s,]+/).filter(Boolean))];
}
