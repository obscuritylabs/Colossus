import {
  IconActivityHeartbeat,
  IconAdjustments,
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconCloud,
  IconCpu,
  IconDatabase,
  IconEdit,
  IconFileImport,
  IconFolder,
  IconKey,
  IconLock,
  IconNetwork,
  IconPlus,
  IconRefresh,
  IconRoute,
  IconSearch,
  IconServer,
  IconShield,
  IconTerminal2,
  IconTrash,
  IconWorld,
  IconX,
} from "@tabler/icons-react";
import { type InputHTMLAttributes, useEffect, useMemo, useState } from "react";

import {
  applyRepositoryConfiguration,
  applySpaceConfiguration,
  beginManagedMcpOAuth,
  completeManagedMcpOAuth,
  createManagedCredential,
  deleteManagedCredential,
  diagnoseManagedMcpServer,
  diagnoseManagedModel,
  diagnoseManagedProvider,
  diagnoseManagedSearch,
  diagnoseManagedTelemetry,
  getManagedExtensionInventory,
  getManagedConfiguration,
  inspectRepositoryConfiguration,
  logoutManagedMcpOAuth,
  managedMcpOAuthStatus,
  rotateManagedCredential,
  saveGlobalDefaults,
  saveSpaceConfiguration,
  upsertGlobalMcpServer,
  upsertGlobalModel,
  upsertGlobalProvider,
  upsertGlobalSearchProvider,
  upsertGlobalTelemetryProfile,
} from "../api";
import type {
  AccessProfile,
  CatalogEntry,
  DesktopStatus,
  ExecutionBoundary,
  ImportConflictDecision,
  ManagedCredentialKind,
  ManagedDefaultOverrides,
  ManagedExtensionInventory,
  ManagedFieldDescriptor,
  ManagedFieldOverride,
  ManagedMcpServer,
  ManagedMcpDiagnostic,
  ManagedMcpOAuthLogin,
  ManagedMcpOAuthStatus,
  ManagedMcpResearchTool,
  ManagedRuntimeDiagnostic,
  ManagedModelCatalogValue,
  ManagedProviderCatalogValue,
  ManagedSearchProvider,
  ManagedSettingsSnapshot,
  ManagedSpaceConfigurationSnapshot,
  ManagedTelemetryProfile,
  ReasoningEffort,
  RepositoryConfigurationProposal,
  RuntimeTarget,
  TerminalKind,
} from "../types";
import { AppearanceSettings } from "./AppearanceSettings";
import { DropdownSelect } from "./DropdownSelect";

type SettingsScope = "global" | "space";
type GlobalTab =
  | "providers"
  | "models"
  | "credentials"
  | "mcp"
  | "search"
  | "telemetry"
  | "defaults"
  | "desktop";
type SpaceTab =
  | "runtime"
  | "providers"
  | "mcp"
  | "access"
  | "sandbox"
  | "search"
  | "telemetry"
  | "research"
  | "advanced"
  | "effective";

interface ManagedSettingsPaneProps {
  desktop: DesktopStatus;
  connecting: boolean;
  updateChecking: boolean;
  updateMessage: string;
  onChooseWorkspace: () => void;
  onConfigureManaged: () => void;
  onRestartManaged: () => void;
  onAddExternalTarget: () => void;
  onRemoveExternalTarget: (targetId: string) => void;
  onSetTerminalEnabled: (enabled: boolean) => void;
  onOpenTerminal: (kind: TerminalKind) => void;
  onCheckForUpdates: () => void;
  onInstallUpdate: () => void;
  onImportCaBundle: () => void;
  onRemoveCaBundle: () => void;
}

interface DefaultsDraft {
  accessProfile: AccessProfile | null;
  executionBoundary: ExecutionBoundary | null;
  terminalEnabled: boolean | null;
  fields: Record<string, unknown>;
}

interface SpaceDraft {
  accessProfile: AccessProfile | null;
  executionBoundary: ExecutionBoundary | null;
  terminalEnabled: boolean | null;
  fields: Record<string, unknown>;
  selectedProviders: string[];
  selectedModels: string[];
  selectedMcp: string[];
  selectedSearch: string[];
  selectedTelemetry: string | null;
  searchRoles: Record<string, string>;
  modelRoles: Record<string, string>;
  credentialOverrides: Record<string, string>;
}

export interface McpCredentialBindingDraft {
  name: string;
  credentialId: string;
  scheme: string;
}

export interface McpEditorDraft {
  resourceId: string | null;
  label: string;
  name: string;
  transport: ManagedMcpServer["transport"];
  commandOrUrl: string;
  argsText: string;
  workingDirectory: string;
  environmentCredentials: McpCredentialBindingDraft[];
  headers: Record<string, string>;
  credentialHeaders: McpCredentialBindingDraft[];
  allowStateless: boolean;
  oauthEnabled: boolean;
  oauthClientId: string;
  oauthClientSecretCredentialId: string;
  oauthCallbackPort: number;
  oauthScopesText: string;
  allowedToolsText: string;
  researchTools: ManagedMcpResearchTool[];
  timeoutMs: number | null;
  maxOutputBytes: number | null;
}

export interface ProviderEditorDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  kind: ManagedProviderCatalogValue["kind"];
  baseUrl: string;
  credentialId: string;
  timeoutMs: number | null;
}

export interface ModelEditorDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  providerProfile: string;
  model: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
  toolCalls: boolean;
  streaming: boolean;
  imageInputs: boolean;
  reasoningEffort: ReasoningEffort | null;
}

interface SearchEditorDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  kind: ManagedSearchProvider["kind"];
  endpoint: string;
  credentialId: string;
  authHeader: string;
  timeoutMs: number;
}

export interface TelemetryEditorDraft extends ManagedTelemetryProfile {
  resourceId: string | null;
  label: string;
  resourceAttributesText: string;
}

const GLOBAL_TABS: ReadonlyArray<{ id: GlobalTab; label: string }> = [
  { id: "providers", label: "Providers" },
  { id: "models", label: "Models" },
  { id: "credentials", label: "Credentials" },
  { id: "mcp", label: "MCP" },
  { id: "search", label: "Search" },
  { id: "telemetry", label: "Telemetry" },
  { id: "defaults", label: "Defaults" },
  { id: "desktop", label: "Desktop" },
];

const SPACE_TABS: ReadonlyArray<{ id: SpaceTab; label: string }> = [
  { id: "runtime", label: "Runtime" },
  { id: "providers", label: "Providers" },
  { id: "mcp", label: "MCP" },
  { id: "access", label: "Access" },
  { id: "sandbox", label: "Sandbox" },
  { id: "search", label: "Search" },
  { id: "telemetry", label: "Telemetry" },
  { id: "research", label: "Research" },
  { id: "advanced", label: "Advanced" },
  { id: "effective", label: "Effective YAML" },
];

export function managedFieldDestination(descriptor: ManagedFieldDescriptor): {
  tab: "sandbox" | "research" | "advanced" | "runtime";
  section: string | null;
} {
  if (descriptor.id.startsWith("sandbox.")) {
    return { tab: "sandbox", section: null };
  }
  if (descriptor.id.startsWith("research.")) {
    return { tab: "research", section: null };
  }
  if (descriptor.advanced) {
    return { tab: "advanced", section: descriptor.section };
  }
  return { tab: "runtime", section: null };
}

export function advancedSectionContainsField(
  descriptors: ManagedFieldDescriptor[],
  fieldId: string | null,
): boolean {
  return fieldId !== null && descriptors.some(({ id }) => id === fieldId);
}

function managedFieldElementId(fieldId: string): string {
  return `managed-setting-${fieldId}`;
}

const EMPTY_MCP_DRAFT: McpEditorDraft = {
  resourceId: null,
  label: "",
  name: "",
  transport: "stdio",
  commandOrUrl: "",
  argsText: "",
  workingDirectory: "",
  environmentCredentials: [],
  headers: {},
  credentialHeaders: [],
  allowStateless: false,
  oauthEnabled: false,
  oauthClientId: "",
  oauthClientSecretCredentialId: "",
  oauthCallbackPort: 8765,
  oauthScopesText: "",
  allowedToolsText: "",
  researchTools: [],
  timeoutMs: null,
  maxOutputBytes: null,
};

const EMPTY_PROVIDER_DRAFT: ProviderEditorDraft = {
  resourceId: null,
  label: "",
  profile: "",
  kind: "openai_compatible",
  baseUrl: "https://",
  credentialId: "",
  timeoutMs: null,
};

const EMPTY_MODEL_DRAFT: ModelEditorDraft = {
  resourceId: null,
  label: "",
  profile: "",
  providerProfile: "",
  model: "",
  contextWindowTokens: 128_000,
  maxOutputTokens: 16_384,
  toolCalls: true,
  streaming: true,
  imageInputs: false,
  reasoningEffort: null,
};

const EMPTY_SEARCH_DRAFT: SearchEditorDraft = {
  resourceId: null,
  label: "",
  profile: "",
  kind: "searxng",
  endpoint: "https://",
  credentialId: "",
  authHeader: "X-Searxng-Key",
  timeoutMs: 30_000,
};

const EMPTY_TELEMETRY_DRAFT: TelemetryEditorDraft = {
  resourceId: null,
  label: "",
  name: "colossus-desktop",
  endpoint: "https://",
  protocol: "grpc",
  timeoutMs: 10_000,
  tracesEnabled: true,
  traceSampleRatioMillionths: 100_000,
  metricsEnabled: true,
  metricExportIntervalMs: 60_000,
  logsOtlp: true,
  logsStdoutJson: false,
  journalPayloads: "metadata",
  acknowledgeSensitiveContent: false,
  acknowledgeInsecureTransport: false,
  resourceAttributes: {},
  resourceAttributesText: "service.namespace=colossus",
};

const FIELD_DESCRIPTORS: ManagedFieldDescriptor[] = [
  advancedField(
    "access.tools.include",
    "Access",
    "Included tools",
    "Tool names explicitly added to the available tool set.",
    "string_list",
    [],
  ),
  advancedField(
    "access.tools.exclude",
    "Access",
    "Excluded tools",
    "Tool names removed even when another setting makes them available.",
    "string_list",
    [],
  ),
  advancedField(
    "access.actions.allow",
    "Access",
    "Allowed actions",
    "Action names that policy may allow without additional approval.",
    "string_list",
    [],
  ),
  advancedField(
    "access.actions.requireApproval",
    "Access",
    "Approval actions",
    "Actions that always require confirmation before they run.",
    "string_list",
    [],
  ),
  advancedField(
    "access.actions.deny",
    "Access",
    "Denied actions",
    "Actions that Colossus must never run.",
    "string_list",
    [],
  ),
  advancedField(
    "audit.exporter",
    "Audit",
    "Evidence exporter",
    "Choose where audit evidence is exported. Leave disabled to keep export off.",
    "json",
    { kind: "disabled" },
  ),
  advancedField(
    "policy",
    "Policy",
    "Decision policy",
    "Set the policy that allows, denies, or requires approval for actions.",
    "json",
    { kind: "built_in", requirePostEffect: false },
  ),
  field(
    "agent.maxTurns",
    "Agent",
    "Maximum turns",
    "Stop a run after this many agent turns.",
    50,
    false,
    1,
    1000,
  ),
  field(
    "subagents.maxConcurrent",
    "Subagents",
    "Concurrent subagents",
    "Limit how many subagents may run at the same time.",
    10,
    false,
    1,
    128,
  ),
  toggleField(
    "context.autoCompaction",
    "Context",
    "Automatic compaction",
    "Shorten older conversation history as the context window fills.",
    true,
    true,
  ),
  field(
    "context.compactAtPercent",
    "Context",
    "Compact at",
    "Start compaction when context usage reaches this percentage.",
    70,
    true,
    2,
    99,
  ),
  field(
    "context.targetPercent",
    "Context",
    "Compaction target",
    "Reduce context usage to this percentage after compaction.",
    45,
    true,
    1,
    98,
  ),
  field(
    "context.preserveRecentMessages",
    "Context",
    "Preserve recent messages",
    "Keep this many of the newest messages unchanged during compaction.",
    8,
    true,
    0,
    1024,
  ),
  toggleField(
    "context.modelAssisted",
    "Context",
    "Model-assisted compaction",
    "Let the selected model summarize older context during compaction.",
    true,
    true,
  ),
  field(
    "research.maxSources",
    "Research",
    "Maximum sources",
    "Limit how many sources a Research run may collect.",
    20,
    false,
    1,
    500,
  ),
  field(
    "research.maxWorkers",
    "Research",
    "Research workers",
    "Limit how many research tasks may run in parallel.",
    4,
    false,
    1,
    64,
  ),
  toggleField(
    "memory.indexEnabled",
    "Memory",
    "Memory index",
    "Index eligible session content so it can be found in later requests.",
    true,
    false,
  ),
  field(
    "memory.retrievalLimit",
    "Memory",
    "Retrieval limit",
    "Limit how many memory results are added to a request.",
    6,
    false,
    1,
    100,
  ),
  advancedField(
    "memory.semantic",
    "Memory",
    "Semantic projection",
    "Configure the optional semantic-memory projection.",
    "json",
    { kind: "disabled" },
  ),
  toggleField(
    "skills.enabled",
    "Skills",
    "Skills",
    "Allow skills to provide reusable instructions and workflows.",
    true,
    true,
  ),
  toggleField(
    "skills.allowUserOverrides",
    "Skills",
    "User skill overrides",
    "Allow user-installed skills to override bundled or repository skills.",
    false,
    true,
  ),
  advancedField(
    "skills.bundled",
    "Skills",
    "Bundled skill library",
    "Select the skill library included with Colossus.",
    "text",
    "bundled-skills",
  ),
  advancedField(
    "skills.repository",
    "Skills",
    "Repository skill library",
    "Set the workspace-relative directory that contains repository skills.",
    "text",
    ".colossus/skills",
  ),
  advancedField(
    "skills.disabled",
    "Skills",
    "Disabled skills",
    "List skill names that should not be loaded.",
    "string_list",
    [],
  ),
  advancedField(
    "workflows.repository",
    "Workflows",
    "Repository workflows",
    "Set the workspace-relative directory that contains reusable workflows.",
    "text",
    ".colossus/workflows",
  ),
  advancedField(
    "sandbox.profile",
    "Sandbox",
    "Policy profile",
    "Choose the isolation policy used for tools and subprocesses.",
    "text",
    "offline-default",
  ),
  toggleField(
    "sandbox.allowBrokerFallback",
    "Sandbox",
    "Broker fallback",
    "Allow the broker when the preferred isolation backend is unavailable.",
    false,
    true,
  ),
  advancedField(
    "sandbox.helperPath",
    "Sandbox",
    "Isolation helper",
    "Optional path to the platform isolation helper executable.",
    "text",
    null,
  ),
  advancedField(
    "sandbox.ociRuntime",
    "Sandbox",
    "OCI runtime",
    "OCI runtime executable used for container isolation.",
    "text",
    null,
  ),
  advancedField(
    "sandbox.ociImage",
    "Sandbox",
    "OCI image",
    "Container image used for isolated execution.",
    "text",
    null,
  ),
  advancedField(
    "sandbox.ociProxyImage",
    "Sandbox",
    "OCI proxy image",
    "Container image used by the sandbox network proxy.",
    "text",
    null,
  ),
  advancedField(
    "sandbox.filesystem",
    "Sandbox",
    "Filesystem grants",
    "Choose which paths sandboxed processes may read or write.",
    "json",
    [],
  ),
  advancedField(
    "sandbox.executables",
    "Sandbox",
    "Executable allowlist",
    "List executables that tools may start inside the sandbox.",
    "string_list",
    [],
  ),
  advancedField(
    "sandbox.environment",
    "Sandbox",
    "Environment allowlist",
    "List environment variables available to sandboxed processes.",
    "string_list",
    [],
  ),
  advancedField(
    "sandbox.networkDestinations",
    "Network trust",
    "Network origins",
    "List network destinations sandboxed processes may contact.",
    "string_list",
    [],
  ),
  field(
    "sandbox.maxOutputBytes",
    "Limits",
    "Maximum output",
    "Limit the combined stdout and stderr captured from one effect, in bytes.",
    1_048_576,
    true,
    1024,
    1_073_741_824,
  ),
  field(
    "sandbox.timeoutMs",
    "Limits",
    "Effect timeout",
    "Stop an effect after this many milliseconds.",
    30_000,
    true,
    100,
    3_600_000,
  ),
  field(
    "sandbox.maxProcesses",
    "Limits",
    "Maximum processes",
    "Limit the number of processes allowed inside one sandbox.",
    16,
    true,
    1,
    1024,
  ),
  field(
    "sandbox.maxMemoryBytes",
    "Limits",
    "Maximum memory",
    "Limit the memory available to one sandbox, in bytes.",
    268_435_456,
    true,
    1_048_576,
    68_719_476_736,
  ),
  field(
    "sandbox.maxConcurrency",
    "Limits",
    "Effect concurrency",
    "Limit how many effects may run at the same time.",
    1,
    true,
    1,
    128,
  ),
];

function advancedField(
  id: string,
  section: string,
  title: string,
  description: string,
  control: ManagedFieldDescriptor["control"],
  defaultValue: unknown,
): ManagedFieldDescriptor {
  return {
    id,
    section,
    title,
    description,
    scope: "both",
    risk: "high",
    control,
    advanced: true,
    defaultValue,
    minimum: null,
    maximum: null,
    options: [],
  };
}

function field(
  id: string,
  section: string,
  title: string,
  description: string,
  defaultValue: number,
  advanced: boolean,
  minimum: number,
  maximum: number,
): ManagedFieldDescriptor {
  return {
    id,
    section,
    title,
    description,
    scope: "both",
    risk: advanced ? "medium" : "low",
    control: "number",
    advanced,
    defaultValue,
    minimum,
    maximum,
    options: [],
  };
}

function toggleField(
  id: string,
  section: string,
  title: string,
  description: string,
  defaultValue: boolean,
  advanced: boolean,
): ManagedFieldDescriptor {
  return {
    id,
    section,
    title,
    description,
    scope: "both",
    risk: advanced ? "high" : "low",
    control: "toggle",
    advanced,
    defaultValue,
    minimum: null,
    maximum: null,
    options: [],
  };
}

function currentValue<T>(entry: CatalogEntry<T>): T {
  return (
    entry.revisions.find(
      (revision) => revision.revision === entry.currentRevision,
    )?.value ?? entry.revisions[entry.revisions.length - 1]!.value
  );
}

function referencedValue<T>(
  entries: CatalogEntry<T>[],
  reference: { resourceId: string; revision: number },
): T | undefined {
  return entries
    .find((entry) => entry.id === reference.resourceId)
    ?.revisions.find((revision) => revision.revision === reference.revision)
    ?.value;
}

function mcpUsesCredential(
  server: ManagedMcpServer,
  credentialId: string,
): boolean {
  return (
    Object.values(server.environmentCredentials).includes(credentialId) ||
    Object.values(server.credentialHeaders).some(
      (binding) => binding.credentialId === credentialId,
    ) ||
    server.oauth?.clientSecretCredentialId === credentialId
  );
}

function catalogReferenceUsesCredential(
  global: ManagedSettingsSnapshot["globalConfiguration"],
  catalogKey: string,
  reference: { resourceId: string; revision: number },
  credentialId: string,
): boolean {
  if (catalogKey.startsWith("provider:")) {
    return (
      referencedValue(global.providers, reference)?.credentialId ===
      credentialId
    );
  }
  if (catalogKey.startsWith("search:")) {
    return (
      referencedValue(global.searchProviders, reference)?.credentialId ===
      credentialId
    );
  }
  if (catalogKey.startsWith("mcp:")) {
    const server = referencedValue(global.mcpServers, reference);
    return server !== undefined && mcpUsesCredential(server, credentialId);
  }
  return false;
}

export function managedCredentialConsumers(
  snapshot: Pick<ManagedSettingsSnapshot, "globalConfiguration" | "spaces">,
  credentialId: string,
): string[] {
  const consumers = new Set<string>();
  const global = snapshot.globalConfiguration;

  for (const entry of global.providers) {
    if (!entry.archived && currentValue(entry).credentialId === credentialId) {
      consumers.add(`Provider · ${entry.label}`);
    }
  }
  for (const entry of global.searchProviders) {
    if (!entry.archived && currentValue(entry).credentialId === credentialId) {
      consumers.add(`Search · ${entry.label}`);
    }
  }
  for (const entry of global.mcpServers) {
    if (entry.archived) continue;
    if (mcpUsesCredential(currentValue(entry), credentialId)) {
      consumers.add(`MCP server · ${entry.label}`);
    }
  }
  for (const space of snapshot.spaces) {
    if (space.archived) continue;
    const overrideUsesCredential = Object.values(
      space.configuration.credentialOverrides,
    ).includes(credentialId);
    const pinnedRevisionUsesCredential = Object.entries(
      space.configuration.catalogRevisions,
    ).some(([catalogKey, reference]) =>
      catalogReferenceUsesCredential(
        global,
        catalogKey,
        reference,
        credentialId,
      ),
    );
    if (overrideUsesCredential || pinnedRevisionUsesCredential) {
      consumers.add(`Workspace · ${space.name}`);
    }
  }

  return [...consumers].sort((left, right) => left.localeCompare(right));
}

export function managedSearchProfileConsumers(
  snapshot: Pick<ManagedSettingsSnapshot, "globalConfiguration" | "spaces">,
  resourceId: string,
): string[] {
  const entry = snapshot.globalConfiguration.searchProviders.find(
    (candidate) => candidate.id === resourceId && !candidate.archived,
  );
  if (!entry) return [];

  const profile = currentValue(entry).profile;
  return snapshot.spaces
    .filter((space) => {
      if (space.archived) return false;
      const pinned = Object.values(space.configuration.catalogRevisions).some(
        (reference) => reference.resourceId === resourceId,
      );
      const assigned = Object.values(space.configuration.searchRoles).includes(
        profile,
      );
      return pinned || assigned;
    })
    .map((space) => space.name)
    .sort((left, right) => left.localeCompare(right));
}

export function managedModelConsumers(
  snapshot: Pick<ManagedSettingsSnapshot, "globalConfiguration" | "spaces">,
  resourceId: string,
): string[] {
  const entry = snapshot.globalConfiguration.models.find(
    (candidate) => candidate.id === resourceId && !candidate.archived,
  );
  if (!entry) return [];

  const profile = currentValue(entry).profile;
  return snapshot.spaces
    .filter((space) => {
      if (space.archived) return false;
      const pinned = Object.values(space.configuration.catalogRevisions).some(
        (reference) => reference.resourceId === resourceId,
      );
      const assigned = Object.values(space.configuration.modelRoles).includes(
        profile,
      );
      return pinned || assigned;
    })
    .map((space) => space.name)
    .sort((left, right) => left.localeCompare(right));
}

export function managedProviderConsumers(
  snapshot: Pick<ManagedSettingsSnapshot, "globalConfiguration">,
  resourceId: string,
): string[] {
  const entry = snapshot.globalConfiguration.providers.find(
    (candidate) => candidate.id === resourceId && !candidate.archived,
  );
  if (!entry) return [];

  const profile = currentValue(entry).profile;
  return snapshot.globalConfiguration.models
    .filter(
      (model) =>
        !model.archived && currentValue(model).providerProfile === profile,
    )
    .map((model) => model.label)
    .sort((left, right) => left.localeCompare(right));
}

export function managedTelemetryConsumers(
  snapshot: Pick<ManagedSettingsSnapshot, "globalConfiguration" | "spaces">,
  resourceId: string,
): string[] {
  const entry = snapshot.globalConfiguration.telemetryProfiles.find(
    (candidate) => candidate.id === resourceId && !candidate.archived,
  );
  if (!entry) return [];

  return snapshot.spaces
    .filter(
      (space) =>
        !space.archived &&
        Object.entries(space.configuration.catalogRevisions).some(
          ([catalogKey, reference]) =>
            catalogKey.startsWith("telemetry:") &&
            reference.resourceId === resourceId,
        ),
    )
    .map((space) => space.name)
    .sort((left, right) => left.localeCompare(right));
}

function compactTokenCount(value: number): string {
  if (value >= 1_000_000) {
    return `${Number((value / 1_000_000).toFixed(1))}m`;
  }
  if (value >= 1_000) {
    return `${Number((value / 1_000).toFixed(1))}k`;
  }
  return String(value);
}

function providerAdapterLabel(
  kind: ManagedProviderCatalogValue["kind"],
): string {
  switch (kind) {
    case "openai_responses":
      return "OpenAI Responses";
    case "openai_compatible":
      return "OpenAI compatible";
    case "open_ai_codex":
      return "Codex subscription";
  }
}

function providerEndpointLabel(provider: ManagedProviderCatalogValue): string {
  if (provider.kind === "open_ai_codex") return "Codex subscription";
  try {
    return new URL(provider.baseUrl).host;
  } catch {
    return "Invalid endpoint";
  }
}

function telemetryProtocolLabel(
  protocol: ManagedTelemetryProfile["protocol"],
): string {
  return protocol === "grpc" ? "OTLP gRPC" : "OTLP HTTP/protobuf";
}

function telemetryEndpointLabel(telemetry: ManagedTelemetryProfile): string {
  if (!telemetry.endpoint) return "No OTLP collector";
  try {
    return new URL(telemetry.endpoint).host;
  } catch {
    return "Invalid collector endpoint";
  }
}

function telemetrySignalCount(telemetry: ManagedTelemetryProfile): number {
  return [
    telemetry.tracesEnabled,
    telemetry.metricsEnabled,
    telemetry.logsOtlp,
  ].filter(Boolean).length;
}

function telemetryJournalLabel(
  journalPayloads: ManagedTelemetryProfile["journalPayloads"],
): string {
  switch (journalPayloads) {
    case "disabled":
      return "Audit content off";
    case "metadata":
      return "Metadata only";
    case "full":
      return "Full audit content";
  }
}

function searchAdapterLabel(kind: ManagedSearchProvider["kind"]): string {
  return kind === "searxng" ? "SearXNG" : "SerpAPI";
}

function credentialKindLabel(kind: ManagedCredentialKind): string {
  switch (kind) {
    case "api_key":
      return "API key";
    case "bearer_token":
      return "Bearer token";
    case "client_secret":
      return "OAuth client secret";
    case "generic_secret":
      return "Generic secret";
  }
}

function SwitchInput(
  props: Omit<
    InputHTMLAttributes<HTMLInputElement>,
    "className" | "role" | "type"
  >,
) {
  return (
    <input {...props} className="switch-input" type="checkbox" role="switch" />
  );
}

function fixtureCatalogUpsert<T>(
  entries: CatalogEntry<T>[],
  resourceId: string | null,
  label: string,
  value: T,
) {
  const existing = entries.find((entry) => entry.id === resourceId);
  if (existing) {
    existing.currentRevision += 1;
    existing.label = label;
    existing.revisions.push({
      revision: existing.currentRevision,
      value,
    });
    return;
  }
  entries.push({
    id: crypto.randomUUID(),
    label,
    currentRevision: 1,
    archived: false,
    revisions: [{ revision: 1, value }],
  });
}

function defaultOverrides(
  snapshot: ManagedSettingsSnapshot,
): ManagedDefaultOverrides {
  const defaults = snapshot.globalConfiguration.defaults;
  return (
    defaults.revisions.find(
      (revision) => revision.revision === defaults.currentRevision,
    )?.value ?? {
      accessProfile: null,
      executionBoundary: null,
      terminalEnabled: null,
      fieldOverrides: [],
    }
  );
}

function overrideMap(overrides: readonly ManagedFieldOverride[]) {
  return Object.fromEntries(
    overrides.map((override) => [override.fieldId, override.value]),
  );
}

function defaultsDraft(snapshot: ManagedSettingsSnapshot): DefaultsDraft {
  const defaults = defaultOverrides(snapshot);
  return {
    accessProfile: defaults.accessProfile,
    executionBoundary: defaults.executionBoundary,
    terminalEnabled: defaults.terminalEnabled,
    fields: overrideMap(defaults.fieldOverrides),
  };
}

export function spaceDraft(
  space: ManagedSpaceConfigurationSnapshot,
): SpaceDraft {
  return {
    accessProfile: space.configuration.accessProfileOverride,
    executionBoundary: space.configuration.executionBoundaryOverride,
    terminalEnabled: space.configuration.terminalEnabledOverride,
    fields: overrideMap(space.configuration.fieldOverrides),
    selectedProviders: Object.entries(space.configuration.catalogRevisions)
      .filter(([key]) => key.startsWith("provider:"))
      .map(([, reference]) => reference.resourceId),
    selectedModels: Object.entries(space.configuration.catalogRevisions)
      .filter(([key]) => key.startsWith("model:"))
      .map(([, reference]) => reference.resourceId),
    selectedMcp: Object.entries(space.configuration.catalogRevisions)
      .filter(
        ([catalogKey, reference]) =>
          catalogKey.startsWith("mcp:") && reference.resourceId.length > 0,
      )
      .map(([, reference]) => reference.resourceId),
    selectedSearch: Object.entries(space.configuration.catalogRevisions)
      .filter(
        ([catalogKey, reference]) =>
          catalogKey.startsWith("search:") && reference.resourceId.length > 0,
      )
      .map(([, reference]) => reference.resourceId),
    selectedTelemetry:
      Object.entries(space.configuration.catalogRevisions).find(
        ([catalogKey, reference]) =>
          catalogKey.startsWith("telemetry:") &&
          reference.resourceId.length > 0,
      )?.[1].resourceId ?? null,
    searchRoles: space.configuration.searchRoles,
    modelRoles: space.configuration.modelRoles,
    credentialOverrides: space.configuration.credentialOverrides,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function isFixtureRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    new URLSearchParams(window.location.search).has("fixture")
  );
}

export function buildManagedSettingsFixture(
  desktop: DesktopStatus,
): ManagedSettingsSnapshot {
  const fixture = isFixtureRuntime();
  const credentials = fixture
    ? [
        {
          id: "018f0000-0000-7000-8000-000000000001",
          label: "GitHub workspace token",
          kind: "bearer_token" as const,
          backend: "desktop" as const,
          createdAtMs: 1_721_490_000_000,
        },
        {
          id: "018f0000-0000-7000-8000-000000000002",
          label: "Documentation API key",
          kind: "api_key" as const,
          backend: "desktop" as const,
          createdAtMs: 1_721_491_000_000,
        },
      ]
    : [];
  const mcpServers: CatalogEntry<ManagedMcpServer>[] = fixture
    ? [
        mcpEntry(
          "018f1000-0000-7000-8000-000000000001",
          "github-local",
          "stdio",
          "github-mcp",
          ["search_repos", "read_file", "create_issue", "open_pull_request"],
          credentials[0]?.id ?? "",
        ),
        mcpEntry(
          "018f1000-0000-7000-8000-000000000002",
          "splunk-search",
          "streamable_http",
          "https://splunk.example.test/mcp",
          ["search_events", "read_metadata", "list_indexes"],
          "",
        ),
        mcpEntry(
          "018f1000-0000-7000-8000-000000000003",
          "docs-index",
          "stdio",
          "docs-mcp",
          ["search", "read_document", "list_collections"],
          credentials[1]?.id ?? "",
        ),
      ]
    : [];
  const providers = desktop.managedModelConfiguration.providers.map(
    (provider, index) => ({
      id: `018f2000-0000-7000-8000-${String(index + 1).padStart(12, "0")}`,
      label: provider.profile,
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            profile: provider.profile,
            kind: provider.providerKind,
            baseUrl: provider.baseUrl,
            credentialId: null,
            timeoutMs: provider.timeoutMs,
          },
        },
      ],
    }),
  );
  const models = desktop.managedModelConfiguration.models.map(
    (model, index) => ({
      id: `018f3000-0000-7000-8000-${String(index + 1).padStart(12, "0")}`,
      label: model.profile,
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: model }],
    }),
  );
  const searchProviders: CatalogEntry<ManagedSearchProvider>[] = fixture
    ? [
        {
          id: "018f4000-0000-7000-8000-000000000001",
          label: "Engineering search",
          currentRevision: 1,
          archived: false,
          revisions: [
            {
              revision: 1,
              value: {
                profile: "engineering-search",
                kind: "searxng",
                endpoint: "https://search.example.test/search",
                credentialId: credentials[1]?.id ?? null,
                authHeader: "X-Searxng-Key",
                timeoutMs: 30_000,
              },
            },
          ],
        },
      ]
    : [];
  const telemetryProfiles: CatalogEntry<ManagedTelemetryProfile>[] = fixture
    ? [
        {
          id: "018f5000-0000-7000-8000-000000000001",
          label: "Local collector",
          currentRevision: 1,
          archived: false,
          revisions: [
            {
              revision: 1,
              value: {
                name: "colossus-desktop",
                endpoint: "http://127.0.0.1:4317",
                protocol: "grpc",
                timeoutMs: 10_000,
                tracesEnabled: true,
                traceSampleRatioMillionths: 100_000,
                metricsEnabled: true,
                metricExportIntervalMs: 60_000,
                logsOtlp: true,
                logsStdoutJson: false,
                journalPayloads: "metadata",
                acknowledgeSensitiveContent: false,
                acknowledgeInsecureTransport: false,
                resourceAttributes: { "service.namespace": "colossus" },
              },
            },
          ],
        },
      ]
    : [];
  const selectedSpaceId = desktop.selectedSpaceId ?? desktop.spaces[0]?.spaceId;
  const spaces = desktop.spaces.map((space, index) => {
    const selectedMcp =
      fixture && index === 0 ? [mcpServers[0]!, mcpServers[2]!] : [];
    const selectedSearch = fixture && index === 0 ? [searchProviders[0]!] : [];
    const selectedTelemetry =
      fixture && index === 0 ? telemetryProfiles[0]! : null;
    return {
      id: space.spaceId,
      name: space.displayName,
      displayPath: space.displayPath,
      archived: space.archived,
      status:
        fixture && index === 0
          ? ("update_available" as const)
          : ("active" as const),
      statusMessage:
        fixture && index === 0
          ? "Global changes are ready to review and apply."
          : "Runtime configuration is active.",
      pendingGlobalRevision: fixture && index === 0 ? 4 : null,
      configuration: {
        acceptedGlobalRevision: fixture && index === 0 ? 3 : 4,
        catalogRevisions: Object.fromEntries([
          ...providers.map(
            (entry) =>
              [
                `provider:${entry.id}`,
                { resourceId: entry.id, revision: entry.currentRevision },
              ] as const,
          ),
          ...models.map(
            (entry) =>
              [
                `model:${entry.id}`,
                { resourceId: entry.id, revision: entry.currentRevision },
              ] as const,
          ),
          ...selectedMcp.map((entry) => [
            `mcp:${entry.id}`,
            { resourceId: entry.id, revision: entry.currentRevision },
          ]),
          ...selectedSearch.map((entry) => [
            `search:${entry.id}`,
            { resourceId: entry.id, revision: entry.currentRevision },
          ]),
          ...(selectedTelemetry
            ? [
                [
                  `telemetry:${selectedTelemetry.id}`,
                  {
                    resourceId: selectedTelemetry.id,
                    revision: selectedTelemetry.currentRevision,
                  },
                ] as const,
              ]
            : []),
        ]),
        credentialOverrides: {},
        searchRoles:
          selectedSearch.length > 0
            ? {
                agent: "engineering-search",
                research: "engineering-search",
              }
            : {},
        modelRoles: desktop.managedModelConfiguration.roles,
        accessProfileOverride:
          space.spaceId === selectedSpaceId ? desktop.accessProfile : null,
        executionBoundaryOverride:
          space.spaceId === selectedSpaceId ? desktop.executionBoundary : null,
        terminalEnabledOverride: null,
        fieldOverrides: [],
        import: null,
      },
      effectiveValues: [
        {
          fieldId: "access.profile",
          value: desktop.accessProfile,
          source: "space" as const,
        },
        {
          fieldId: "sandbox.executionBoundary",
          value: desktop.executionBoundary,
          source: "space" as const,
        },
        ...FIELD_DESCRIPTORS.map((descriptor) => ({
          fieldId: descriptor.id,
          value: descriptor.defaultValue,
          source: "built_in" as const,
        })),
      ],
      effectiveYaml: `schemaVersion: 1\ndesktopManaged:\n  workspaceIdentity: <desktop-managed>\n  storagePath: <desktop-managed>\nworkspace:\n  id: ${space.spaceId}\n  acceptedGlobalRevision: ${fixture && index === 0 ? 3 : 4}\naccess:\n  profile: ${desktop.accessProfile}\nsandbox:\n  executionBoundary: ${desktop.executionBoundary}\nmcp:\n  servers: ${selectedMcp.length}\n`,
    };
  });
  return {
    globalConfiguration: {
      revision: 4,
      providers,
      models,
      mcpServers,
      searchProviders,
      telemetryProfiles,
      credentials,
      defaults: {
        currentRevision: 4,
        revisions: [
          {
            revision: 4,
            value: {
              accessProfile: "development",
              executionBoundary: "workspace_isolated",
              terminalEnabled: false,
              fieldOverrides: [],
            },
          },
        ],
      },
    },
    spaces,
    fieldDescriptors: FIELD_DESCRIPTORS,
    lockedInvariants: [
      locked("storage.path", "Runtime storage path"),
      locked("workspace.identity", "Workspace identity"),
      locked("runtime.workerIpc", "Worker IPC"),
      locked("runtime.bootstrapAuthentication", "Bootstrap authentication"),
      locked("sandbox.backend", "Sandbox backend"),
      locked("sandbox.filesystem", "Desktop filesystem grants"),
    ],
  };
}

function locked(id: string, title: string) {
  return {
    id,
    title,
    owner: "Desktop" as const,
    explanation: "Owned and generated by the native Desktop runtime.",
  };
}

function mcpEntry(
  id: string,
  name: string,
  transport: ManagedMcpServer["transport"],
  endpoint: string,
  allowedTools: string[],
  credentialId: string,
): CatalogEntry<ManagedMcpServer> {
  const server: ManagedMcpServer = {
    name,
    transport,
    command: transport === "stdio" ? endpoint : null,
    args: [],
    workingDirectory: null,
    environmentCredentials:
      credentialId && transport === "stdio" ? { MCP_TOKEN: credentialId } : {},
    url: transport === "streamable_http" ? endpoint : null,
    headers: {},
    credentialHeaders:
      credentialId && transport === "streamable_http"
        ? { Authorization: { scheme: "Bearer", credentialId } }
        : {},
    allowStateless: false,
    oauth: null,
    allowedTools,
    researchTools: [],
    timeoutMs: 30_000,
    maxOutputBytes: 1_048_576,
  };
  return {
    id,
    label: name,
    currentRevision: 1,
    archived: false,
    revisions: [{ revision: 1, value: server }],
  };
}

export function ManagedSettingsPane({
  desktop,
  connecting,
  updateChecking,
  updateMessage,
  onChooseWorkspace,
  onConfigureManaged,
  onRestartManaged,
  onAddExternalTarget,
  onRemoveExternalTarget,
  onSetTerminalEnabled,
  onOpenTerminal,
  onCheckForUpdates,
  onInstallUpdate,
  onImportCaBundle,
  onRemoveCaBundle,
}: ManagedSettingsPaneProps) {
  const initial = useMemo(
    () => buildManagedSettingsFixture(desktop),
    [desktop],
  );
  const [snapshot, setSnapshot] = useState(initial);
  const [scope, setScope] = useState<SettingsScope>("space");
  const [globalTab, setGlobalTab] = useState<GlobalTab>("mcp");
  const [spaceTab, setSpaceTab] = useState<SpaceTab>("runtime");
  const [focusedFieldId, setFocusedFieldId] = useState<string | null>(null);
  const [expandedAdvancedSections, setExpandedAdvancedSections] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const [selectedSpaceId, setSelectedSpaceId] = useState(
    desktop.selectedSpaceId ?? initial.spaces[0]?.id ?? "",
  );
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [failure, setFailure] = useState("");
  const [defaults, setDefaults] = useState(() => defaultsDraft(initial));
  const [space, setSpace] = useState<SpaceDraft>(() =>
    initial.spaces[0]
      ? spaceDraft(initial.spaces[0])
      : {
          accessProfile: null,
          executionBoundary: null,
          terminalEnabled: null,
          fields: {},
          selectedProviders: [],
          selectedModels: [],
          selectedMcp: [],
          selectedSearch: [],
          selectedTelemetry: null,
          searchRoles: {},
          modelRoles: {},
          credentialOverrides: {},
        },
  );
  const [mcpEditor, setMcpEditor] = useState<McpEditorDraft | null>(null);
  const [providerEditor, setProviderEditor] =
    useState<ProviderEditorDraft | null>(null);
  const [modelEditor, setModelEditor] = useState<ModelEditorDraft | null>(null);
  const [searchEditor, setSearchEditor] = useState<SearchEditorDraft | null>(
    null,
  );
  const [telemetryEditor, setTelemetryEditor] =
    useState<TelemetryEditorDraft | null>(null);
  const [importProposal, setImportProposal] =
    useState<RepositoryConfigurationProposal | null>(null);
  const [importStage, setImportStage] = useState(0);
  const [importMappings, setImportMappings] = useState<Record<string, string>>(
    {},
  );
  const [importConflicts, setImportConflicts] = useState<
    Record<string, ImportConflictDecision>
  >({});
  const [credentialLabel, setCredentialLabel] = useState("");
  const [credentialKind, setCredentialKind] =
    useState<ManagedCredentialKind>("api_key");
  const [mcpDiagnostics, setMcpDiagnostics] = useState<
    Record<string, ManagedMcpDiagnostic>
  >({});
  const [mcpOauthStatuses, setMcpOauthStatuses] = useState<
    Record<string, ManagedMcpOAuthStatus>
  >({});
  const [mcpOauthLogins, setMcpOauthLogins] = useState<
    Record<string, ManagedMcpOAuthLogin>
  >({});
  const [mcpOauthCallbacks, setMcpOauthCallbacks] = useState<
    Record<string, string>
  >({});
  const [runtimeDiagnostics, setRuntimeDiagnostics] = useState<
    Record<string, ManagedRuntimeDiagnostic>
  >({});
  const [extensionInventory, setExtensionInventory] =
    useState<ManagedExtensionInventory | null>(null);
  const [extensionInventorySpaceId, setExtensionInventorySpaceId] =
    useState("");
  const [extensionInventoryBusy, setExtensionInventoryBusy] = useState(false);

  const selectedSpace =
    snapshot.spaces.find((candidate) => candidate.id === selectedSpaceId) ??
    snapshot.spaces[0];

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let active = true;
    void getManagedConfiguration()
      .then((next) => {
        if (active) setSnapshot(next);
      })
      .catch((error: unknown) => {
        if (active)
          setFailure(
            error instanceof Error
              ? error.message
              : "Settings could not be loaded.",
          );
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    setDefaults(defaultsDraft(snapshot));
  }, [snapshot]);

  useEffect(() => {
    if (selectedSpace) setSpace(spaceDraft(selectedSpace));
  }, [selectedSpace]);

  useEffect(() => {
    if (spaceTab !== "advanced" || !selectedSpace) return;
    if (selectedSpace.status !== "active") {
      setExtensionInventory(null);
      setExtensionInventorySpaceId("");
      return;
    }
    if (extensionInventorySpaceId === selectedSpace.id) return;
    void loadExtensionInventory();
  }, [spaceTab, selectedSpace?.id, selectedSpace?.status]);

  useEffect(() => {
    if (!focusedFieldId || query || scope !== "space") return;
    const frame = window.requestAnimationFrame(() => {
      const target = document.getElementById(
        managedFieldElementId(focusedFieldId),
      );
      target?.scrollIntoView({ block: "center" });
      target?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [focusedFieldId, query, scope, spaceTab]);

  const defaultsDirty =
    JSON.stringify(defaults) !== JSON.stringify(defaultsDraft(snapshot));
  const spaceDirty =
    selectedSpace !== undefined &&
    JSON.stringify(space) !== JSON.stringify(spaceDraft(selectedSpace));
  const descriptors = snapshot.fieldDescriptors.length
    ? snapshot.fieldDescriptors
    : FIELD_DESCRIPTORS;
  const effective = new Map(
    selectedSpace?.effectiveValues.map((value) => [value.fieldId, value]) ?? [],
  );

  async function perform(
    action: () => Promise<ManagedSettingsSnapshot>,
    fixtureAction: () => ManagedSettingsSnapshot,
    success: string,
  ) {
    setBusy(true);
    setFailure("");
    setNotice("");
    try {
      const next = isTauriRuntime() ? await action() : fixtureAction();
      setSnapshot(next);
      setNotice(success);
    } catch (error: unknown) {
      setFailure(
        error instanceof Error ? error.message : "The settings change failed.",
      );
    } finally {
      setBusy(false);
    }
  }

  function fixtureRevision(mutator: (draft: ManagedSettingsSnapshot) => void) {
    const draft = structuredClone(snapshot);
    draft.globalConfiguration.revision += 1;
    draft.globalConfiguration.defaults.currentRevision =
      draft.globalConfiguration.revision;
    mutator(draft);
    return draft;
  }

  async function saveDefaults() {
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      accessProfile: defaults.accessProfile,
      executionBoundary: defaults.executionBoundary,
      terminalEnabled: defaults.terminalEnabled,
      fieldOverrides: Object.entries(defaults.fields).map(
        ([fieldId, value]) => ({
          fieldId,
          value,
        }),
      ),
    };
    await perform(
      () => saveGlobalDefaults(request),
      () =>
        fixtureRevision((draft) => {
          draft.globalConfiguration.defaults.revisions.push({
            revision: draft.globalConfiguration.revision,
            value: {
              accessProfile: defaults.accessProfile,
              executionBoundary: defaults.executionBoundary,
              terminalEnabled: defaults.terminalEnabled,
              fieldOverrides: request.fieldOverrides,
            },
          });
          for (const configuredSpace of draft.spaces) {
            configuredSpace.pendingGlobalRevision =
              draft.globalConfiguration.revision;
            configuredSpace.status = "update_available";
          }
        }),
      "Global changes saved.",
    );
  }

  async function saveSpace() {
    if (!selectedSpace) return;
    const request = {
      expectedGlobalRevision: snapshot.globalConfiguration.revision,
      spaceId: selectedSpace.id,
      accessProfileOverride: space.accessProfile,
      executionBoundaryOverride: space.executionBoundary,
      terminalEnabledOverride: space.terminalEnabled,
      fieldOverrides: Object.entries(space.fields).map(([fieldId, value]) => ({
        fieldId,
        value,
      })),
      selectedProviderResourceIds: space.selectedProviders,
      selectedModelResourceIds: space.selectedModels,
      selectedMcpResourceIds: space.selectedMcp,
      selectedSearchResourceIds: space.selectedSearch,
      selectedTelemetryResourceId: space.selectedTelemetry,
      searchRoles: space.searchRoles,
      modelRoles: space.modelRoles,
      credentialOverrides: space.credentialOverrides,
    };
    await perform(
      () => saveSpaceConfiguration(request),
      () => {
        const draft = structuredClone(snapshot);
        const target = draft.spaces.find(
          (candidate) => candidate.id === selectedSpace.id,
        )!;
        target.configuration.accessProfileOverride = space.accessProfile;
        target.configuration.executionBoundaryOverride =
          space.executionBoundary;
        target.configuration.terminalEnabledOverride = space.terminalEnabled;
        target.configuration.fieldOverrides = request.fieldOverrides;
        target.configuration.credentialOverrides = space.credentialOverrides;
        target.configuration.searchRoles = space.searchRoles;
        target.configuration.modelRoles = space.modelRoles;
        const retained = Object.entries(
          target.configuration.catalogRevisions,
        ).filter(
          ([key]) =>
            !key.startsWith("provider:") &&
            !key.startsWith("model:") &&
            !key.startsWith("mcp:") &&
            !key.startsWith("search:") &&
            !key.startsWith("telemetry:"),
        );
        const providerReferences = space.selectedProviders.map((resourceId) => {
          const entry = draft.globalConfiguration.providers.find(
            (candidate) => candidate.id === resourceId,
          )!;
          return [
            `provider:${resourceId}`,
            { resourceId, revision: entry.currentRevision },
          ] as const;
        });
        const modelReferences = space.selectedModels.map((resourceId) => {
          const entry = draft.globalConfiguration.models.find(
            (candidate) => candidate.id === resourceId,
          )!;
          return [
            `model:${resourceId}`,
            { resourceId, revision: entry.currentRevision },
          ] as const;
        });
        const mcpReferences = space.selectedMcp.map((resourceId) => {
          const entry = draft.globalConfiguration.mcpServers.find(
            (candidate) => candidate.id === resourceId,
          )!;
          return [
            `mcp:${resourceId}`,
            { resourceId, revision: entry.currentRevision },
          ] as const;
        });
        const searchReferences = space.selectedSearch.map((resourceId) => {
          const entry = draft.globalConfiguration.searchProviders.find(
            (candidate) => candidate.id === resourceId,
          )!;
          return [
            `search:${resourceId}`,
            { resourceId, revision: entry.currentRevision },
          ] as const;
        });
        const telemetryReferences = space.selectedTelemetry
          ? (() => {
              const entry = draft.globalConfiguration.telemetryProfiles.find(
                (candidate) => candidate.id === space.selectedTelemetry,
              )!;
              return [
                [
                  `telemetry:${entry.id}`,
                  { resourceId: entry.id, revision: entry.currentRevision },
                ] as const,
              ];
            })()
          : [];
        target.configuration.catalogRevisions = Object.fromEntries([
          ...retained,
          ...providerReferences,
          ...modelReferences,
          ...mcpReferences,
          ...searchReferences,
          ...telemetryReferences,
        ]);
        target.status = "active";
        target.statusMessage = "Workspace settings applied.";
        return draft;
      },
      "Workspace settings applied.",
    );
  }

  async function applyPendingRevision() {
    if (!selectedSpace) return;
    await perform(
      () => applySpaceConfiguration(selectedSpace.id),
      () => {
        const draft = structuredClone(snapshot);
        const target = draft.spaces.find(
          (candidate) => candidate.id === selectedSpace.id,
        )!;
        target.configuration.acceptedGlobalRevision =
          draft.globalConfiguration.revision;
        target.pendingGlobalRevision = null;
        target.status = "active";
        target.statusMessage = "Global changes applied.";
        return draft;
      },
      "Global changes applied to this Workspace.",
    );
  }

  async function inspectRepositoryImport() {
    if (!selectedSpace) return;
    setBusy(true);
    setFailure("");
    try {
      const proposal = isTauriRuntime()
        ? await inspectRepositoryConfiguration(selectedSpace.id)
        : fixtureImportProposal(selectedSpace.id);
      setImportProposal(proposal);
      setImportStage(0);
      setImportMappings(
        Object.fromEntries(
          proposal.credentialSlots.map((slot) => [slot.slotId, ""]),
        ),
      );
      setImportConflicts(
        Object.fromEntries(
          proposal.resources
            .filter((resource) => resource.conflict)
            .map((resource) => [
              `${resource.kind}:${resource.sourceId}`,
              {
                action: "rename",
                renamedSourceId: `${resource.sourceId}-imported`,
              },
            ]),
        ),
      );
    } catch (error: unknown) {
      setFailure(
        error instanceof Error
          ? error.message
          : "Repository configuration inspection failed.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function applyRepositoryImport() {
    if (!importProposal) return;
    if (Object.values(importMappings).some((credential) => !credential)) {
      setImportStage(1);
      setFailure("Map every repository credential before applying.");
      return;
    }
    if (
      Object.values(importConflicts).some(
        (decision) =>
          decision.action === "rename" && !decision.renamedSourceId?.trim(),
      )
    ) {
      setImportStage(2);
      setFailure("Enter a new profile name for every rename decision.");
      return;
    }
    setBusy(true);
    setFailure("");
    try {
      if (isTauriRuntime()) {
        await applyRepositoryConfiguration({
          spaceId: importProposal.spaceId,
          expectedSha256: importProposal.sha256,
          credentialMappings: importMappings,
          conflictDecisions: importConflicts,
        });
        setSnapshot(await getManagedConfiguration());
      } else {
        const draft = structuredClone(snapshot);
        const target = draft.spaces.find(
          (candidate) => candidate.id === importProposal.spaceId,
        );
        if (target) {
          target.configuration.import = {
            relativePath: importProposal.relativePath,
            sha256: importProposal.sha256,
            importedAtMs: Date.now(),
          };
          target.statusMessage = "Repository configuration imported.";
        }
        setSnapshot(draft);
      }
      setImportProposal(null);
      setNotice("Repository configuration applied to this Workspace.");
    } catch (error: unknown) {
      setFailure(
        error instanceof Error
          ? error.message
          : "Repository configuration could not be applied.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function testMcpServer(server: string) {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const diagnostic = isTauriRuntime()
        ? await diagnoseManagedMcpServer(selectedSpace.id, server)
        : {
            server,
            healthy: true,
            tools: [
              {
                server,
                name: "search",
                title: "Search",
                description: "Fixture allowlisted search tool.",
              },
            ],
          };
      setMcpDiagnostics((current) => ({ ...current, [server]: diagnostic }));
      setNotice(
        `${server} is healthy; ${diagnostic.tools.length} tools discovered.`,
      );
    });
  }

  async function loadMcpOAuthStatus(server: string) {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const status = isTauriRuntime()
        ? await managedMcpOAuthStatus(selectedSpace.id, server)
        : { server, configured: true, authenticated: false };
      setMcpOauthStatuses((current) => ({ ...current, [server]: status }));
    });
  }

  async function loginMcpOAuth(server: string) {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const login = isTauriRuntime()
        ? await beginManagedMcpOAuth(selectedSpace.id, server)
        : {
            server,
            authorizationUrl: "https://auth.example.test/authorize",
            callbackUrl: "http://127.0.0.1:8765/callback",
          };
      setMcpOauthLogins((current) => ({ ...current, [server]: login }));
      setNotice("OAuth authorization is ready to open in your browser.");
    });
  }

  async function completeMcpOAuth(server: string) {
    if (!selectedSpace || !mcpOauthCallbacks[server]?.trim()) return;
    await runMcpDiagnostic(async () => {
      const status = isTauriRuntime()
        ? await completeManagedMcpOAuth(
            selectedSpace.id,
            server,
            mcpOauthCallbacks[server]!,
          )
        : { server, configured: true, authenticated: true };
      setMcpOauthStatuses((current) => ({ ...current, [server]: status }));
      setMcpOauthLogins((current) => {
        const next = { ...current };
        delete next[server];
        return next;
      });
      setNotice(`${server} OAuth login completed.`);
    });
  }

  async function logoutMcpOAuth(server: string) {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const status = isTauriRuntime()
        ? await logoutManagedMcpOAuth(selectedSpace.id, server)
        : { server, configured: true, authenticated: false };
      setMcpOauthStatuses((current) => ({ ...current, [server]: status }));
      setNotice(`${server} OAuth credential removed.`);
    });
  }

  async function runMcpDiagnostic(operation: () => Promise<void>) {
    setBusy(true);
    setFailure("");
    try {
      await operation();
    } catch (error: unknown) {
      setFailure(
        error instanceof Error
          ? error.message
          : "The managed MCP operation failed.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function testRuntimeProfile(
    kind: "provider" | "model",
    profile: string,
  ) {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const diagnostic = isTauriRuntime()
        ? kind === "provider"
          ? await diagnoseManagedProvider(selectedSpace.id, profile)
          : await diagnoseManagedModel(selectedSpace.id, profile)
        : fixtureRuntimeDiagnostic(kind, profile);
      setRuntimeDiagnostics((current) => ({
        ...current,
        [`${kind}:${profile}`]: diagnostic,
      }));
      setNotice(
        `${profile} ${kind} diagnostic ${diagnostic.ready ? "passed" : "failed"}.`,
      );
    });
  }

  async function testSearchRole(role: "agent" | "research") {
    if (!selectedSpace) return;
    await runMcpDiagnostic(async () => {
      const diagnostic = isTauriRuntime()
        ? await diagnoseManagedSearch(selectedSpace.id, role)
        : fixtureRuntimeDiagnostic("search", role);
      setRuntimeDiagnostics((current) => ({
        ...current,
        [`search:${role}`]: diagnostic,
      }));
      setNotice(`${role} search diagnostic passed.`);
    });
  }

  async function testTelemetry(resourceId: string) {
    if (!selectedSpace) return;
    const entry = snapshot.globalConfiguration.telemetryProfiles.find(
      (candidate) => candidate.id === resourceId,
    );
    if (!entry) return;
    await runMcpDiagnostic(async () => {
      const diagnostic = isTauriRuntime()
        ? await diagnoseManagedTelemetry(selectedSpace.id, entry.label)
        : fixtureRuntimeDiagnostic("telemetry", entry.label);
      setRuntimeDiagnostics((current) => ({
        ...current,
        [`telemetry:${resourceId}`]: diagnostic,
      }));
      setNotice(
        `${entry.label} telemetry diagnostic ${diagnostic.ready ? "passed" : "failed"}.`,
      );
    });
  }

  async function loadExtensionInventory() {
    if (!selectedSpace || selectedSpace.status !== "active") return;
    if (extensionInventorySpaceId !== selectedSpace.id) {
      setExtensionInventory(null);
    }
    setExtensionInventoryBusy(true);
    setFailure("");
    try {
      const inventory = isTauriRuntime()
        ? await getManagedExtensionInventory(selectedSpace.id)
        : fixtureExtensionInventory();
      setExtensionInventory(inventory);
      setExtensionInventorySpaceId(selectedSpace.id);
    } catch (error: unknown) {
      setFailure(
        error instanceof Error
          ? error.message
          : "The extension inventory could not be loaded.",
      );
    } finally {
      setExtensionInventoryBusy(false);
    }
  }

  async function saveMcp() {
    if (!mcpEditor) return;
    const server = managedMcpServer(mcpEditor);
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      resourceId: mcpEditor.resourceId,
      label: mcpEditor.label,
      server,
    };
    await perform(
      () => upsertGlobalMcpServer(request),
      () =>
        fixtureRevision((draft) => {
          const existing = draft.globalConfiguration.mcpServers.find(
            (entry) => entry.id === request.resourceId,
          );
          if (existing) {
            existing.currentRevision += 1;
            existing.label = request.label;
            existing.revisions.push({
              revision: existing.currentRevision,
              value: server,
            });
          } else {
            draft.globalConfiguration.mcpServers.push({
              id: crypto.randomUUID(),
              label: request.label,
              currentRevision: 1,
              archived: false,
              revisions: [{ revision: 1, value: server }],
            });
          }
        }),
      "MCP server saved.",
    );
    setMcpEditor(null);
  }

  async function saveProvider() {
    if (!providerEditor) return;
    const provider = managedProvider(providerEditor);
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      resourceId: providerEditor.resourceId,
      label: providerEditor.label,
      provider,
    };
    await perform(
      () => upsertGlobalProvider(request),
      () =>
        fixtureRevision((draft) => {
          fixtureCatalogUpsert(
            draft.globalConfiguration.providers,
            request.resourceId,
            request.label,
            provider,
          );
        }),
      "Provider saved.",
    );
    setProviderEditor(null);
  }

  async function saveModel() {
    if (!modelEditor) return;
    const model = managedModel(modelEditor);
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      resourceId: modelEditor.resourceId,
      label: modelEditor.label,
      model,
    };
    await perform(
      () => upsertGlobalModel(request),
      () =>
        fixtureRevision((draft) => {
          fixtureCatalogUpsert(
            draft.globalConfiguration.models,
            request.resourceId,
            request.label,
            model,
          );
        }),
      "Model saved.",
    );
    setModelEditor(null);
  }

  async function saveSearch() {
    if (!searchEditor) return;
    const search: ManagedSearchProvider = {
      profile: searchEditor.profile,
      kind: searchEditor.kind,
      endpoint: searchEditor.endpoint,
      credentialId: searchEditor.credentialId || null,
      authHeader:
        searchEditor.kind === "searxng"
          ? searchEditor.authHeader || null
          : null,
      timeoutMs: searchEditor.timeoutMs,
    };
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      resourceId: searchEditor.resourceId,
      label: searchEditor.label,
      search,
    };
    await perform(
      () => upsertGlobalSearchProvider(request),
      () =>
        fixtureRevision((draft) => {
          fixtureCatalogUpsert(
            draft.globalConfiguration.searchProviders,
            request.resourceId,
            request.label,
            search,
          );
        }),
      "Search service saved.",
    );
    setSearchEditor(null);
  }

  async function saveTelemetry() {
    if (!telemetryEditor) return;
    const telemetry = managedTelemetry(telemetryEditor);
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      resourceId: telemetryEditor.resourceId,
      label: telemetryEditor.label,
      telemetry,
    };
    await perform(
      () => upsertGlobalTelemetryProfile(request),
      () =>
        fixtureRevision((draft) => {
          fixtureCatalogUpsert(
            draft.globalConfiguration.telemetryProfiles,
            request.resourceId,
            request.label,
            telemetry,
          );
        }),
      "Telemetry connection saved.",
    );
    setTelemetryEditor(null);
  }

  async function createCredential() {
    if (!credentialLabel.trim()) return;
    const request = {
      expectedRevision: snapshot.globalConfiguration.revision,
      label: credentialLabel.trim(),
      kind: credentialKind,
    };
    await perform(
      () => createManagedCredential(request),
      () =>
        fixtureRevision((draft) => {
          draft.globalConfiguration.credentials.push({
            id: crypto.randomUUID(),
            label: request.label,
            kind: request.kind,
            backend: "desktop",
            createdAtMs: Date.now(),
          });
        }),
      "Credential stored securely.",
    );
    setCredentialLabel("");
  }

  async function rotateCredential(credentialId: string) {
    await perform(
      () =>
        rotateManagedCredential({
          expectedRevision: snapshot.globalConfiguration.revision,
          credentialId,
        }),
      () =>
        fixtureRevision((draft) => {
          const old = draft.globalConfiguration.credentials.find(
            (credential) => credential.id === credentialId,
          )!;
          draft.globalConfiguration.credentials.push({
            ...old,
            id: crypto.randomUUID(),
            createdAtMs: Date.now(),
          });
        }),
      "Credential rotated. Existing configurations keep using the previous value until updated.",
    );
  }

  async function removeCredential(credentialId: string) {
    await perform(
      () =>
        deleteManagedCredential({
          expectedRevision: snapshot.globalConfiguration.revision,
          credentialId,
        }),
      () =>
        fixtureRevision((draft) => {
          draft.globalConfiguration.credentials =
            draft.globalConfiguration.credentials.filter(
              (credential) => credential.id !== credentialId,
            );
        }),
      "Credential deleted.",
    );
  }

  const searchResults = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return [];
    const fields = descriptors
      .filter((descriptor) =>
        `${descriptor.title} ${descriptor.section} ${descriptor.id}`
          .toLowerCase()
          .includes(normalized),
      )
      .map((descriptor) => ({
        id: descriptor.id,
        title: descriptor.title,
        meta: `${descriptor.section} · ${descriptor.id}`,
        scope: "field" as const,
      }));
    const resources = [
      ...snapshot.globalConfiguration.providers.map((entry) => ({
        id: entry.id,
        title: entry.label,
        meta: "Provider",
        scope: "providers" as const,
      })),
      ...snapshot.globalConfiguration.models.map((entry) => ({
        id: entry.id,
        title: entry.label,
        meta: "Model",
        scope: "models" as const,
      })),
      ...snapshot.globalConfiguration.mcpServers.map((entry) => ({
        id: entry.id,
        title: entry.label,
        meta: "MCP server",
        scope: "mcp" as const,
      })),
      ...snapshot.globalConfiguration.credentials.map((credential) => ({
        id: credential.id,
        title: credential.label,
        meta: "Credential",
        scope: "credentials" as const,
      })),
      ...snapshot.globalConfiguration.searchProviders.map((entry) => ({
        id: entry.id,
        title: entry.label,
        meta: "Search service",
        scope: "search" as const,
      })),
      ...snapshot.globalConfiguration.telemetryProfiles.map((entry) => ({
        id: entry.id,
        title: entry.label,
        meta: "Telemetry connection",
        scope: "telemetry" as const,
      })),
    ].filter((item) =>
      `${item.title} ${item.meta}`.toLowerCase().includes(normalized),
    );
    return [...fields, ...resources];
  }, [descriptors, query, snapshot]);

  const externalTargets = desktop.targets.filter(
    (target) => target.kind === "external_daemon",
  );

  return (
    <div className="managed-settings-shell">
      <header className="managed-settings-header">
        <div>
          <p className="surface-breadcrumb">
            Settings /{" "}
            {scope === "global"
              ? "Global configuration"
              : (selectedSpace?.name ?? "Workspace")}
          </p>
          <h2>
            {scope === "global"
              ? "Global configuration"
              : "Workspace configuration"}
          </h2>
          <div
            className="managed-scope-switch"
            role="group"
            aria-label="Configuration scope"
          >
            <button
              type="button"
              className={scope === "global" ? "is-active" : ""}
              aria-pressed={scope === "global"}
              onClick={() => {
                setFocusedFieldId(null);
                setScope("global");
              }}
            >
              <IconWorld size={16} aria-hidden="true" />
              Global
            </button>
            <button
              type="button"
              className={scope === "space" ? "is-active" : ""}
              aria-pressed={scope === "space"}
              onClick={() => {
                setFocusedFieldId(null);
                setScope("space");
              }}
            >
              <IconFolder size={16} aria-hidden="true" />
              Workspace
            </button>
          </div>
        </div>
        <label className="managed-settings-search">
          <IconSearch size={17} aria-hidden="true" />
          <span className="sr-only">Search settings</span>
          <input
            type="search"
            value={query}
            placeholder="Search every setting"
            onChange={(event) => {
              setFocusedFieldId(null);
              setQuery(event.target.value);
            }}
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear settings search"
              onClick={() => {
                setFocusedFieldId(null);
                setQuery("");
              }}
            >
              <IconX size={15} aria-hidden="true" />
            </button>
          ) : null}
        </label>
      </header>

      {failure ? (
        <p className="managed-settings-message is-error" role="alert">
          {failure}
        </p>
      ) : null}
      {notice ? (
        <p className="managed-settings-message is-success" role="status">
          {notice}
        </p>
      ) : null}

      {query ? (
        <SettingsSearchResults
          results={searchResults}
          onOpen={(result) => {
            setQuery("");
            if (result.scope === "field") {
              setScope("space");
              const descriptor = descriptors.find(
                (candidate) => candidate.id === result.id,
              );
              if (!descriptor) return;
              const destination = managedFieldDestination(descriptor);
              setFocusedFieldId(result.id);
              if (destination.section) {
                const section = destination.section;
                setExpandedAdvancedSections((current) => {
                  const next = new Set(current);
                  next.add(section);
                  return next;
                });
              }
              setSpaceTab(destination.tab);
            } else {
              setFocusedFieldId(null);
              setScope("global");
              setGlobalTab(result.scope);
            }
          }}
        />
      ) : scope === "global" ? (
        <>
          <SettingsTabs
            tabs={GLOBAL_TABS}
            active={globalTab}
            onChange={(tab) => setGlobalTab(tab as GlobalTab)}
          />
          <GlobalSettingsBody
            tab={globalTab}
            snapshot={snapshot}
            defaults={defaults}
            setDefaults={setDefaults}
            descriptors={descriptors}
            busy={busy}
            mcpEditor={mcpEditor}
            setMcpEditor={setMcpEditor}
            onSaveMcp={() => void saveMcp()}
            providerEditor={providerEditor}
            setProviderEditor={setProviderEditor}
            onSaveProvider={() => void saveProvider()}
            modelEditor={modelEditor}
            setModelEditor={setModelEditor}
            onSaveModel={() => void saveModel()}
            searchEditor={searchEditor}
            setSearchEditor={setSearchEditor}
            onSaveSearch={() => void saveSearch()}
            telemetryEditor={telemetryEditor}
            setTelemetryEditor={setTelemetryEditor}
            onSaveTelemetry={() => void saveTelemetry()}
            credentialLabel={credentialLabel}
            setCredentialLabel={setCredentialLabel}
            credentialKind={credentialKind}
            setCredentialKind={setCredentialKind}
            onCreateCredential={() => void createCredential()}
            onRotateCredential={(id) => void rotateCredential(id)}
            onDeleteCredential={(id) => void removeCredential(id)}
            onConfigureManaged={onConfigureManaged}
            desktop={desktop}
            connecting={connecting}
            updateChecking={updateChecking}
            updateMessage={updateMessage}
            externalTargets={externalTargets}
            onChooseWorkspace={onChooseWorkspace}
            onRestartManaged={onRestartManaged}
            onAddExternalTarget={onAddExternalTarget}
            onRemoveExternalTarget={onRemoveExternalTarget}
            onSetTerminalEnabled={onSetTerminalEnabled}
            onOpenTerminal={onOpenTerminal}
            onCheckForUpdates={onCheckForUpdates}
            onInstallUpdate={onInstallUpdate}
            onImportCaBundle={onImportCaBundle}
            onRemoveCaBundle={onRemoveCaBundle}
          />
          {globalTab === "defaults" ? (
            <SettingsActionBar
              dirty={defaultsDirty}
              busy={busy}
              failure={failure}
              label="Save global changes"
              onDiscard={() => setDefaults(defaultsDraft(snapshot))}
              onApply={() => void saveDefaults()}
            />
          ) : null}
        </>
      ) : (
        <>
          <div className="space-settings-context">
            <label>
              <span>Workspace</span>
              <DropdownSelect
                value={selectedSpace?.id ?? ""}
                onChange={(event) => setSelectedSpaceId(event.target.value)}
              >
                {snapshot.spaces
                  .filter((candidate) => !candidate.archived)
                  .map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>
                      {candidate.name}
                    </option>
                  ))}
              </DropdownSelect>
            </label>
            {selectedSpace ? <RuntimeStatus space={selectedSpace} /> : null}
            {selectedSpace ? (
              <button
                className="button secondary"
                type="button"
                disabled={busy}
                onClick={() => void inspectRepositoryImport()}
              >
                <IconFileImport size={16} aria-hidden="true" />
                {selectedSpace.configuration.import
                  ? "Re-import config"
                  : "Import config"}
              </button>
            ) : null}
            {selectedSpace?.pendingGlobalRevision ? (
              <button
                className="button primary"
                type="button"
                disabled={busy}
                onClick={() => void applyPendingRevision()}
              >
                <IconRefresh size={16} aria-hidden="true" />
                Review and apply r{selectedSpace.pendingGlobalRevision}
              </button>
            ) : null}
          </div>
          <SettingsTabs
            tabs={SPACE_TABS}
            active={spaceTab}
            onChange={(tab) => {
              setFocusedFieldId(null);
              setSpaceTab(tab as SpaceTab);
            }}
          />
          {selectedSpace ? (
            <SpaceSettingsBody
              tab={spaceTab}
              snapshot={snapshot}
              selectedSpace={selectedSpace}
              draft={space}
              setDraft={setSpace}
              descriptors={descriptors}
              effective={effective}
              focusedFieldId={focusedFieldId}
              expandedAdvancedSections={expandedAdvancedSections}
              onAdvancedSectionToggle={(section, open) => {
                setExpandedAdvancedSections((current) => {
                  if (current.has(section) === open) return current;
                  const next = new Set(current);
                  if (open) next.add(section);
                  else next.delete(section);
                  return next;
                });
                if (
                  !open &&
                  descriptors.find(({ id }) => id === focusedFieldId)
                    ?.section === section
                ) {
                  setFocusedFieldId(null);
                }
              }}
              busy={busy}
              mcpDiagnostics={mcpDiagnostics}
              mcpOauthStatuses={mcpOauthStatuses}
              mcpOauthLogins={mcpOauthLogins}
              mcpOauthCallbacks={mcpOauthCallbacks}
              onMcpOauthCallback={(server, value) =>
                setMcpOauthCallbacks((current) => ({
                  ...current,
                  [server]: value,
                }))
              }
              onTestMcp={testMcpServer}
              onLoadMcpOAuthStatus={loadMcpOAuthStatus}
              onLoginMcpOAuth={loginMcpOAuth}
              onCompleteMcpOAuth={completeMcpOAuth}
              onLogoutMcpOAuth={logoutMcpOAuth}
              runtimeDiagnostics={runtimeDiagnostics}
              onTestRuntimeProfile={testRuntimeProfile}
              onTestSearchRole={testSearchRole}
              onTestTelemetry={testTelemetry}
              extensionInventory={extensionInventory}
              extensionInventoryBusy={extensionInventoryBusy}
              onRefreshExtensionInventory={() => void loadExtensionInventory()}
            />
          ) : (
            <EmptySettings
              icon={<IconFolder size={24} />}
              title="No Workspace selected"
            />
          )}
          <SettingsActionBar
            dirty={spaceDirty}
            busy={busy}
            failure={failure}
            label="Apply Workspace changes"
            onDiscard={() =>
              selectedSpace && setSpace(spaceDraft(selectedSpace))
            }
            onApply={() => void saveSpace()}
          />
          {importProposal ? (
            <RepositoryImportDialog
              proposal={importProposal}
              stage={importStage}
              credentials={snapshot.globalConfiguration.credentials}
              mappings={importMappings}
              conflicts={importConflicts}
              busy={busy}
              onStageChange={setImportStage}
              onMappingsChange={setImportMappings}
              onConflictsChange={setImportConflicts}
              onApply={() => void applyRepositoryImport()}
              onClose={() => setImportProposal(null)}
            />
          ) : null}
        </>
      )}
    </div>
  );
}

function SettingsTabs({
  tabs,
  active,
  onChange,
}: {
  tabs: ReadonlyArray<{ id: string; label: string }>;
  active: string;
  onChange: (tab: string) => void;
}) {
  return (
    <nav className="managed-settings-tabs" aria-label="Settings sections">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          className={active === tab.id ? "is-active" : ""}
          aria-current={active === tab.id ? "page" : undefined}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}

function GlobalSettingsBody({
  tab,
  snapshot,
  defaults,
  setDefaults,
  descriptors,
  busy,
  mcpEditor,
  setMcpEditor,
  onSaveMcp,
  providerEditor,
  setProviderEditor,
  onSaveProvider,
  modelEditor,
  setModelEditor,
  onSaveModel,
  searchEditor,
  setSearchEditor,
  onSaveSearch,
  telemetryEditor,
  setTelemetryEditor,
  onSaveTelemetry,
  credentialLabel,
  setCredentialLabel,
  credentialKind,
  setCredentialKind,
  onCreateCredential,
  onRotateCredential,
  onDeleteCredential,
  onConfigureManaged,
  desktop,
  connecting,
  updateChecking,
  updateMessage,
  externalTargets,
  onChooseWorkspace,
  onRestartManaged,
  onAddExternalTarget,
  onRemoveExternalTarget,
  onSetTerminalEnabled,
  onOpenTerminal,
  onCheckForUpdates,
  onInstallUpdate,
  onImportCaBundle,
  onRemoveCaBundle,
}: {
  tab: GlobalTab;
  snapshot: ManagedSettingsSnapshot;
  defaults: DefaultsDraft;
  setDefaults: (draft: DefaultsDraft) => void;
  descriptors: ManagedFieldDescriptor[];
  busy: boolean;
  mcpEditor: McpEditorDraft | null;
  setMcpEditor: (draft: McpEditorDraft | null) => void;
  onSaveMcp: () => void;
  providerEditor: ProviderEditorDraft | null;
  setProviderEditor: (draft: ProviderEditorDraft | null) => void;
  onSaveProvider: () => void;
  modelEditor: ModelEditorDraft | null;
  setModelEditor: (draft: ModelEditorDraft | null) => void;
  onSaveModel: () => void;
  searchEditor: SearchEditorDraft | null;
  setSearchEditor: (draft: SearchEditorDraft | null) => void;
  onSaveSearch: () => void;
  telemetryEditor: TelemetryEditorDraft | null;
  setTelemetryEditor: (draft: TelemetryEditorDraft | null) => void;
  onSaveTelemetry: () => void;
  credentialLabel: string;
  setCredentialLabel: (label: string) => void;
  credentialKind: ManagedCredentialKind;
  setCredentialKind: (kind: ManagedCredentialKind) => void;
  onCreateCredential: () => void;
  onRotateCredential: (id: string) => void;
  onDeleteCredential: (id: string) => void;
  onConfigureManaged: () => void;
  desktop: DesktopStatus;
  connecting: boolean;
  updateChecking: boolean;
  updateMessage: string;
  externalTargets: RuntimeTarget[];
  onChooseWorkspace: () => void;
  onRestartManaged: () => void;
  onAddExternalTarget: () => void;
  onRemoveExternalTarget: (targetId: string) => void;
  onSetTerminalEnabled: (enabled: boolean) => void;
  onOpenTerminal: (kind: TerminalKind) => void;
  onCheckForUpdates: () => void;
  onInstallUpdate: () => void;
  onImportCaBundle: () => void;
  onRemoveCaBundle: () => void;
}) {
  const global = snapshot.globalConfiguration;
  if (tab === "mcp") {
    return (
      <section className="managed-settings-body" aria-labelledby="mcp-heading">
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Tools and integrations</p>
            <h3 id="mcp-heading">MCP servers</h3>
            <p className="managed-heading-copy">
              Add local processes or HTTP services that provide tools to
              Colossus.
            </p>
          </div>
          <button
            className="button primary"
            type="button"
            onClick={() => setMcpEditor({ ...EMPTY_MCP_DRAFT })}
          >
            <IconPlus size={16} aria-hidden="true" />
            Add MCP server
          </button>
        </div>
        <div className="managed-metric-strip" aria-label="MCP server summary">
          <Metric
            icon={<IconServer size={19} />}
            value={global.mcpServers.length}
            label="Connected servers"
          />
          <Metric
            icon={<IconKey size={19} />}
            value={global.credentials.length}
            label="Credentials available"
          />
          <Metric
            icon={<IconActivityHeartbeat size={19} />}
            value={global.telemetryProfiles.length}
            label="Telemetry connections"
          />
        </div>
        {mcpEditor ? (
          <McpEditor
            draft={mcpEditor}
            credentials={global.credentials}
            busy={busy}
            onChange={setMcpEditor}
            onCancel={() => setMcpEditor(null)}
            onSave={onSaveMcp}
          />
        ) : null}
        <div
          className="managed-resource-table"
          role="table"
          aria-label="Global MCP servers"
        >
          <div className="managed-resource-header" role="row">
            <span role="columnheader">Server</span>
            <span role="columnheader">Transport</span>
            <span role="columnheader">Credential</span>
            <span role="columnheader">Version</span>
            <span role="columnheader">Actions</span>
          </div>
          {global.mcpServers.map((entry) => {
            const server = currentValue(entry);
            const credentialCount =
              Object.keys(server.environmentCredentials).length +
              Object.keys(server.credentialHeaders).length +
              (server.oauth?.clientSecretCredentialId ? 1 : 0);
            return (
              <div className="managed-resource-row" role="row" key={entry.id}>
                <div role="cell">
                  <span className="resource-icon">
                    <IconServer size={18} />
                  </span>
                  <span>
                    <strong>{entry.label}</strong>
                    {entry.label === server.name ? null : (
                      <small>Server ID: {server.name}</small>
                    )}
                  </span>
                </div>
                <span role="cell">{server.transport.replace("_", " ")}</span>
                <span
                  role="cell"
                  className={
                    credentialCount ? "tone-success-text" : "tone-muted-text"
                  }
                >
                  {credentialCount ? "Configured" : "None"}
                </span>
                <span role="cell">v{entry.currentRevision}</span>
                <div role="cell" className="resource-actions">
                  <button
                    className="icon-button"
                    type="button"
                    aria-label={`Edit ${entry.label}`}
                    title={`Edit ${entry.label}`}
                    onClick={() => setMcpEditor(mcpDraft(entry))}
                  >
                    <IconEdit size={17} />
                  </button>
                </div>
              </div>
            );
          })}
          {global.mcpServers.length === 0 ? (
            <EmptySettings
              icon={<IconServer size={24} />}
              title="No MCP servers"
            />
          ) : null}
        </div>
      </section>
    );
  }
  if (tab === "credentials") {
    const credentialConsumers = new Map(
      global.credentials.map((credential) => [
        credential.id,
        managedCredentialConsumers(snapshot, credential.id),
      ]),
    );
    const nativeCredentialCount = global.credentials.filter(
      (credential) => credential.backend === "desktop",
    ).length;
    const referencedCredentialCount = [...credentialConsumers.values()].filter(
      (consumers) => consumers.length > 0,
    ).length;
    return (
      <section
        className="managed-settings-body credentials-settings"
        aria-labelledby="credentials-heading"
      >
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Secure storage</p>
            <h3 id="credentials-heading">Credentials</h3>
            <p className="credentials-heading-copy">
              Save API keys, tokens, and client secrets for providers, search,
              and MCP servers.
            </p>
          </div>
          <div
            className="credential-security-note"
            aria-label="Credential security"
          >
            <IconShield size={18} aria-hidden="true" />
            <span>
              <strong>Stored securely on this device</strong>
              <small>
                Values are entered in a system dialog and are not shown again.
              </small>
            </span>
          </div>
        </div>
        <div
          className="managed-metric-strip credentials-metric-strip"
          aria-label="Credential summary"
        >
          <Metric
            icon={<IconKey size={19} />}
            value={global.credentials.length}
            label="Saved credentials"
          />
          <Metric
            icon={<IconShield size={19} />}
            value={nativeCredentialCount}
            label="Stored securely"
          />
          <Metric
            icon={<IconNetwork size={19} />}
            value={referencedCredentialCount}
            label="In use"
          />
        </div>
        <section
          className="credential-create-card"
          aria-labelledby="create-credential-heading"
        >
          <div className="credential-card-heading">
            <span className="resource-icon">
              <IconPlus size={18} aria-hidden="true" />
            </span>
            <div>
              <h4 id="create-credential-heading">Add credential</h4>
              <p>
                Give the credential a name and choose how it will be used. A
                secure system dialog asks for the value next.
              </p>
            </div>
          </div>
          <form
            className="credential-create-row"
            onSubmit={(event) => {
              event.preventDefault();
              onCreateCredential();
            }}
          >
            <label>
              <span>Display label</span>
              <input
                value={credentialLabel}
                onChange={(event) => setCredentialLabel(event.target.value)}
                placeholder="For example, GitHub workspace token"
                aria-describedby="credential-label-help"
                required
              />
              <small id="credential-label-help">
                Use a name you will recognize when choosing a credential. Do not
                enter the secret here.
              </small>
            </label>
            <label>
              <span>Credential type</span>
              <DropdownSelect
                value={credentialKind}
                aria-describedby="credential-kind-help"
                onChange={(event) =>
                  setCredentialKind(event.target.value as ManagedCredentialKind)
                }
              >
                <option value="api_key">API key</option>
                <option value="bearer_token">Bearer token</option>
                <option value="client_secret">OAuth client secret</option>
                <option value="generic_secret">Generic secret</option>
              </DropdownSelect>
              <small id="credential-kind-help">
                Helps integrations send the credential in the expected format.
              </small>
            </label>
            <button
              className="button primary"
              type="submit"
              disabled={busy || !credentialLabel.trim()}
            >
              <IconPlus size={16} aria-hidden="true" /> Add credential
            </button>
          </form>
        </section>
        <section
          className="credential-inventory"
          aria-labelledby="stored-credentials-heading"
        >
          <div className="credential-inventory-heading">
            <div>
              <h4 id="stored-credentials-heading">Stored credentials</h4>
              <p>
                Rotating saves a new value. Existing configurations keep using
                the previous value until they are updated.
              </p>
            </div>
            <span
              className="credential-count"
              aria-label={`${global.credentials.length} credentials`}
            >
              {global.credentials.length}
            </span>
          </div>
          <div className="managed-list credential-list" role="list">
            {global.credentials.map((credential) => {
              const consumers = credentialConsumers.get(credential.id) ?? [];
              const native = credential.backend === "desktop";
              return (
                <div
                  className="managed-list-row"
                  key={credential.id}
                  role="listitem"
                >
                  <span className="resource-icon">
                    <IconKey size={18} aria-hidden="true" />
                  </span>
                  <div>
                    <strong>{credential.label}</strong>
                    <small>
                      {credentialKindLabel(credential.kind)} ·{" "}
                      {native
                        ? "Secure system storage"
                        : "External credential source"}
                    </small>
                  </div>
                  <div className="credential-row-meta">
                    <span
                      className={`status-chip${consumers.length ? " tone-success" : ""}`}
                      title={
                        consumers.length
                          ? consumers.join("\n")
                          : "No active configuration uses this credential."
                      }
                    >
                      <IconNetwork size={13} aria-hidden="true" />
                      {consumers.length
                        ? `Used by ${consumers.length}`
                        : "Not referenced"}
                    </span>
                    <span
                      className={`status-chip${native ? " tone-success" : ""}`}
                    >
                      <IconLock size={13} aria-hidden="true" />
                      {native ? "Stored securely" : "External reference"}
                    </span>
                  </div>
                  <div className="resource-actions">
                    <button
                      className="button secondary"
                      type="button"
                      disabled={busy || !native}
                      aria-label={`Rotate ${credential.label}`}
                      title={`Rotate ${credential.label}`}
                      onClick={() => onRotateCredential(credential.id)}
                    >
                      <IconRefresh size={15} aria-hidden="true" /> Rotate
                    </button>
                    <button
                      className="icon-button danger-icon-button"
                      type="button"
                      disabled={busy || !native}
                      aria-label={`Delete ${credential.label}`}
                      title={`Delete ${credential.label}`}
                      onClick={() => onDeleteCredential(credential.id)}
                    >
                      <IconTrash size={17} aria-hidden="true" />
                    </button>
                  </div>
                </div>
              );
            })}
            {global.credentials.length === 0 ? (
              <EmptySettings
                icon={<IconKey size={24} />}
                title="No credentials"
              />
            ) : null}
          </div>
        </section>
      </section>
    );
  }
  if (tab === "defaults") {
    return (
      <section className="managed-settings-body">
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Workspace starting point</p>
            <h3>Global defaults</h3>
            <p className="managed-heading-copy">
              Set the values workspaces use unless they override them.
            </p>
          </div>
        </div>
        <AuthorityControls
          access={defaults.accessProfile}
          boundary={defaults.executionBoundary}
          terminal={defaults.terminalEnabled}
          onAccess={(value) =>
            setDefaults({ ...defaults, accessProfile: value })
          }
          onBoundary={(value) =>
            setDefaults({ ...defaults, executionBoundary: value })
          }
          onTerminal={(value) =>
            setDefaults({ ...defaults, terminalEnabled: value })
          }
        />
        <FieldGrid
          descriptors={descriptors.filter((descriptor) => !descriptor.advanced)}
          values={defaults.fields}
          effective={new Map()}
          scope="global"
          onChange={(id, value) =>
            setDefaults({
              ...defaults,
              fields: { ...defaults.fields, [id]: value },
            })
          }
          onInherit={(id) => {
            const fields = { ...defaults.fields };
            delete fields[id];
            setDefaults({ ...defaults, fields });
          }}
        />
        <details className="managed-advanced-disclosure">
          <summary>
            <span>Advanced defaults</span>
            <small>
              {descriptors.filter((descriptor) => descriptor.advanced).length}{" "}
              settings
            </small>
          </summary>
          <FieldGrid
            descriptors={descriptors.filter(
              (descriptor) => descriptor.advanced,
            )}
            values={defaults.fields}
            effective={new Map()}
            scope="global"
            onChange={(id, value) =>
              setDefaults({
                ...defaults,
                fields: { ...defaults.fields, [id]: value },
              })
            }
            onInherit={(id) => {
              const fields = { ...defaults.fields };
              delete fields[id];
              setDefaults({ ...defaults, fields });
            }}
          />
        </details>
      </section>
    );
  }
  if (tab === "models") {
    const activeModels = global.models.filter((entry) => !entry.archived);
    const firstActiveProvider = global.providers.find(
      (entry) => !entry.archived,
    );
    const modelConsumers = new Map(
      activeModels.map((entry) => [
        entry.id,
        managedModelConsumers(snapshot, entry.id),
      ]),
    );
    const providersInUse = new Set(
      activeModels.map((entry) => currentValue(entry).providerProfile),
    ).size;
    const routedWorkspaceCount = new Set([...modelConsumers.values()].flat())
      .size;
    return (
      <section
        className="managed-settings-body models-settings"
        aria-labelledby="models-heading"
      >
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">AI models</p>
            <h3 id="models-heading">Models</h3>
            <p className="models-heading-copy">
              Choose the models available to workspaces, their token limits, and
              the features they support.
            </p>
          </div>
          <div className="model-contract-note" aria-label="Model capabilities">
            <IconShield size={18} aria-hidden="true" />
            <span>
              <strong>Supported features</strong>
              <small>
                Turn on only the features this model supports: tools, streaming,
                and images.
              </small>
            </span>
          </div>
        </div>
        <div
          className="managed-metric-strip models-metric-strip"
          aria-label="Model summary"
        >
          <Metric
            icon={<IconCpu size={19} />}
            value={activeModels.length}
            label="Active models"
          />
          <Metric
            icon={<IconCloud size={19} />}
            value={providersInUse}
            label="Providers in use"
          />
          <Metric
            icon={<IconRoute size={19} />}
            value={routedWorkspaceCount}
            label="Workspaces using models"
          />
        </div>
        <div className="model-catalog-toolbar">
          <div>
            <h4>Model inventory</h4>
            <p>Add models here, then choose which workspaces can use them.</p>
          </div>
          <button
            className="button primary"
            type="button"
            disabled={Boolean(modelEditor)}
            onClick={() =>
              setModelEditor({
                ...EMPTY_MODEL_DRAFT,
                providerProfile: firstActiveProvider
                  ? currentValue(firstActiveProvider).profile
                  : "",
              })
            }
          >
            <IconPlus size={16} aria-hidden="true" /> Add model
          </button>
        </div>
        {modelEditor ? (
          <ModelEditor
            draft={modelEditor}
            providers={global.providers.filter((entry) => !entry.archived)}
            busy={busy}
            onChange={setModelEditor}
            onCancel={() => setModelEditor(null)}
            onSave={onSaveModel}
          />
        ) : null}
        <section
          className="model-inventory"
          aria-labelledby="configured-models-heading"
        >
          <div className="model-inventory-heading">
            <div>
              <h4 id="configured-models-heading">Configured models</h4>
              <p>
                Compare each model's provider, token limits, and supported
                features.
              </p>
            </div>
            <span
              className="credential-count"
              aria-label={`${activeModels.length} models`}
            >
              {activeModels.length}
            </span>
          </div>
          <div className="managed-list model-list" role="list">
            {activeModels.map((entry) => {
              const model = currentValue(entry);
              const consumers = modelConsumers.get(entry.id) ?? [];
              const provider = global.providers.find(
                (candidate) =>
                  !candidate.archived &&
                  currentValue(candidate).profile === model.providerProfile,
              );
              const enabledCapabilities = [
                model.capabilities.toolCalls ? "Tools" : null,
                model.capabilities.streaming ? "Streaming" : null,
                model.capabilities.imageInputs ? "Images" : null,
              ].filter((value): value is string => Boolean(value));
              return (
                <div
                  className="managed-list-row model-row"
                  key={entry.id}
                  role="listitem"
                >
                  <span className="resource-icon">
                    <IconCpu size={18} aria-hidden="true" />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {entry.label === model.profile
                        ? model.model
                        : `${model.profile} · ${model.model}`}
                    </small>
                    <small className="model-provider-route">
                      Provider · {provider?.label ?? model.providerProfile}
                    </small>
                  </div>
                  <div
                    className="model-capability-list"
                    aria-label="Capabilities"
                  >
                    {(enabledCapabilities.length
                      ? enabledCapabilities
                      : ["Text only"]
                    ).map((capability) => (
                      <span
                        className="status-chip tone-success"
                        key={capability}
                      >
                        {capability}
                      </span>
                    ))}
                  </div>
                  <div className="model-row-meta">
                    <span
                      className={`status-chip${consumers.length ? " tone-success" : ""}`}
                      title={
                        consumers.length
                          ? consumers.join("\n")
                          : "No active workspace references this model."
                      }
                    >
                      <IconRoute size={13} aria-hidden="true" />
                      {consumers.length
                        ? `Used by ${consumers.length}`
                        : "Not used"}
                    </span>
                    <span className="status-chip tone-neutral">
                      {compactTokenCount(model.contextWindowTokens)} context ·{" "}
                      {compactTokenCount(model.maxOutputTokens)} output · v
                      {entry.currentRevision}
                    </span>
                  </div>
                  <div className="resource-actions">
                    <button
                      className="button secondary"
                      type="button"
                      aria-label={`Edit ${entry.label}`}
                      onClick={() => setModelEditor(modelDraft(entry))}
                    >
                      <IconEdit size={15} aria-hidden="true" /> Edit
                    </button>
                  </div>
                </div>
              );
            })}
            {activeModels.length === 0 ? (
              <EmptySettings icon={<IconCpu size={24} />} title="No models" />
            ) : null}
          </div>
        </section>
      </section>
    );
  }
  if (tab === "providers") {
    const activeProviders = global.providers.filter((entry) => !entry.archived);
    const providerConsumers = new Map(
      activeProviders.map((entry) => [
        entry.id,
        managedProviderConsumers(snapshot, entry.id),
      ]),
    );
    const routedModelCount = [...providerConsumers.values()].reduce(
      (total, consumers) => total + consumers.length,
      0,
    );
    const credentialBindingCount = activeProviders.filter((entry) =>
      Boolean(currentValue(entry).credentialId),
    ).length;
    return (
      <section
        className="managed-settings-body providers-settings"
        aria-labelledby="providers-heading"
      >
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">AI connections</p>
            <h3 id="providers-heading">Providers</h3>
            <p className="providers-heading-copy">
              Connect the services and accounts Colossus uses to access AI
              models.
            </p>
          </div>
          <div
            className="provider-security-note"
            aria-label="Provider credential safety"
          >
            <IconLock size={18} aria-hidden="true" />
            <span>
              <strong>Credentials stay protected</strong>
              <small>
                Colossus stores which credential to use, not its secret value.
              </small>
            </span>
          </div>
        </div>
        <div
          className="managed-metric-strip providers-metric-strip"
          aria-label="Provider summary"
        >
          <Metric
            icon={<IconCloud size={19} />}
            value={activeProviders.length}
            label="Active providers"
          />
          <Metric
            icon={<IconCpu size={19} />}
            value={routedModelCount}
            label="Models using them"
          />
          <Metric
            icon={<IconKey size={19} />}
            value={credentialBindingCount}
            label="Credentials used"
          />
        </div>
        <div className="provider-catalog-toolbar">
          <div>
            <h4>Provider connections</h4>
            <p>
              Add a provider once, then reuse the connection for multiple
              models.
            </p>
          </div>
          <button
            className="button primary"
            type="button"
            disabled={Boolean(providerEditor)}
            onClick={() => setProviderEditor({ ...EMPTY_PROVIDER_DRAFT })}
          >
            <IconPlus size={16} /> Add provider
          </button>
        </div>
        {providerEditor ? (
          <ProviderEditor
            draft={providerEditor}
            credentials={global.credentials}
            busy={busy}
            onChange={setProviderEditor}
            onCancel={() => setProviderEditor(null)}
            onSave={onSaveProvider}
          />
        ) : null}
        <section
          className="provider-inventory"
          aria-labelledby="configured-providers-heading"
        >
          <div className="provider-inventory-heading">
            <div>
              <h4 id="configured-providers-heading">Providers</h4>
              <p>
                Compare each provider's endpoint, sign-in method, model usage,
                and timeout.
              </p>
            </div>
            <span
              className="credential-count"
              aria-label={`${activeProviders.length} providers`}
            >
              {activeProviders.length}
            </span>
          </div>
          <div className="managed-list provider-list" role="list">
            {activeProviders.map((entry) => {
              const provider = currentValue(entry);
              const consumers = providerConsumers.get(entry.id) ?? [];
              const codex = provider.kind === "open_ai_codex";
              const timeout = provider.timeoutMs
                ? `${Math.round(provider.timeoutMs / 1_000)}s timeout`
                : "Default timeout";
              return (
                <div
                  className="managed-list-row provider-row"
                  key={entry.id}
                  role="listitem"
                >
                  <span className="resource-icon">
                    <IconCloud size={18} aria-hidden="true" />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {entry.label === provider.profile
                        ? providerEndpointLabel(provider)
                        : `${provider.profile} · ${providerEndpointLabel(provider)}`}
                    </small>
                  </div>
                  <span className="status-chip tone-neutral provider-adapter-chip">
                    {providerAdapterLabel(provider.kind)}
                  </span>
                  <div className="provider-row-meta">
                    <span
                      className={`status-chip${provider.credentialId || codex ? " tone-success" : ""}`}
                    >
                      <IconLock size={13} aria-hidden="true" />
                      {codex
                        ? "Codex account"
                        : provider.credentialId
                          ? "Credential attached"
                          : "No credential"}
                    </span>
                    <span
                      className={`status-chip${consumers.length ? " tone-success" : ""}`}
                      title={
                        consumers.length
                          ? consumers.join("\n")
                          : "No active model references this provider."
                      }
                    >
                      <IconRoute size={13} aria-hidden="true" />
                      {consumers.length
                        ? `Used by ${consumers.length}`
                        : "No models"}
                    </span>
                    <span className="status-chip tone-neutral">
                      {timeout} · v{entry.currentRevision}
                    </span>
                  </div>
                  <div className="resource-actions">
                    <button
                      className="button secondary"
                      type="button"
                      aria-label={`Edit ${entry.label}`}
                      onClick={() => setProviderEditor(providerDraft(entry))}
                    >
                      <IconEdit size={15} aria-hidden="true" /> Edit
                    </button>
                  </div>
                </div>
              );
            })}
            {activeProviders.length === 0 ? (
              <EmptySettings
                icon={<IconCloud size={24} />}
                title="No providers"
              />
            ) : null}
          </div>
        </section>
      </section>
    );
  }
  if (tab === "search") {
    const activeSearchProfiles = global.searchProviders.filter(
      (entry) => !entry.archived,
    );
    const searchConsumers = new Map(
      activeSearchProfiles.map((entry) => [
        entry.id,
        managedSearchProfileConsumers(snapshot, entry.id),
      ]),
    );
    const authenticatedProfileCount = activeSearchProfiles.filter((entry) =>
      Boolean(currentValue(entry).credentialId),
    ).length;
    const routedWorkspaceCount = new Set([...searchConsumers.values()].flat())
      .size;
    return (
      <section
        className="managed-settings-body search-settings"
        aria-labelledby="search-profiles-heading"
      >
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Web search</p>
            <h3 id="search-profiles-heading">Search services</h3>
            <p className="search-heading-copy">
              Choose the search services Colossus can use and which workspaces
              can access them.
            </p>
          </div>
          <div
            className="search-routing-note"
            aria-label="Search workspace availability"
          >
            <IconRoute size={18} aria-hidden="true" />
            <span>
              <strong>Available by workspace</strong>
              <small>
                Adding a service here does not enable it automatically.
              </small>
            </span>
          </div>
        </div>
        <div
          className="managed-metric-strip search-metric-strip"
          aria-label="Search service summary"
        >
          <Metric
            icon={<IconSearch size={19} />}
            value={activeSearchProfiles.length}
            label="Search services"
          />
          <Metric
            icon={<IconLock size={19} />}
            value={authenticatedProfileCount}
            label="Use credentials"
          />
          <Metric
            icon={<IconNetwork size={19} />}
            value={routedWorkspaceCount}
            label="Workspaces using search"
          />
        </div>
        <div className="search-profile-toolbar">
          <div>
            <h4>Configured search services</h4>
            <p>Choose each service in the workspaces that need it.</p>
          </div>
          <button
            className="button primary"
            type="button"
            disabled={Boolean(searchEditor)}
            onClick={() => setSearchEditor({ ...EMPTY_SEARCH_DRAFT })}
          >
            <IconPlus size={16} aria-hidden="true" /> Add search service
          </button>
        </div>
        {searchEditor ? (
          <SearchEditor
            draft={searchEditor}
            credentials={global.credentials}
            busy={busy}
            onChange={setSearchEditor}
            onCancel={() => setSearchEditor(null)}
            onSave={onSaveSearch}
          />
        ) : null}
        <section
          className="search-profile-inventory"
          aria-labelledby="configured-search-profiles-heading"
        >
          <div className="search-inventory-heading">
            <div>
              <h4 id="configured-search-profiles-heading">
                Configured services
              </h4>
              <p>
                See where each service connects, whether it uses a credential,
                and which workspaces use it.
              </p>
            </div>
            <span
              className="credential-count"
              aria-label={`${activeSearchProfiles.length} search services`}
            >
              {activeSearchProfiles.length}
            </span>
          </div>
          <div className="managed-list search-profile-list" role="list">
            {activeSearchProfiles.map((entry) => {
              const search = currentValue(entry);
              const credential = global.credentials.find(
                (candidate) => candidate.id === search.credentialId,
              );
              const consumers = searchConsumers.get(entry.id) ?? [];
              return (
                <div
                  className="managed-list-row search-profile-row"
                  key={entry.id}
                  role="listitem"
                >
                  <span className="resource-icon">
                    <IconSearch size={18} aria-hidden="true" />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {entry.label === search.profile
                        ? searchAdapterLabel(search.kind)
                        : `${searchAdapterLabel(search.kind)} · ${search.profile}`}
                    </small>
                    <small
                      className="search-profile-endpoint"
                      title={search.endpoint}
                    >
                      {search.endpoint}
                    </small>
                  </div>
                  <div className="search-row-meta">
                    <span
                      className={`status-chip${credential ? " tone-success" : ""}`}
                      title={
                        credential
                          ? `Credential · ${credential.label}`
                          : "No credential is attached to this search service."
                      }
                    >
                      <IconLock size={13} aria-hidden="true" />
                      {credential ? "Credential attached" : "No credential"}
                    </span>
                    <span
                      className={`status-chip${consumers.length ? " tone-success" : ""}`}
                      title={
                        consumers.length
                          ? consumers.join("\n")
                          : "No active workspace uses this search service."
                      }
                    >
                      <IconRoute size={13} aria-hidden="true" />
                      {consumers.length
                        ? `Used by ${consumers.length}`
                        : "Not used"}
                    </span>
                    <span className="status-chip tone-neutral">
                      {search.timeoutMs / 1_000}s · v{entry.currentRevision}
                    </span>
                  </div>
                  <div className="resource-actions">
                    <button
                      className="button secondary"
                      type="button"
                      aria-label={`Edit ${entry.label}`}
                      onClick={() => setSearchEditor(searchDraft(entry))}
                    >
                      <IconEdit size={15} aria-hidden="true" /> Edit
                    </button>
                  </div>
                </div>
              );
            })}
            {activeSearchProfiles.length === 0 ? (
              <EmptySettings
                icon={<IconSearch size={24} />}
                title="No search services"
              />
            ) : null}
          </div>
        </section>
      </section>
    );
  }
  if (tab === "telemetry") {
    const activeTelemetryProfiles = global.telemetryProfiles.filter(
      (entry) => !entry.archived,
    );
    const telemetryConsumers = new Map(
      activeTelemetryProfiles.map((entry) => [
        entry.id,
        managedTelemetryConsumers(snapshot, entry.id),
      ]),
    );
    const routedWorkspaceCount = new Set(
      [...telemetryConsumers.values()].flat(),
    ).size;
    const enabledSignalCount = activeTelemetryProfiles.reduce(
      (total, entry) => total + telemetrySignalCount(currentValue(entry)),
      0,
    );
    return (
      <section
        className="managed-settings-body telemetry-settings"
        aria-labelledby="telemetry-profiles-heading"
      >
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Monitoring and audit</p>
            <h3 id="telemetry-profiles-heading">Telemetry connections</h3>
            <p className="telemetry-heading-copy">
              Send traces, metrics, and logs to your monitoring system, and
              choose what audit information is included.
            </p>
          </div>
          <div
            className="telemetry-disclosure-note"
            aria-label="Telemetry data warning"
          >
            <IconShield size={18} aria-hidden="true" />
            <span>
              <strong>Review shared data</strong>
              <small>
                Full audit records can include prompts, responses, and tool
                input or output.
              </small>
            </span>
          </div>
        </div>
        <div
          className="managed-metric-strip telemetry-metric-strip"
          aria-label="Telemetry summary"
        >
          <Metric
            icon={<IconActivityHeartbeat size={19} />}
            value={activeTelemetryProfiles.length}
            label="Telemetry connections"
          />
          <Metric
            icon={<IconRoute size={19} />}
            value={routedWorkspaceCount}
            label="Workspaces using telemetry"
          />
          <Metric
            icon={<IconNetwork size={19} />}
            value={enabledSignalCount}
            label="Signals enabled"
          />
        </div>
        <div className="telemetry-catalog-toolbar">
          <div>
            <h4>Configured connections</h4>
            <p>See what each connection sends and which workspaces use it.</p>
          </div>
          <button
            className="button primary"
            type="button"
            disabled={Boolean(telemetryEditor)}
            onClick={() => setTelemetryEditor({ ...EMPTY_TELEMETRY_DRAFT })}
          >
            <IconPlus size={16} aria-hidden="true" />
            Add telemetry connection
          </button>
        </div>
        {telemetryEditor ? (
          <TelemetryEditor
            draft={telemetryEditor}
            busy={busy}
            onChange={setTelemetryEditor}
            onSave={onSaveTelemetry}
            onCancel={() => setTelemetryEditor(null)}
          />
        ) : null}
        <section
          className="telemetry-inventory"
          aria-labelledby="configured-telemetry-heading"
        >
          <div className="telemetry-inventory-heading">
            <div>
              <h4 id="configured-telemetry-heading">Telemetry connections</h4>
              <p>
                Compare each destination, exported signals, and audit record
                content.
              </p>
            </div>
            <span
              className="credential-count"
              aria-label={`${activeTelemetryProfiles.length} telemetry connections`}
            >
              {activeTelemetryProfiles.length}
            </span>
          </div>
          <div className="managed-list telemetry-list" role="list">
            {activeTelemetryProfiles.map((entry) => {
              const telemetry = currentValue(entry);
              const consumers = telemetryConsumers.get(entry.id) ?? [];
              const signalCount = telemetrySignalCount(telemetry);
              return (
                <div
                  className="managed-list-row telemetry-row"
                  key={entry.id}
                  role="listitem"
                >
                  <span className="resource-icon">
                    <IconActivityHeartbeat size={18} aria-hidden="true" />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {telemetryEndpointLabel(telemetry)} · {telemetry.name}
                    </small>
                  </div>
                  <span className="status-chip tone-neutral telemetry-protocol-chip">
                    {telemetryProtocolLabel(telemetry.protocol)}
                  </span>
                  <div className="telemetry-row-meta">
                    <span
                      className={`status-chip${signalCount ? " tone-success" : ""}`}
                    >
                      <IconActivityHeartbeat size={13} aria-hidden="true" />
                      {signalCount
                        ? `${signalCount} OTLP signal${signalCount === 1 ? "" : "s"}`
                        : "No OTLP signals"}
                    </span>
                    <span
                      className={`status-chip${telemetry.journalPayloads === "full" ? " tone-attention" : ""}`}
                    >
                      <IconShield size={13} aria-hidden="true" />
                      {telemetryJournalLabel(telemetry.journalPayloads)}
                    </span>
                    <span
                      className={`status-chip${consumers.length ? " tone-success" : ""}`}
                      title={
                        consumers.length
                          ? consumers.join("\n")
                          : "No active workspace uses this telemetry connection."
                      }
                    >
                      <IconRoute size={13} aria-hidden="true" />
                      {consumers.length
                        ? `Used by ${consumers.length}`
                        : "Not used"}
                    </span>
                    <span className="status-chip tone-neutral">
                      {Math.round(telemetry.timeoutMs / 1_000)}s · v
                      {entry.currentRevision}
                    </span>
                  </div>
                  <div className="resource-actions">
                    <button
                      className="button secondary"
                      type="button"
                      aria-label={`Edit ${entry.label}`}
                      onClick={() => setTelemetryEditor(telemetryDraft(entry))}
                    >
                      <IconEdit size={15} aria-hidden="true" /> Edit
                    </button>
                  </div>
                </div>
              );
            })}
            {activeTelemetryProfiles.length === 0 ? (
              <EmptySettings
                icon={<IconActivityHeartbeat size={24} />}
                title="No telemetry connections"
              />
            ) : null}
          </div>
        </section>
      </section>
    );
  }
  return (
    <DesktopSettings
      desktop={desktop}
      connecting={connecting}
      updateChecking={updateChecking}
      updateMessage={updateMessage}
      externalTargets={externalTargets}
      onChooseWorkspace={onChooseWorkspace}
      onConfigureManaged={onConfigureManaged}
      onRestartManaged={onRestartManaged}
      onAddExternalTarget={onAddExternalTarget}
      onRemoveExternalTarget={onRemoveExternalTarget}
      onSetTerminalEnabled={onSetTerminalEnabled}
      onOpenTerminal={onOpenTerminal}
      onCheckForUpdates={onCheckForUpdates}
      onInstallUpdate={onInstallUpdate}
      onImportCaBundle={onImportCaBundle}
      onRemoveCaBundle={onRemoveCaBundle}
    />
  );
}

export function SpaceSettingsBody({
  tab,
  snapshot,
  selectedSpace,
  draft,
  setDraft,
  descriptors,
  effective,
  focusedFieldId,
  expandedAdvancedSections,
  onAdvancedSectionToggle,
  busy,
  mcpDiagnostics,
  mcpOauthStatuses,
  mcpOauthLogins,
  mcpOauthCallbacks,
  onMcpOauthCallback,
  onTestMcp,
  onLoadMcpOAuthStatus,
  onLoginMcpOAuth,
  onCompleteMcpOAuth,
  onLogoutMcpOAuth,
  runtimeDiagnostics,
  onTestRuntimeProfile,
  onTestSearchRole,
  onTestTelemetry,
  extensionInventory,
  extensionInventoryBusy,
  onRefreshExtensionInventory,
}: {
  tab: SpaceTab;
  snapshot: ManagedSettingsSnapshot;
  selectedSpace: ManagedSpaceConfigurationSnapshot;
  draft: SpaceDraft;
  setDraft: (draft: SpaceDraft) => void;
  descriptors: ManagedFieldDescriptor[];
  effective: Map<string, { value: unknown; source: string }>;
  focusedFieldId: string | null;
  expandedAdvancedSections: ReadonlySet<string>;
  onAdvancedSectionToggle: (section: string, open: boolean) => void;
  busy: boolean;
  mcpDiagnostics: Record<string, ManagedMcpDiagnostic>;
  mcpOauthStatuses: Record<string, ManagedMcpOAuthStatus>;
  mcpOauthLogins: Record<string, ManagedMcpOAuthLogin>;
  mcpOauthCallbacks: Record<string, string>;
  onMcpOauthCallback: (server: string, value: string) => void;
  onTestMcp: (server: string) => void;
  onLoadMcpOAuthStatus: (server: string) => void;
  onLoginMcpOAuth: (server: string) => void;
  onCompleteMcpOAuth: (server: string) => void;
  onLogoutMcpOAuth: (server: string) => void;
  runtimeDiagnostics: Record<string, ManagedRuntimeDiagnostic>;
  onTestRuntimeProfile: (kind: "provider" | "model", profile: string) => void;
  onTestSearchRole: (role: "agent" | "research") => void;
  onTestTelemetry: (resourceId: string) => void;
  extensionInventory: ManagedExtensionInventory | null;
  extensionInventoryBusy: boolean;
  onRefreshExtensionInventory: () => void;
}) {
  if (tab === "mcp") {
    return (
      <section className="managed-settings-layout">
        <div className="managed-settings-body">
          <div className="managed-section-heading">
            <div>
              <p className="eyebrow">Global resources</p>
              <h3>MCP servers</h3>
            </div>
            <span>{draft.selectedMcp.length} enabled</span>
          </div>
          <div className="managed-list mcp-selection-list">
            {snapshot.globalConfiguration.mcpServers.map((entry) => {
              const server = currentValue(entry);
              const enabled = draft.selectedMcp.includes(entry.id);
              const diagnostic = mcpDiagnostics[server.name];
              const oauthStatus = mcpOauthStatuses[server.name];
              const oauthLogin = mcpOauthLogins[server.name];
              return (
                <div className="managed-mcp-resource" key={entry.id}>
                  <div className="managed-list-row mcp-diagnostic-row">
                    <span className="resource-icon">
                      <IconServer size={18} />
                    </span>
                    <div>
                      <strong>{entry.label}</strong>
                      <small>
                        {server.transport.replace("_", " ")} ·{" "}
                        {server.allowedTools.length} allowed tools
                      </small>
                    </div>
                    <span
                      className={`status-chip ${diagnostic?.healthy ? "tone-success" : enabled ? "tone-success" : "tone-neutral"}`}
                    >
                      {diagnostic?.healthy
                        ? "Healthy"
                        : enabled
                          ? "Enabled"
                          : "Available"}
                    </span>
                    <div className="resource-actions">
                      <button
                        className="button secondary"
                        type="button"
                        disabled={
                          busy || !enabled || selectedSpace.status !== "active"
                        }
                        onClick={() => onTestMcp(server.name)}
                      >
                        <IconActivityHeartbeat size={15} />
                        Test
                      </button>
                      {server.oauth ? (
                        <button
                          className="button secondary"
                          type="button"
                          disabled={
                            busy ||
                            !enabled ||
                            selectedSpace.status !== "active"
                          }
                          onClick={() => onLoadMcpOAuthStatus(server.name)}
                        >
                          <IconKey size={15} />
                          OAuth
                        </button>
                      ) : null}
                    </div>
                    <SwitchInput
                      checked={enabled}
                      aria-label={`Enable ${entry.label}`}
                      onChange={(event) =>
                        setDraft({
                          ...draft,
                          selectedMcp: event.target.checked
                            ? [...draft.selectedMcp, entry.id]
                            : draft.selectedMcp.filter((id) => id !== entry.id),
                        })
                      }
                    />
                  </div>
                  {diagnostic ? (
                    <div className="mcp-diagnostic-detail" role="status">
                      <strong>
                        {diagnostic.tools.length} tools discovered
                      </strong>
                      <span>
                        {diagnostic.tools.map((tool) => tool.name).join(", ") ||
                          "No allowlisted tools"}
                      </span>
                    </div>
                  ) : null}
                  {oauthStatus || oauthLogin ? (
                    <div className="mcp-oauth-detail">
                      <span
                        className={`status-chip ${oauthStatus?.authenticated ? "tone-success" : "tone-neutral"}`}
                      >
                        {oauthStatus?.authenticated
                          ? "Signed in"
                          : "Signed out"}
                      </span>
                      {oauthStatus?.authenticated ? (
                        <button
                          className="button secondary"
                          type="button"
                          disabled={busy}
                          onClick={() => onLogoutMcpOAuth(server.name)}
                        >
                          Sign out
                        </button>
                      ) : (
                        <button
                          className="button secondary"
                          type="button"
                          disabled={busy}
                          onClick={() => onLoginMcpOAuth(server.name)}
                        >
                          Sign in
                        </button>
                      )}
                      {oauthLogin ? (
                        <>
                          <a
                            className="button secondary"
                            href={oauthLogin.authorizationUrl}
                            target="_blank"
                            rel="noreferrer"
                          >
                            Open authorization
                          </a>
                          <input
                            type="url"
                            value={mcpOauthCallbacks[server.name] ?? ""}
                            aria-label={`OAuth callback URL for ${entry.label}`}
                            placeholder={oauthLogin.callbackUrl}
                            onChange={(event) =>
                              onMcpOauthCallback(
                                server.name,
                                event.target.value,
                              )
                            }
                          />
                          <button
                            className="button primary"
                            type="button"
                            disabled={
                              busy || !mcpOauthCallbacks[server.name]?.trim()
                            }
                            onClick={() => onCompleteMcpOAuth(server.name)}
                          >
                            Complete
                          </button>
                        </>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              );
            })}
            {snapshot.globalConfiguration.mcpServers.length === 0 ? (
              <EmptySettings
                icon={<IconServer size={24} />}
                title="No global MCP servers"
              />
            ) : null}
          </div>
        </div>
        <AuthoritySummary selectedSpace={selectedSpace} draft={draft} />
      </section>
    );
  }
  if (tab === "search") {
    const selectedProfiles =
      snapshot.globalConfiguration.searchProviders.filter((entry) =>
        draft.selectedSearch.includes(entry.id),
      );
    return (
      <section className="managed-settings-layout">
        <div className="managed-settings-body">
          <div className="managed-section-heading">
            <div>
              <p className="eyebrow">Pinned global revisions</p>
              <h3>Search routing</h3>
            </div>
            <span>{draft.selectedSearch.length} enabled</span>
          </div>
          <div className="managed-list mcp-selection-list">
            {snapshot.globalConfiguration.searchProviders.map((entry) => {
              const search = currentValue(entry);
              const enabled = draft.selectedSearch.includes(entry.id);
              return (
                <label className="managed-list-row" key={entry.id}>
                  <span className="resource-icon">
                    <IconSearch size={18} />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {search.kind.replace("_", " ")} · {search.endpoint}
                    </small>
                  </div>
                  <span
                    className={`status-chip ${enabled ? "tone-success" : "tone-neutral"}`}
                  >
                    {enabled ? "Enabled" : "Available"}
                  </span>
                  <SwitchInput
                    checked={enabled}
                    aria-label={`Enable ${entry.label}`}
                    onChange={(event) => {
                      const selectedSearch = event.target.checked
                        ? [...draft.selectedSearch, entry.id]
                        : draft.selectedSearch.filter((id) => id !== entry.id);
                      const searchRoles = Object.fromEntries(
                        Object.entries(draft.searchRoles).filter(
                          ([, profile]) =>
                            event.target.checked || profile !== search.profile,
                        ),
                      );
                      setDraft({ ...draft, selectedSearch, searchRoles });
                    }}
                  />
                </label>
              );
            })}
          </div>
          <div className="authority-control-grid search-route-grid">
            {(["agent", "research"] as const).map((role) => (
              <label key={role}>
                <span>
                  {role === "agent" ? "Agent search" : "Research search"}
                </span>
                <DropdownSelect
                  value={draft.searchRoles[role] ?? ""}
                  onChange={(event) => {
                    const searchRoles = { ...draft.searchRoles };
                    if (event.target.value)
                      searchRoles[role] = event.target.value;
                    else delete searchRoles[role];
                    setDraft({ ...draft, searchRoles });
                  }}
                >
                  <option value="">Disabled</option>
                  {selectedProfiles.map((entry) => (
                    <option key={entry.id} value={currentValue(entry).profile}>
                      {entry.label}
                    </option>
                  ))}
                </DropdownSelect>
              </label>
            ))}
          </div>
          <div className="managed-diagnostic-actions">
            {(["agent", "research"] as const).map((role) => {
              const diagnostic = runtimeDiagnostics[`search:${role}`];
              return (
                <div key={role}>
                  <button
                    className="button secondary"
                    type="button"
                    disabled={
                      busy ||
                      selectedSpace.status !== "active" ||
                      !draft.searchRoles[role]
                    }
                    onClick={() => onTestSearchRole(role)}
                  >
                    <IconActivityHeartbeat size={15} />
                    Test {role} search
                  </button>
                  {diagnostic ? <DiagnosticResult value={diagnostic} /> : null}
                </div>
              );
            })}
          </div>
        </div>
        <AuthoritySummary selectedSpace={selectedSpace} draft={draft} />
      </section>
    );
  }
  if (tab === "telemetry") {
    return (
      <section className="managed-settings-layout">
        <div className="managed-settings-body">
          <div className="managed-section-heading">
            <div>
              <p className="eyebrow">Pinned global revision</p>
              <h3>Telemetry policy</h3>
            </div>
            <span>{draft.selectedTelemetry ? "Enabled" : "Disabled"}</span>
          </div>
          <div className="authority-control-grid">
            <label>
              <span>Telemetry profile</span>
              <DropdownSelect
                value={draft.selectedTelemetry ?? ""}
                onChange={(event) =>
                  setDraft({
                    ...draft,
                    selectedTelemetry: event.target.value || null,
                  })
                }
              >
                <option value="">Disabled</option>
                {snapshot.globalConfiguration.telemetryProfiles.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.label} · r{entry.currentRevision}
                  </option>
                ))}
              </DropdownSelect>
            </label>
          </div>
          {draft.selectedTelemetry ? (
            <>
              <div className="authority-note">
                <IconActivityHeartbeat size={18} />
                <div>
                  <strong>Immutable while active</strong>
                  <p>
                    Runs keep this telemetry revision until the Workspace
                    applies a newer global revision.
                  </p>
                </div>
              </div>
              <div className="managed-diagnostic-actions">
                <div>
                  <button
                    className="button secondary"
                    type="button"
                    disabled={busy || selectedSpace.status !== "active"}
                    onClick={() => onTestTelemetry(draft.selectedTelemetry!)}
                  >
                    <IconActivityHeartbeat size={15} />
                    Test OTLP exporters
                  </button>
                  {runtimeDiagnostics[
                    `telemetry:${draft.selectedTelemetry}`
                  ] ? (
                    <DiagnosticResult
                      value={
                        runtimeDiagnostics[
                          `telemetry:${draft.selectedTelemetry}`
                        ]!
                      }
                    />
                  ) : null}
                </div>
              </div>
            </>
          ) : null}
        </div>
        <AuthoritySummary selectedSpace={selectedSpace} draft={draft} />
      </section>
    );
  }
  if (tab === "effective") {
    return (
      <section className="managed-settings-body effective-yaml-section">
        <div className="managed-section-heading">
          <div>
            <p className="eyebrow">Read-only · sanitized</p>
            <h3>Effective YAML</h3>
          </div>
          <span className="status-chip tone-neutral">
            r{selectedSpace.configuration.acceptedGlobalRevision}
          </span>
        </div>
        <pre
          className="effective-configuration-code"
          aria-label="Effective Workspace configuration"
        >
          <code>{selectedSpace.effectiveYaml}</code>
        </pre>
        <div className="locked-invariant-list">
          {snapshot.lockedInvariants.map((invariant) => (
            <div key={invariant.id}>
              <IconLock size={16} />
              <span>
                <strong>{invariant.title}</strong>
                <small>
                  {invariant.id} · {invariant.owner}
                </small>
              </span>
            </div>
          ))}
        </div>
      </section>
    );
  }
  if (tab === "access" || tab === "runtime") {
    return (
      <section className="managed-settings-layout">
        <div className="managed-settings-body">
          <div className="managed-section-heading">
            <div>
              <p className="eyebrow">Workspace override</p>
              <h3>
                {tab === "access" ? "Access and authority" : "Runtime defaults"}
              </h3>
            </div>
          </div>
          <AuthorityControls
            access={draft.accessProfile}
            boundary={draft.executionBoundary}
            terminal={draft.terminalEnabled}
            onAccess={(value) => setDraft({ ...draft, accessProfile: value })}
            onBoundary={(value) =>
              setDraft({ ...draft, executionBoundary: value })
            }
            onTerminal={(value) =>
              setDraft({ ...draft, terminalEnabled: value })
            }
          />
          {tab === "runtime" ? (
            <FieldGrid
              descriptors={descriptors.filter(
                (descriptor) =>
                  !descriptor.advanced &&
                  !descriptor.id.startsWith("research."),
              )}
              values={draft.fields}
              effective={effective}
              scope="space"
              focusedFieldId={focusedFieldId}
              onChange={(id, value) =>
                setDraft({ ...draft, fields: { ...draft.fields, [id]: value } })
              }
              onInherit={(id) => removeDraftField(draft, setDraft, id)}
            />
          ) : null}
        </div>
        <AuthoritySummary selectedSpace={selectedSpace} draft={draft} />
      </section>
    );
  }
  if (tab === "providers") {
    const selectedModels = snapshot.globalConfiguration.models.filter((entry) =>
      draft.selectedModels.includes(entry.id),
    );
    return (
      <section className="managed-settings-layout">
        <div className="managed-settings-body">
          <div className="managed-section-heading">
            <div>
              <p className="eyebrow">Pinned global revisions</p>
              <h3>Providers and models</h3>
            </div>
            <span>
              {draft.selectedProviders.length} providers ·{" "}
              {draft.selectedModels.length} models
            </span>
          </div>
          <div className="managed-list mcp-selection-list">
            {snapshot.globalConfiguration.providers.map((entry) => {
              const provider = currentValue(entry);
              const enabled = draft.selectedProviders.includes(entry.id);
              return (
                <label className="managed-list-row" key={entry.id}>
                  <span className="resource-icon">
                    <IconCloud size={18} />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>{provider.baseUrl}</small>
                  </div>
                  <span
                    className={`status-chip ${enabled ? "tone-success" : "tone-neutral"}`}
                  >
                    {enabled ? "Selected" : "Global"}
                  </span>
                  <SwitchInput
                    checked={enabled}
                    aria-label={`Select ${entry.label}`}
                    onChange={(event) => {
                      const selectedProviders = event.target.checked
                        ? [...draft.selectedProviders, entry.id]
                        : draft.selectedProviders.filter(
                            (id) => id !== entry.id,
                          );
                      const removedModels = new Set(
                        snapshot.globalConfiguration.models
                          .filter(
                            (model) =>
                              currentValue(model).providerProfile ===
                              provider.profile,
                          )
                          .map((model) => model.id),
                      );
                      const selectedModelProfiles = new Set(
                        snapshot.globalConfiguration.models
                          .filter(
                            (model) =>
                              event.target.checked ||
                              !removedModels.has(model.id),
                          )
                          .map((model) => currentValue(model).profile),
                      );
                      setDraft({
                        ...draft,
                        selectedProviders,
                        selectedModels: event.target.checked
                          ? draft.selectedModels
                          : draft.selectedModels.filter(
                              (id) => !removedModels.has(id),
                            ),
                        modelRoles: Object.fromEntries(
                          Object.entries(draft.modelRoles).filter(
                            ([, profile]) => selectedModelProfiles.has(profile),
                          ),
                        ),
                      });
                    }}
                  />
                </label>
              );
            })}
            {snapshot.globalConfiguration.models.map((entry) => {
              const model = currentValue(entry);
              const providerSelected =
                snapshot.globalConfiguration.providers.some(
                  (provider) =>
                    draft.selectedProviders.includes(provider.id) &&
                    currentValue(provider).profile === model.providerProfile,
                );
              const enabled = draft.selectedModels.includes(entry.id);
              return (
                <label className="managed-list-row" key={entry.id}>
                  <span className="resource-icon">
                    <IconCpu size={18} />
                  </span>
                  <div>
                    <strong>{entry.label}</strong>
                    <small>
                      {model.model} · {model.providerProfile}
                    </small>
                  </div>
                  <span
                    className={`status-chip ${enabled ? "tone-success" : "tone-neutral"}`}
                  >
                    {enabled ? "Selected" : `r${entry.currentRevision}`}
                  </span>
                  <SwitchInput
                    disabled={!providerSelected}
                    checked={enabled}
                    aria-label={`Select ${entry.label}`}
                    onChange={(event) => {
                      const selectedModels = event.target.checked
                        ? [...draft.selectedModels, entry.id]
                        : draft.selectedModels.filter((id) => id !== entry.id);
                      const modelRoles = Object.fromEntries(
                        Object.entries(draft.modelRoles).filter(
                          ([, profile]) =>
                            event.target.checked || profile !== model.profile,
                        ),
                      );
                      setDraft({ ...draft, selectedModels, modelRoles });
                    }}
                  />
                </label>
              );
            })}
          </div>
          <div className="managed-diagnostic-actions">
            {snapshot.globalConfiguration.providers
              .filter((entry) => draft.selectedProviders.includes(entry.id))
              .map((entry) => {
                const profile = currentValue(entry).profile;
                const diagnostic = runtimeDiagnostics[`provider:${profile}`];
                return (
                  <div key={`provider:${entry.id}`}>
                    <button
                      className="button secondary"
                      type="button"
                      disabled={busy || selectedSpace.status !== "active"}
                      onClick={() => onTestRuntimeProfile("provider", profile)}
                    >
                      <IconActivityHeartbeat size={15} />
                      Test {entry.label}
                    </button>
                    {diagnostic ? (
                      <DiagnosticResult value={diagnostic} />
                    ) : null}
                  </div>
                );
              })}
            {selectedModels.map((entry) => {
              const profile = currentValue(entry).profile;
              const diagnostic = runtimeDiagnostics[`model:${profile}`];
              return (
                <div key={`model:${entry.id}`}>
                  <button
                    className="button secondary"
                    type="button"
                    disabled={busy || selectedSpace.status !== "active"}
                    onClick={() => onTestRuntimeProfile("model", profile)}
                  >
                    <IconCpu size={15} />
                    Test {entry.label}
                  </button>
                  {diagnostic ? <DiagnosticResult value={diagnostic} /> : null}
                </div>
              );
            })}
          </div>
          <div className="authority-control-grid search-route-grid">
            {(
              [
                "primary",
                "risk_evaluator",
                "context_summarizer",
                "subagent_default",
              ] as const
            ).map((role) => (
              <label key={role}>
                <span>{role.replaceAll("_", " ")}</span>
                <DropdownSelect
                  required={role === "primary"}
                  value={draft.modelRoles[role] ?? ""}
                  onChange={(event) => {
                    const modelRoles = { ...draft.modelRoles };
                    if (event.target.value)
                      modelRoles[role] = event.target.value;
                    else delete modelRoles[role];
                    setDraft({ ...draft, modelRoles });
                  }}
                >
                  <option value="">Inherit primary</option>
                  {selectedModels.map((entry) => (
                    <option key={entry.id} value={currentValue(entry).profile}>
                      {entry.label}
                    </option>
                  ))}
                </DropdownSelect>
              </label>
            ))}
          </div>
        </div>
        <AuthoritySummary selectedSpace={selectedSpace} draft={draft} />
      </section>
    );
  }
  const filtered = descriptors.filter((descriptor) => {
    if (tab === "sandbox") return descriptor.id.startsWith("sandbox.");
    if (tab === "research") return descriptor.id.startsWith("research.");
    return descriptor.advanced;
  });
  return (
    <section className="managed-settings-body">
      <div className="managed-section-heading">
        <div>
          <p className="eyebrow">Sparse Workspace overrides</p>
          <h3>{tab[0]!.toUpperCase() + tab.slice(1)}</h3>
        </div>
      </div>
      {tab === "advanced" ? (
        [
          ...new Set([
            ...filtered.map((descriptor) => descriptor.section),
            "Packs",
          ]),
        ]
          .sort()
          .map((section) => {
            const sectionDescriptors = filtered.filter(
              (descriptor) => descriptor.section === section,
            );
            return (
              <details
                className="managed-advanced-disclosure"
                key={section}
                open={
                  expandedAdvancedSections.has(section) ||
                  advancedSectionContainsField(
                    sectionDescriptors,
                    focusedFieldId,
                  )
                }
                onToggle={(event) =>
                  onAdvancedSectionToggle(section, event.currentTarget.open)
                }
              >
                <summary>
                  <span>{section}</span>
                  <small>
                    {section === "Packs"
                      ? "Live catalog"
                      : `${sectionDescriptors.length} settings`}
                  </small>
                </summary>
                {sectionDescriptors.length > 0 ? (
                  <FieldGrid
                    descriptors={sectionDescriptors}
                    values={draft.fields}
                    effective={effective}
                    scope="space"
                    focusedFieldId={focusedFieldId}
                    onChange={(id, value) =>
                      setDraft({
                        ...draft,
                        fields: { ...draft.fields, [id]: value },
                      })
                    }
                    onInherit={(id) => removeDraftField(draft, setDraft, id)}
                  />
                ) : null}
                <ExtensionCatalog
                  section={section}
                  inventory={extensionInventory}
                  busy={extensionInventoryBusy}
                  runtimeActive={selectedSpace.status === "active"}
                  onRefresh={onRefreshExtensionInventory}
                />
              </details>
            );
          })
      ) : (
        <FieldGrid
          descriptors={filtered}
          values={draft.fields}
          effective={effective}
          scope="space"
          focusedFieldId={focusedFieldId}
          onChange={(id, value) =>
            setDraft({ ...draft, fields: { ...draft.fields, [id]: value } })
          }
          onInherit={(id) => removeDraftField(draft, setDraft, id)}
        />
      )}
      {filtered.length === 0 ? (
        <EmptySettings
          icon={<IconAdjustments size={24} />}
          title={`No ${tab} overrides`}
        />
      ) : null}
    </section>
  );
}

export function ExtensionCatalog({
  section,
  inventory,
  busy,
  runtimeActive,
  onRefresh,
}: {
  section: string;
  inventory: ManagedExtensionInventory | null;
  busy: boolean;
  runtimeActive: boolean;
  onRefresh: () => void;
}) {
  if (!matchesExtensionSection(section)) return null;
  const count =
    section === "Skills"
      ? (inventory?.skills.length ?? 0)
      : section === "Packs"
        ? (inventory?.packs.length ?? 0)
        : (inventory?.workflows.length ?? 0);
  return (
    <div className="managed-extension-catalog">
      <div className="managed-catalog-toolbar">
        <div>
          <strong>Live runtime catalog</strong>
          <small>{count} resources</small>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label={`Refresh ${section.toLowerCase()} catalog`}
          title={`Refresh ${section.toLowerCase()} catalog`}
          disabled={!runtimeActive || busy}
          onClick={onRefresh}
        >
          <IconRefresh size={16} />
        </button>
      </div>
      {!runtimeActive ? (
        <EmptySettings
          icon={<IconActivityHeartbeat size={24} />}
          title="Runtime not active"
        />
      ) : null}
      {runtimeActive && busy && !inventory ? (
        <EmptySettings
          icon={<IconRefresh size={24} />}
          title="Loading catalog"
        />
      ) : null}
      {runtimeActive && !busy && inventory && count === 0 ? (
        <EmptySettings
          icon={<IconDatabase size={24} />}
          title={`No ${section.toLowerCase()} registered`}
        />
      ) : null}
      {runtimeActive && inventory && count > 0 ? (
        <div className="managed-list">
          {section === "Skills"
            ? inventory.skills.map((skill) => (
                <div
                  className="managed-list-row"
                  key={`${skill.name}:${skill.version}:${skill.source}`}
                >
                  <span className="resource-icon">
                    <IconFileImport size={17} />
                  </span>
                  <div>
                    <strong>
                      {skill.name} · {skill.version}
                    </strong>
                    <small>{skill.description}</small>
                  </div>
                  <span className="status-chip tone-neutral">
                    {skill.source}
                  </span>
                  <span
                    className={`status-chip ${skill.offlineCompatible ? "tone-success" : "tone-warning"}`}
                  >
                    {skill.offlineCompatible ? "Offline" : "Network"}
                  </span>
                </div>
              ))
            : null}
          {section === "Packs"
            ? inventory.packs.map((pack) => (
                <div
                  className="managed-list-row"
                  key={`${pack.name}:${pack.version}:${pack.manifestSha256}`}
                >
                  <span className="resource-icon">
                    <IconDatabase size={17} />
                  </span>
                  <div>
                    <strong>
                      {pack.name} · {pack.version}
                    </strong>
                    <small>{pack.publisher}</small>
                  </div>
                  <span
                    className={`status-chip ${pack.status === "enabled" ? "tone-success" : "tone-neutral"}`}
                  >
                    {pack.status}
                  </span>
                  <span
                    className={`status-chip ${pack.trusted ? "tone-success" : "tone-warning"}`}
                  >
                    {pack.trusted ? "Trusted" : "Untrusted"}
                  </span>
                </div>
              ))
            : null}
          {section === "Workflows"
            ? inventory.workflows.map((workflow) => (
                <div
                  className="managed-list-row"
                  key={`${workflow.name}:${workflow.version}:${workflow.revisionHash}`}
                >
                  <span className="resource-icon">
                    <IconActivityHeartbeat size={17} />
                  </span>
                  <div>
                    <strong>
                      {workflow.name} · {workflow.version}
                    </strong>
                    <small>{workflow.updatedAt}</small>
                  </div>
                  <span className="status-chip tone-neutral">
                    {workflow.status}
                  </span>
                </div>
              ))
            : null}
        </div>
      ) : null}
    </div>
  );
}

function matchesExtensionSection(section: string) {
  return section === "Skills" || section === "Packs" || section === "Workflows";
}

function AuthorityControls({
  access,
  boundary,
  terminal,
  onAccess,
  onBoundary,
  onTerminal,
}: {
  access: AccessProfile | null;
  boundary: ExecutionBoundary | null;
  terminal: boolean | null;
  onAccess: (value: AccessProfile | null) => void;
  onBoundary: (value: ExecutionBoundary | null) => void;
  onTerminal: (value: boolean | null) => void;
}) {
  return (
    <div className="authority-control-grid">
      <label>
        <span>Access profile</span>
        <DropdownSelect
          aria-label="Access profile"
          value={access ?? "inherit"}
          onChange={(event) =>
            onAccess(
              event.target.value === "inherit"
                ? null
                : (event.target.value as AccessProfile),
            )
          }
        >
          <option value="inherit">Inherit</option>
          <option value="minimal">Minimal</option>
          <option value="pinned">Pinned</option>
          <option value="development">Development</option>
          <option value="allow_all">Allow all</option>
        </DropdownSelect>
        <small>Choose which tools and actions agents may use by default.</small>
      </label>
      <label>
        <span>Execution boundary</span>
        <DropdownSelect
          aria-label="Execution boundary"
          value={boundary ?? "inherit"}
          onChange={(event) =>
            onBoundary(
              event.target.value === "inherit"
                ? null
                : (event.target.value as ExecutionBoundary),
            )
          }
        >
          <option value="inherit">Inherit</option>
          <option value="offline_isolated">Offline isolated</option>
          <option value="workspace_isolated">Workspace isolated</option>
          <option value="full_access">Full access</option>
        </DropdownSelect>
        <small>
          Choose how strongly tool and process execution is isolated.
        </small>
      </label>
      <label>
        <span>Local terminal</span>
        <DropdownSelect
          aria-label="Local terminal"
          value={terminal === null ? "inherit" : String(terminal)}
          onChange={(event) =>
            onTerminal(
              event.target.value === "inherit"
                ? null
                : event.target.value === "true",
            )
          }
        >
          <option value="inherit">Inherit</option>
          <option value="false">Disabled</option>
          <option value="true">Enabled</option>
        </DropdownSelect>
        <small>Choose whether workspaces may open a local terminal.</small>
      </label>
    </div>
  );
}

export function FieldGrid({
  descriptors,
  values,
  effective,
  scope,
  focusedFieldId = null,
  onChange,
  onInherit,
}: {
  descriptors: ManagedFieldDescriptor[];
  values: Record<string, unknown>;
  effective: Map<string, { value: unknown; source: string }>;
  scope: "global" | "space";
  focusedFieldId?: string | null;
  onChange: (id: string, value: unknown) => void;
  onInherit: (id: string) => void;
}) {
  return (
    <div className="managed-field-grid">
      {descriptors.map((descriptor) => {
        const overridden = Object.hasOwn(values, descriptor.id);
        const resolved = overridden
          ? values[descriptor.id]
          : (effective.get(descriptor.id)?.value ?? descriptor.defaultValue);
        const source = overridden
          ? scope
          : (effective.get(descriptor.id)?.source ?? "built_in");
        return (
          <div
            className={`managed-field-row${focusedFieldId === descriptor.id ? " is-search-target" : ""}`}
            id={managedFieldElementId(descriptor.id)}
            key={descriptor.id}
            tabIndex={focusedFieldId === descriptor.id ? -1 : undefined}
          >
            <div>
              <strong>{descriptor.title}</strong>
              <span>{descriptor.description}</span>
              <small>
                {descriptor.section} · {descriptor.id}
              </small>
            </div>
            <SourceBadge source={source} />
            <FieldControl
              descriptor={descriptor}
              value={resolved}
              onChange={(value) => onChange(descriptor.id, value)}
            />
            <button
              className="button secondary inherit-button"
              type="button"
              disabled={!overridden}
              onClick={() => onInherit(descriptor.id)}
            >
              Inherit
            </button>
          </div>
        );
      })}
    </div>
  );
}

function FieldControl({
  descriptor,
  value,
  onChange,
}: {
  descriptor: ManagedFieldDescriptor;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  if (descriptor.control === "toggle") {
    return (
      <input
        type="checkbox"
        checked={Boolean(value)}
        aria-label={descriptor.title}
        onChange={(event) => onChange(event.target.checked)}
      />
    );
  }
  if (descriptor.control === "number") {
    return (
      <input
        type="number"
        value={Number(value)}
        min={descriptor.minimum ?? undefined}
        max={descriptor.maximum ?? undefined}
        aria-label={descriptor.title}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    );
  }
  if (descriptor.control === "string_list") {
    return (
      <textarea
        rows={3}
        value={Array.isArray(value) ? value.join("\n") : ""}
        aria-label={descriptor.title}
        placeholder="One exact value per line"
        onChange={(event) =>
          onChange(
            event.target.value
              .split("\n")
              .map((item) => item.trim())
              .filter(Boolean),
          )
        }
      />
    );
  }
  if (descriptor.control === "select") {
    return (
      <DropdownSelect
        value={typeof value === "string" ? value : ""}
        aria-label={descriptor.title}
        onChange={(event) => onChange(event.target.value)}
      >
        {descriptor.options.map((option) => (
          <option key={option} value={option}>
            {option.replaceAll("_", " ")}
          </option>
        ))}
      </DropdownSelect>
    );
  }
  if (descriptor.control === "json") {
    return (
      <JsonFieldEditor
        label={descriptor.title}
        value={value}
        onChange={onChange}
      />
    );
  }
  return (
    <input
      type="text"
      value={typeof value === "string" ? value : ""}
      aria-label={descriptor.title}
      onChange={(event) =>
        onChange(
          event.target.value === "" && descriptor.defaultValue === null
            ? null
            : event.target.value,
        )
      }
    />
  );
}

function JsonFieldEditor({
  label,
  value,
  onChange,
}: {
  label: string;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const serialized = JSON.stringify(value, null, 2);
  const [text, setText] = useState(serialized);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => {
    setText(serialized);
    setInvalid(false);
  }, [serialized]);
  function commit() {
    try {
      onChange(JSON.parse(text));
      setInvalid(false);
    } catch {
      setInvalid(true);
    }
  }
  return (
    <span className={`managed-json-editor ${invalid ? "is-invalid" : ""}`}>
      <textarea
        rows={5}
        value={text}
        aria-label={label}
        aria-invalid={invalid}
        onChange={(event) => setText(event.target.value)}
        onBlur={commit}
      />
      {invalid ? <small role="alert">Enter valid JSON.</small> : null}
    </span>
  );
}

function SourceBadge({ source }: { source: string }) {
  const label = source === "space" ? "workspace" : source.replace("_", " ");
  return <span className={`source-badge source-${source}`}>{label}</span>;
}

function DiagnosticResult({ value }: { value: ManagedRuntimeDiagnostic }) {
  return (
    <div
      className={`managed-diagnostic-result ${value.ready ? "is-ready" : "is-failed"}`}
      role="status"
    >
      <strong>{value.ready ? "Passed" : "Failed"}</strong>
      <span>
        {value.checks.map((check) => check.detail).join(" ") ||
          "Diagnostic completed."}
      </span>
    </div>
  );
}

function RuntimeStatus({
  space,
}: {
  space: ManagedSpaceConfigurationSnapshot;
}) {
  const tone =
    space.status === "active"
      ? "tone-success"
      : space.status === "runtime_failed" ||
          space.status === "validation_failed"
        ? "tone-danger"
        : "tone-warning";
  return (
    <span className={`status-chip ${tone}`} title={space.statusMessage}>
      {space.status.replaceAll("_", " ")}
    </span>
  );
}

function AuthoritySummary({
  selectedSpace,
  draft,
}: {
  selectedSpace: ManagedSpaceConfigurationSnapshot;
  draft: SpaceDraft;
}) {
  const boundary =
    draft.executionBoundary ??
    String(
      selectedSpace.effectiveValues.find(
        (value) => value.fieldId === "sandbox.executionBoundary",
      )?.value ?? "workspace_isolated",
    );
  return (
    <aside className="authority-summary" aria-label="Authority summary">
      <h3>Authority summary</h3>
      <AuthorityItem
        icon={<IconCpu size={18} />}
        label="Process"
        value={draft.accessProfile?.replace("_", " ") ?? "Inherited"}
      />
      <AuthorityItem
        icon={<IconNetwork size={18} />}
        label="Network"
        value={
          boundary === "offline_isolated" ? "Offline" : "Managed destinations"
        }
      />
      <AuthorityItem
        icon={<IconTerminal2 size={18} />}
        label="Environment"
        value={draft.terminalEnabled ? "Local terminal" : "Managed variables"}
      />
      <AuthorityItem
        icon={<IconFolder size={18} />}
        label="Filesystem"
        value={boundary === "full_access" ? "Host access" : "Workspace root"}
      />
      {boundary === "full_access" ? (
        <p className="authority-warning">
          <IconAlertTriangle size={17} />
          Full access requires native confirmation.
        </p>
      ) : null}
    </aside>
  );
}

function AuthorityItem({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="authority-item">
      <span>{icon}</span>
      <div>
        <strong>{label}</strong>
        <small>{value}</small>
      </div>
    </div>
  );
}

export function RepositoryImportDialog({
  proposal,
  stage,
  credentials,
  mappings,
  conflicts,
  busy,
  onStageChange,
  onMappingsChange,
  onConflictsChange,
  onApply,
  onClose,
}: {
  proposal: RepositoryConfigurationProposal;
  stage: number;
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  mappings: Record<string, string>;
  conflicts: Record<string, ImportConflictDecision>;
  busy: boolean;
  onStageChange: (stage: number) => void;
  onMappingsChange: (mappings: Record<string, string>) => void;
  onConflictsChange: (
    conflicts: Record<string, ImportConflictDecision>,
  ) => void;
  onApply: () => void;
  onClose: () => void;
}) {
  const stages = [
    "Review",
    "Map credentials",
    "Conflicts",
    "Authority",
    "Apply",
  ];
  const credentialIds = new Set(credentials.map((credential) => credential.id));
  const mappingsValid = proposal.credentialSlots.every((slot) => {
    const credentialId = mappings[slot.slotId];
    return Boolean(credentialId && credentialIds.has(credentialId));
  });
  const conflictsValid = proposal.resources
    .filter((resource) => resource.conflict)
    .every((resource) => {
      const decision = conflicts[`${resource.kind}:${resource.sourceId}`];
      if (!decision) return false;
      if (decision.action !== "rename") return true;
      const renamed = decision.renamedSourceId?.trim() ?? "";
      return (
        renamed.length > 0 &&
        renamed.length <= 64 &&
        renamed !== resource.sourceId &&
        /^[A-Za-z0-9._-]+$/.test(renamed)
      );
    });
  const stageValid =
    (stage !== 1 || mappingsValid) &&
    (stage !== 2 || conflictsValid) &&
    (stage !== stages.length - 1 || (mappingsValid && conflictsValid));
  return (
    <div className="settings-dialog-backdrop" role="presentation">
      <section
        className="settings-dialog repository-import-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="repository-import-heading"
      >
        <header>
          <div>
            <p className="eyebrow">{proposal.relativePath}</p>
            <h3 id="repository-import-heading">
              Import workspace configuration
            </h3>
          </div>
          <button
            className="icon-button"
            type="button"
            aria-label="Close repository import"
            onClick={onClose}
          >
            <IconX size={17} />
          </button>
        </header>
        <ol className="import-stage-list">
          {stages.map((label, index) => (
            <li className={index === stage ? "is-active" : ""} key={label}>
              <span>{index + 1}</span>
              {label}
            </li>
          ))}
        </ol>
        <div className="repository-import-content">
          {stage === 0 ? (
            <>
              {proposal.changedSinceImport ? (
                <div className="authority-note">
                  <IconAlertTriangle size={18} />
                  <div>
                    <strong>Repository configuration changed</strong>
                    <p>
                      Imported {proposal.previousSha256?.slice(0, 12)} ·
                      proposed {proposal.sha256.slice(0, 12)}. Review the
                      resources and overrides below before re-importing.
                    </p>
                  </div>
                </div>
              ) : null}
              <div className="managed-metric-strip">
                <Metric
                  icon={<IconDatabase size={19} />}
                  value={proposal.resources.length}
                  label="resources"
                />
                <Metric
                  icon={<IconAdjustments size={19} />}
                  value={proposal.fieldOverrides.length}
                  label="Workspace overrides"
                />
                <Metric
                  icon={<IconLock size={19} />}
                  value={proposal.lockedFields.length}
                  label="Desktop-owned"
                />
              </div>
              <div className="managed-list">
                {proposal.resources.map((resource) => (
                  <div
                    className="managed-list-row"
                    key={`${resource.kind}:${resource.sourceId}`}
                  >
                    <span className="resource-icon">
                      <IconFileImport size={17} />
                    </span>
                    <div>
                      <strong>{resource.label}</strong>
                      <small>{resource.detail}</small>
                    </div>
                    <span className="status-chip tone-neutral">
                      {resource.kind}
                    </span>
                  </div>
                ))}
              </div>
            </>
          ) : null}
          {stage === 1 ? (
            <div className="authority-control-grid">
              {proposal.credentialSlots.map((slot) => (
                <label key={slot.slotId}>
                  <span>{slot.label}</span>
                  <DropdownSelect
                    value={mappings[slot.slotId] ?? ""}
                    onChange={(event) =>
                      onMappingsChange({
                        ...mappings,
                        [slot.slotId]: event.target.value,
                      })
                    }
                  >
                    <option value="">Choose native credential</option>
                    {credentials.map((credential) => (
                      <option key={credential.id} value={credential.id}>
                        {credential.label}
                      </option>
                    ))}
                  </DropdownSelect>
                  <small>{slot.consumers.length} consumers</small>
                </label>
              ))}
              {proposal.credentialSlots.length === 0 ? (
                <EmptySettings
                  icon={<IconKey size={24} />}
                  title="No credential mappings required"
                />
              ) : null}
            </div>
          ) : null}
          {stage === 2 ? (
            <div className="managed-list import-conflict-list">
              {proposal.resources
                .filter((resource) => resource.conflict)
                .map((resource) => {
                  const key = `${resource.kind}:${resource.sourceId}`;
                  const decision = conflicts[key] ?? {
                    action: "rename" as const,
                    renamedSourceId: `${resource.sourceId}-imported`,
                  };
                  return (
                    <div className="managed-list-row" key={key}>
                      <span className="resource-icon">
                        <IconAlertTriangle size={17} />
                      </span>
                      <div>
                        <strong>{resource.label}</strong>
                        <small>
                          A global {resource.kind} already uses this profile
                          name.
                        </small>
                      </div>
                      <label>
                        <span className="sr-only">
                          Resolution for {resource.label}
                        </span>
                        <DropdownSelect
                          aria-label={`Resolution for ${resource.label}`}
                          value={decision.action}
                          onChange={(event) => {
                            const action = event.target
                              .value as ImportConflictDecision["action"];
                            onConflictsChange({
                              ...conflicts,
                              [key]: {
                                action,
                                renamedSourceId:
                                  action === "rename"
                                    ? (decision.renamedSourceId ??
                                      `${resource.sourceId}-imported`)
                                    : null,
                              },
                            });
                          }}
                        >
                          <option value="rename">Rename imported</option>
                          <option value="replace">
                            Replace global revision
                          </option>
                          <option value="skip">Skip definition</option>
                        </DropdownSelect>
                      </label>
                      {decision.action === "rename" ? (
                        <input
                          type="text"
                          aria-label={`New profile name for ${resource.label}`}
                          value={decision.renamedSourceId ?? ""}
                          onChange={(event) =>
                            onConflictsChange({
                              ...conflicts,
                              [key]: {
                                ...decision,
                                renamedSourceId: event.target.value,
                              },
                            })
                          }
                        />
                      ) : null}
                    </div>
                  );
                })}
              {proposal.resources.every((resource) => !resource.conflict) ? (
                <div className="authority-note">
                  <IconCheck size={18} />
                  <div>
                    <strong>No profile-name conflicts</strong>
                    <p>Exact definitions will be reused automatically.</p>
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}
          {stage === 3 ? (
            <div className="managed-list">
              {proposal.lockedFields.map((field) => (
                <div className="managed-list-row" key={field}>
                  <span className="resource-icon">
                    <IconLock size={17} />
                  </span>
                  <div>
                    <strong>{field}</strong>
                    <small>Remains Desktop-managed</small>
                  </div>
                </div>
              ))}
              {proposal.warnings.map((warning) => (
                <p className="authority-warning" key={warning}>
                  <IconAlertTriangle size={17} /> {warning}
                </p>
              ))}
            </div>
          ) : null}
          {stage === 4 ? (
            <div className="authority-note">
              <IconCheck size={18} />
              <div>
                <strong>Ready for conflict decisions</strong>
                <p>The repository file will remain unchanged.</p>
              </div>
            </div>
          ) : null}
        </div>
        <footer>
          <span className="import-hash">
            SHA-256 {proposal.sha256.slice(0, 12)}
          </span>
          <button
            className="button secondary"
            type="button"
            disabled={stage === 0}
            onClick={() => onStageChange(stage - 1)}
          >
            Back
          </button>
          <button
            className="button primary"
            type="button"
            disabled={busy || !stageValid}
            onClick={() =>
              stage === stages.length - 1 ? onApply() : onStageChange(stage + 1)
            }
          >
            {stage === stages.length - 1
              ? busy
                ? "Applying..."
                : "Apply import"
              : "Next"}
          </button>
        </footer>
      </section>
    </div>
  );
}

export function SettingsActionBar({
  dirty,
  busy,
  failure,
  label,
  onDiscard,
  onApply,
}: {
  dirty: boolean;
  busy: boolean;
  failure: string;
  label: string;
  onDiscard: () => void;
  onApply: () => void;
}) {
  return (
    <div className="managed-settings-actions">
      <span
        className={failure ? "is-error" : undefined}
        role={failure ? "alert" : undefined}
      >
        {failure || (dirty ? "Unsaved changes" : "No local changes")}
      </span>
      <button
        className="button secondary"
        type="button"
        disabled={!dirty || busy}
        onClick={onDiscard}
      >
        Discard
      </button>
      <button
        className="button primary"
        type="button"
        disabled={!dirty || busy}
        onClick={onApply}
      >
        <IconCheck size={16} />
        {busy ? "Applying…" : label}
      </button>
    </div>
  );
}

function SearchEditor({
  draft,
  credentials,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  draft: SearchEditorDraft;
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  busy: boolean;
  onChange: (draft: SearchEditorDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const title = draft.resourceId
    ? `Edit ${draft.label || "search service"}`
    : "Add search service";
  return (
    <form
      className="mcp-editor catalog-editor search-editor"
      aria-labelledby="search-editor-heading"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="search-editor-heading">
        <span className="resource-icon">
          <IconSearch size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id="search-editor-heading">{title}</h4>
          <p>Connect a search API, then choose which workspaces can use it.</p>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close search editor"
          onClick={onCancel}
        >
          <IconX size={17} />
        </button>
      </div>
      <section
        className="search-editor-section"
        aria-labelledby="search-editor-identity-heading"
      >
        <div className="search-editor-section-heading">
          <div>
            <h5 id="search-editor-identity-heading">Identity</h5>
            <p>
              Choose a name shown in settings and an ID used in configuration.
            </p>
          </div>
        </div>
        <div className="search-editor-grid search-editor-identity-grid">
          <label>
            <span>Display label</span>
            <input
              value={draft.label}
              placeholder="For example, Engineering search"
              aria-describedby="search-label-help"
              required
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small id="search-label-help">Shown to people in settings.</small>
          </label>
          <label>
            <span>Profile ID</span>
            <input
              value={draft.profile}
              placeholder="engineering-search"
              aria-describedby="search-profile-help"
              required
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
            <small id="search-profile-help">
              Used to identify this search service in workspace configuration.
            </small>
          </label>
          <label>
            <span>Adapter</span>
            <DropdownSelect
              value={draft.kind}
              aria-describedby="search-adapter-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  kind: event.target.value as ManagedSearchProvider["kind"],
                })
              }
            >
              <option value="searxng">SearXNG</option>
              <option value="serp_api">SerpAPI</option>
            </DropdownSelect>
            <small id="search-adapter-help">
              Choose the search API you are connecting to.
            </small>
          </label>
        </div>
      </section>
      <section
        className="search-editor-section"
        aria-labelledby="search-editor-connection-heading"
      >
        <div className="search-editor-section-heading">
          <div>
            <h5 id="search-editor-connection-heading">Connection</h5>
            <p>
              Enter the API endpoint and choose a saved credential when one is
              required.
            </p>
          </div>
          <span className="status-chip">
            {draft.kind === "searxng"
              ? "Credential optional"
              : "Credential required"}
          </span>
        </div>
        <div className="search-editor-grid search-editor-connection-grid">
          <label className="search-editor-endpoint">
            <span>Endpoint URL</span>
            <input
              type="url"
              value={draft.endpoint}
              placeholder={
                draft.kind === "searxng"
                  ? "https://search.example.com/search"
                  : "https://serpapi.com/search"
              }
              aria-describedby="search-endpoint-help"
              required
              onChange={(event) =>
                onChange({ ...draft, endpoint: event.target.value })
              }
            />
            <small id="search-endpoint-help">
              HTTPS is required for non-loopback services.
            </small>
          </label>
          <label>
            <span>Credential reference</span>
            <DropdownSelect
              value={draft.credentialId}
              aria-describedby="search-credential-help"
              required={draft.kind === "serp_api"}
              onChange={(event) =>
                onChange({ ...draft, credentialId: event.target.value })
              }
            >
              <option value="">
                {draft.kind === "serp_api" ? "Select a credential" : "None"}
              </option>
              {credentials.map((credential) => (
                <option key={credential.id} value={credential.id}>
                  {credential.label}
                </option>
              ))}
            </DropdownSelect>
            <small id="search-credential-help">
              The credential value stays in secure system storage.
            </small>
          </label>
        </div>
      </section>
      <details className="search-editor-advanced">
        <summary>
          <span>
            <strong>Request controls</strong>
            <small>Optional authentication header and request timeout</small>
          </span>
          <IconChevronDown size={16} aria-hidden="true" />
        </summary>
        <div className="search-editor-grid search-editor-request-grid">
          {draft.kind === "searxng" ? (
            <label>
              <span>Authentication header</span>
              <input
                value={draft.authHeader}
                placeholder="X-Searxng-Key"
                aria-describedby="search-auth-header-help"
                onChange={(event) =>
                  onChange({ ...draft, authHeader: event.target.value })
                }
              />
              <small id="search-auth-header-help">
                Header used when a credential is attached.
              </small>
            </label>
          ) : null}
          <label>
            <span>Timeout (ms)</span>
            <input
              aria-label="Timeout (ms)"
              type="number"
              min={1}
              max={300_000}
              value={draft.timeoutMs}
              aria-describedby="search-timeout-help"
              onChange={(event) =>
                onChange({ ...draft, timeoutMs: Number(event.target.value) })
              }
            />
            <small id="search-timeout-help">
              Allowed range: 1–300,000 milliseconds.
            </small>
          </label>
        </div>
      </details>
      <div className="mcp-editor-actions">
        <button className="button secondary" type="button" onClick={onCancel}>
          Cancel
        </button>
        <button className="button primary" type="submit" disabled={busy}>
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add search service"}
        </button>
      </div>
    </form>
  );
}

function TelemetryEditor({
  draft,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  draft: TelemetryEditorDraft;
  busy: boolean;
  onChange: (draft: TelemetryEditorDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  return (
    <form
      className="mcp-editor catalog-editor telemetry-editor"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="telemetry-editor-heading">
        <span className="resource-icon">
          <IconActivityHeartbeat size={18} aria-hidden="true" />
        </span>
        <div>
          <h4>
            {draft.resourceId
              ? `Edit ${draft.label}`
              : "Add telemetry connection"}
          </h4>
          <p>
            Choose where telemetry is sent and what audit information is saved.
          </p>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close telemetry editor"
          onClick={onCancel}
        >
          <IconX size={17} />
        </button>
      </div>
      <section
        className="telemetry-editor-section"
        aria-labelledby="telemetry-identity-heading"
      >
        <div className="telemetry-editor-section-heading">
          <h5 id="telemetry-identity-heading">Identity</h5>
          <p>
            Give this connection a recognizable name and choose the service name
            sent with telemetry.
          </p>
        </div>
        <div className="telemetry-editor-grid telemetry-identity-grid">
          <label>
            <span>Display label</span>
            <input
              required
              value={draft.label}
              placeholder="Production observability"
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small>Shown in settings and workspace selectors.</small>
          </label>
          <label>
            <span>Service name</span>
            <input
              required
              value={draft.name}
              onChange={(event) =>
                onChange({ ...draft, name: event.target.value })
              }
            />
            <small>Emitted as the OpenTelemetry service identity.</small>
          </label>
        </div>
      </section>
      <section
        className="telemetry-editor-section"
        aria-labelledby="telemetry-destination-heading"
      >
        <div className="telemetry-editor-section-heading">
          <h5 id="telemetry-destination-heading">Collector destination</h5>
          <p>
            Choose where telemetry is sent and how long Colossus waits for the
            collector.
          </p>
        </div>
        <div className="telemetry-editor-grid telemetry-destination-grid">
          <label>
            <span>Collector endpoint</span>
            <input
              type="url"
              required={
                draft.tracesEnabled || draft.metricsEnabled || draft.logsOtlp
              }
              value={draft.endpoint ?? ""}
              placeholder="https://collector.example.com:4317"
              onChange={(event) =>
                onChange({ ...draft, endpoint: event.target.value })
              }
            />
            <small>Required while any OTLP signal is enabled.</small>
          </label>
          <label>
            <span>Protocol</span>
            <DropdownSelect
              aria-label="Protocol"
              value={draft.protocol}
              onChange={(event) =>
                onChange({
                  ...draft,
                  protocol: event.target
                    .value as ManagedTelemetryProfile["protocol"],
                })
              }
            >
              <option value="grpc">OTLP gRPC</option>
              <option value="http_protobuf">OTLP HTTP/protobuf</option>
            </DropdownSelect>
            <small>
              The collector must support the selected OTLP transport.
            </small>
          </label>
          <label>
            <span>Timeout (ms)</span>
            <input
              aria-label="Timeout (ms)"
              type="number"
              min={100}
              max={120_000}
              value={draft.timeoutMs}
              onChange={(event) =>
                onChange({ ...draft, timeoutMs: Number(event.target.value) })
              }
            />
            <small>Allowed range: 100–120,000 milliseconds.</small>
          </label>
        </div>
      </section>
      <section
        className="telemetry-editor-section"
        aria-labelledby="telemetry-signals-heading"
      >
        <div className="telemetry-editor-section-heading">
          <h5 id="telemetry-signals-heading">Signals</h5>
          <p>Choose which telemetry data to send.</p>
        </div>
        <div className="telemetry-signal-grid">
          <article className="telemetry-signal-card">
            <label className="telemetry-signal-toggle">
              <span>
                <strong>Traces</strong>
                <small>
                  Send a sample of operation traces to the collector.
                </small>
              </span>
              <SwitchInput
                aria-label="Traces"
                checked={draft.tracesEnabled}
                onChange={(event) =>
                  onChange({ ...draft, tracesEnabled: event.target.checked })
                }
              />
            </label>
            <label>
              <span>Trace sample (millionths)</span>
              <input
                aria-label="Trace sample (millionths)"
                type="number"
                min={0}
                max={1_000_000}
                disabled={!draft.tracesEnabled}
                value={draft.traceSampleRatioMillionths}
                onChange={(event) =>
                  onChange({
                    ...draft,
                    traceSampleRatioMillionths: Number(event.target.value),
                  })
                }
              />
              <small>
                {(draft.traceSampleRatioMillionths / 10_000).toLocaleString()}%
                of traces.
              </small>
            </label>
          </article>
          <article className="telemetry-signal-card">
            <label className="telemetry-signal-toggle">
              <span>
                <strong>Metrics</strong>
                <small>Export runtime and operational measurements.</small>
              </span>
              <SwitchInput
                aria-label="Metrics"
                checked={draft.metricsEnabled}
                onChange={(event) =>
                  onChange({ ...draft, metricsEnabled: event.target.checked })
                }
              />
            </label>
            <label>
              <span>Metric interval (ms)</span>
              <input
                type="number"
                min={1_000}
                max={300_000}
                disabled={!draft.metricsEnabled}
                value={draft.metricExportIntervalMs}
                onChange={(event) =>
                  onChange({
                    ...draft,
                    metricExportIntervalMs: Number(event.target.value),
                  })
                }
              />
              <small>Allowed range: 1,000–300,000 milliseconds.</small>
            </label>
          </article>
          <article className="telemetry-signal-card telemetry-log-card">
            <label className="telemetry-signal-toggle">
              <span>
                <strong>OTLP logs</strong>
                <small>Send structured runtime logs to the collector.</small>
              </span>
              <SwitchInput
                aria-label="OTLP logs"
                checked={draft.logsOtlp}
                onChange={(event) =>
                  onChange({ ...draft, logsOtlp: event.target.checked })
                }
              />
            </label>
            <label className="telemetry-signal-toggle telemetry-stdout-toggle">
              <span>
                <strong>JSON stdout</strong>
                <small>Also emit machine-readable local logs.</small>
              </span>
              <SwitchInput
                aria-label="JSON stdout"
                checked={draft.logsStdoutJson}
                onChange={(event) =>
                  onChange({ ...draft, logsStdoutJson: event.target.checked })
                }
              />
            </label>
          </article>
        </div>
      </section>
      <section
        className="telemetry-editor-section"
        aria-labelledby="telemetry-disclosure-heading"
      >
        <div className="telemetry-editor-section-heading">
          <h5 id="telemetry-disclosure-heading">Audit record content</h5>
          <p>
            Choose whether audit records include no payloads, metadata only, or
            full content.
          </p>
        </div>
        <div className="telemetry-editor-grid telemetry-disclosure-grid">
          <label>
            <span>Audit content</span>
            <DropdownSelect
              aria-label="Audit content"
              value={draft.journalPayloads}
              onChange={(event) =>
                onChange({
                  ...draft,
                  journalPayloads: event.target
                    .value as ManagedTelemetryProfile["journalPayloads"],
                  acknowledgeSensitiveContent:
                    event.target.value === "full"
                      ? draft.acknowledgeSensitiveContent
                      : false,
                })
              }
            >
              <option value="disabled">Disabled</option>
              <option value="metadata">Metadata only</option>
              <option value="full">Full sensitive payloads</option>
            </DropdownSelect>
            <small>Metadata is the safer default for audit correlation.</small>
          </label>
          <label className="telemetry-signal-toggle telemetry-insecure-toggle">
            <span>
              <strong>Allow insecure remote transport</strong>
              <small>
                Allow unencrypted traffic to a non-local collector. Use only on
                a trusted network.
              </small>
            </span>
            <SwitchInput
              aria-label="Allow insecure remote transport"
              checked={draft.acknowledgeInsecureTransport}
              onChange={(event) =>
                onChange({
                  ...draft,
                  acknowledgeInsecureTransport: event.target.checked,
                })
              }
            />
          </label>
        </div>
        {draft.journalPayloads === "full" ? (
          <label className="compact-switch authority-warning telemetry-sensitive-acknowledgment">
            <SwitchInput
              aria-label="Acknowledge sensitive journal content"
              checked={draft.acknowledgeSensitiveContent}
              onChange={(event) =>
                onChange({
                  ...draft,
                  acknowledgeSensitiveContent: event.target.checked,
                })
              }
            />
            <span>
              I understand that full audit records may contain sensitive
              prompts, responses, and tool data
              <small>
                You must acknowledge this risk before saving the connection.
              </small>
            </span>
          </label>
        ) : null}
      </section>
      <section
        className="telemetry-editor-section"
        aria-labelledby="telemetry-resource-heading"
      >
        <div className="telemetry-editor-section-heading">
          <h5 id="telemetry-resource-heading">Resource attributes</h5>
          <p>
            Add key-value tags that help organize telemetry in your monitoring
            system.
          </p>
        </div>
        <label className="telemetry-resource-attributes">
          <span>Attributes</span>
          <textarea
            rows={4}
            value={draft.resourceAttributesText}
            placeholder="deployment.environment=development"
            onChange={(event) =>
              onChange({ ...draft, resourceAttributesText: event.target.value })
            }
          />
          <small>Enter one key=value attribute per line.</small>
        </label>
      </section>
      <div className="mcp-editor-actions">
        <button className="button secondary" type="button" onClick={onCancel}>
          Cancel
        </button>
        <button
          className="button primary"
          type="submit"
          disabled={
            busy ||
            (draft.journalPayloads === "full" &&
              !draft.acknowledgeSensitiveContent)
          }
        >
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add telemetry connection"}
        </button>
      </div>
    </form>
  );
}

function ProviderEditor({
  draft,
  credentials,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  draft: ProviderEditorDraft;
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  busy: boolean;
  onChange: (draft: ProviderEditorDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const codex = draft.kind === "open_ai_codex";
  const adapterDescription =
    draft.kind === "openai_responses"
      ? "Use for providers that support the OpenAI Responses API."
      : draft.kind === "openai_compatible"
        ? "Use for providers with an OpenAI-compatible API."
        : "Use your Codex subscription; the endpoint and sign-in are managed for you.";
  const title = draft.resourceId
    ? `Edit ${draft.label || "provider"}`
    : "Add provider";
  return (
    <form
      className="mcp-editor catalog-editor provider-editor"
      aria-labelledby="provider-editor-heading"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="provider-editor-heading">
        <span className="resource-icon">
          <IconCloud size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id="provider-editor-heading">{title}</h4>
          <p>
            Connect Colossus to a model provider. You can reuse this connection
            for more than one model.
          </p>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close provider editor"
          onClick={onCancel}
        >
          <IconX size={17} />
        </button>
      </div>
      <section
        className="provider-editor-section"
        aria-labelledby="provider-editor-identity-heading"
      >
        <div className="provider-editor-section-heading">
          <h5 id="provider-editor-identity-heading">Identity</h5>
          <p>
            Choose a name shown in settings and an ID used in configuration.
          </p>
        </div>
        <div className="provider-editor-grid">
          <label>
            <span>Display label</span>
            <input
              value={draft.label}
              placeholder="For example, OpenRouter production"
              aria-describedby="provider-label-help"
              required
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small id="provider-label-help">Shown to people in settings.</small>
          </label>
          <label>
            <span>Profile ID</span>
            <input
              value={draft.profile}
              placeholder="openrouter-production"
              aria-describedby="provider-profile-help"
              required
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
            <small id="provider-profile-help">
              Used to identify this provider in model configuration.
            </small>
          </label>
        </div>
      </section>
      <section
        className="provider-editor-section"
        aria-labelledby="provider-editor-connection-heading"
      >
        <div className="provider-editor-section-heading">
          <h5 id="provider-editor-connection-heading">Connection</h5>
          <p>Choose the provider API format and endpoint.</p>
        </div>
        <div className="provider-editor-grid provider-connection-grid">
          <label>
            <span>Adapter</span>
            <DropdownSelect
              value={draft.kind}
              aria-describedby="provider-adapter-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  kind: event.target
                    .value as ManagedProviderCatalogValue["kind"],
                })
              }
            >
              <option value="openai_responses">OpenAI Responses</option>
              <option value="openai_compatible">OpenAI compatible</option>
              <option value="open_ai_codex">Codex subscription</option>
            </DropdownSelect>
            <small id="provider-adapter-help">{adapterDescription}</small>
          </label>
          <label>
            <span>Endpoint URL</span>
            <input
              type="url"
              value={
                codex ? "https://chatgpt.com/backend-api/codex" : draft.baseUrl
              }
              disabled={codex}
              aria-describedby="provider-endpoint-help"
              required
              onChange={(event) =>
                onChange({ ...draft, baseUrl: event.target.value })
              }
            />
            <small id="provider-endpoint-help">
              {codex
                ? "This endpoint is managed by the Codex subscription connection."
                : "Use HTTPS except when connecting to a service on this device."}
            </small>
          </label>
        </div>
      </section>
      <section
        className="provider-editor-section"
        aria-labelledby="provider-editor-auth-heading"
      >
        <div className="provider-editor-section-heading">
          <h5 id="provider-editor-auth-heading">Authentication</h5>
          <p>Choose a saved credential when this provider requires one.</p>
        </div>
        <div className="provider-auth-grid">
          <label>
            <span>Credential reference</span>
            <DropdownSelect
              value={codex ? "" : draft.credentialId}
              disabled={codex}
              aria-describedby="provider-credential-help"
              onChange={(event) =>
                onChange({ ...draft, credentialId: event.target.value })
              }
            >
              <option value="">None</option>
              {credentials.map((credential) => (
                <option key={credential.id} value={credential.id}>
                  {credential.label}
                </option>
              ))}
            </DropdownSelect>
            <small id="provider-credential-help">
              {codex
                ? "Uses your signed-in Codex account instead."
                : "The credential value stays in secure system storage."}
            </small>
          </label>
          <div className="provider-auth-note">
            <IconShield size={18} aria-hidden="true" />
            <span>
              <strong>{codex ? "Codex account" : "Stored securely"}</strong>
              <small>
                {codex
                  ? "Uses your existing Codex subscription sign-in."
                  : "Settings store which credential to use, not its secret value."}
              </small>
            </span>
          </div>
        </div>
      </section>
      <section
        className="provider-editor-section"
        aria-labelledby="provider-editor-runtime-heading"
      >
        <div className="provider-editor-section-heading">
          <h5 id="provider-editor-runtime-heading">Runtime</h5>
          <p>
            Set a custom request timeout, or leave it blank to use the provider
            default.
          </p>
        </div>
        <div className="provider-runtime-control">
          <label>
            <span>Request timeout (ms)</span>
            <input
              type="number"
              min={1}
              max={3_600_000}
              value={draft.timeoutMs ?? ""}
              placeholder="Provider default"
              aria-describedby="provider-timeout-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  timeoutMs:
                    event.target.value === ""
                      ? null
                      : Number(event.target.value),
                })
              }
            />
            <small id="provider-timeout-help">
              Leave empty to use the provider default, up to one hour.
            </small>
          </label>
        </div>
      </section>
      <div className="mcp-editor-actions">
        <button className="button secondary" type="button" onClick={onCancel}>
          Cancel
        </button>
        <button className="button primary" type="submit" disabled={busy}>
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add provider"}
        </button>
      </div>
    </form>
  );
}

function ModelEditor({
  draft,
  providers,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  draft: ModelEditorDraft;
  providers: CatalogEntry<ManagedProviderCatalogValue>[];
  busy: boolean;
  onChange: (draft: ModelEditorDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const title = draft.resourceId
    ? `Edit ${draft.label || "model"}`
    : "Add model";
  return (
    <form
      className="mcp-editor catalog-editor model-editor"
      aria-labelledby="model-editor-heading"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="model-editor-heading">
        <span className="resource-icon">
          <IconCpu size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id="model-editor-heading">{title}</h4>
          <p>Add a model and describe the limits and features it supports.</p>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close model editor"
          onClick={onCancel}
        >
          <IconX size={17} />
        </button>
      </div>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-identity-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-identity-heading">Identity</h5>
          <p>
            Choose a name shown in settings and an ID used in configuration.
          </p>
        </div>
        <div className="model-editor-grid model-identity-grid">
          <label>
            <span>Display label</span>
            <input
              value={draft.label}
              placeholder="For example, Primary reasoning model"
              aria-describedby="model-label-help"
              required
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small id="model-label-help">Shown to people in settings.</small>
          </label>
          <label>
            <span>Profile ID</span>
            <input
              value={draft.profile}
              placeholder="primary-reasoning"
              aria-describedby="model-profile-help"
              required
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
            <small id="model-profile-help">
              Used to identify this model in workspace configuration.
            </small>
          </label>
        </div>
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-route-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-route-heading">Provider connection</h5>
          <p>Choose the provider and enter its model ID.</p>
        </div>
        <div className="model-editor-grid model-route-grid">
          <label>
            <span>Provider profile</span>
            <DropdownSelect
              value={draft.providerProfile}
              aria-describedby="model-provider-help"
              required
              onChange={(event) =>
                onChange({ ...draft, providerProfile: event.target.value })
              }
            >
              <option value="">Select provider</option>
              {providers.map((provider) => {
                const value = currentValue(provider);
                return (
                  <option key={provider.id} value={value.profile}>
                    {provider.label}
                  </option>
                );
              })}
            </DropdownSelect>
            <small id="model-provider-help">
              Uses one of the provider connections configured above.
            </small>
          </label>
          <label>
            <span>Model identifier</span>
            <input
              value={draft.model}
              placeholder="Provider-specific model name"
              aria-describedby="model-identifier-help"
              required
              onChange={(event) =>
                onChange({ ...draft, model: event.target.value })
              }
            />
            <small id="model-identifier-help">
              Enter the model name expected by the selected provider.
            </small>
          </label>
        </div>
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-budget-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-budget-heading">Token limits</h5>
          <p>
            Set how much context the model can read and how long it can reply.
          </p>
        </div>
        <div className="model-editor-grid model-budget-grid">
          <label>
            <span>Context window (tokens)</span>
            <input
              type="number"
              min={1_024}
              value={draft.contextWindowTokens}
              aria-describedby="model-context-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  contextWindowTokens: Number(event.target.value),
                })
              }
            />
            <small id="model-context-help">
              Minimum 1,024 tokens, including input and output.
            </small>
          </label>
          <label>
            <span>Maximum output (tokens)</span>
            <input
              type="number"
              min={1}
              value={draft.maxOutputTokens}
              aria-describedby="model-output-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  maxOutputTokens: Number(event.target.value),
                })
              }
            />
            <small id="model-output-help">
              Upper bound for one generated model response.
            </small>
          </label>
        </div>
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-capabilities-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-capabilities-heading">Supported features</h5>
          <p>Turn on only the features supported by this model.</p>
        </div>
        <div className="model-capability-grid">
          <label className="compact-switch model-capability-toggle">
            <input
              type="checkbox"
              checked={draft.toolCalls}
              onChange={(event) =>
                onChange({ ...draft, toolCalls: event.target.checked })
              }
            />
            <span>
              <strong>Tool calls</strong>
              <small>Let the model request tools.</small>
            </span>
          </label>
          <label className="compact-switch model-capability-toggle">
            <input
              type="checkbox"
              checked={draft.streaming}
              onChange={(event) =>
                onChange({ ...draft, streaming: event.target.checked })
              }
            />
            <span>
              <strong>Streaming</strong>
              <small>Show response text as it arrives.</small>
            </span>
          </label>
          <label className="compact-switch model-capability-toggle">
            <input
              type="checkbox"
              checked={draft.imageInputs}
              onChange={(event) =>
                onChange({ ...draft, imageInputs: event.target.checked })
              }
            />
            <span>
              <strong>Image inputs</strong>
              <small>Allow this model to receive attached images.</small>
            </span>
          </label>
        </div>
        <div className="model-reasoning-control">
          <label>
            <span>Reasoning effort</span>
            <DropdownSelect
              value={draft.reasoningEffort ?? "inherit"}
              aria-describedby="model-reasoning-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  reasoningEffort:
                    event.target.value === "inherit"
                      ? null
                      : (event.target.value as ReasoningEffort),
                })
              }
            >
              <option value="inherit">Provider default</option>
              {[
                "none",
                "minimal",
                "low",
                "medium",
                "high",
                "xhigh",
                "max",
                "ultra",
              ].map((effort) => (
                <option key={effort} value={effort}>
                  {effort}
                </option>
              ))}
            </DropdownSelect>
            <small id="model-reasoning-help">
              Leave at provider default unless the provider supports a specific
              effort level.
            </small>
          </label>
        </div>
      </section>
      <div className="mcp-editor-actions">
        <button className="button secondary" type="button" onClick={onCancel}>
          Cancel
        </button>
        <button className="button primary" type="submit" disabled={busy}>
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add model"}
        </button>
      </div>
    </form>
  );
}

export function McpEditor({
  draft,
  credentials,
  busy,
  onChange,
  onCancel,
  onSave,
}: {
  draft: McpEditorDraft;
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  busy: boolean;
  onChange: (draft: McpEditorDraft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  return (
    <form
      className="mcp-editor mcp-server-editor"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="mcp-editor-heading">
        <div>
          <h4>{draft.resourceId ? "Edit MCP server" : "Add MCP server"}</h4>
          <p>
            {draft.resourceId
              ? "Update how Colossus connects and what this server can access."
              : "Connect a local process or an HTTP endpoint to Colossus."}
          </p>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close MCP editor"
          onClick={onCancel}
        >
          <IconX size={17} />
        </button>
      </div>
      <section
        className="mcp-editor-section"
        aria-labelledby="mcp-connection-heading"
      >
        <div className="mcp-editor-section-heading">
          <h5 id="mcp-connection-heading">Connection</h5>
          <p>Where the server runs and how Colossus connects to it.</p>
        </div>
        <div className="mcp-editor-section-grid">
          <label>
            <span>Server name</span>
            <input
              aria-label="Server name"
              value={draft.name}
              required
              placeholder="for example, splunk"
              onChange={(event) =>
                onChange(updateMcpDraftName(draft, event.target.value))
              }
            />
            <small>
              Identifies this server in configuration and prefixes its tools.
            </small>
          </label>
          <label>
            <span>Transport</span>
            <DropdownSelect
              aria-label="Transport"
              value={draft.transport}
              onChange={(event) =>
                onChange({
                  ...draft,
                  transport: event.target
                    .value as ManagedMcpServer["transport"],
                  allowStateless:
                    event.target.value === "streamable_http"
                      ? draft.allowStateless
                      : false,
                })
              }
            >
              <option value="stdio">Local process (stdio)</option>
              <option value="streamable_http">Remote endpoint (HTTP)</option>
            </DropdownSelect>
          </label>
          <label className="mcp-editor-wide">
            <span>
              {draft.transport === "stdio" ? "Executable" : "Server URL"}
            </span>
            <input
              value={draft.commandOrUrl}
              required
              placeholder={
                draft.transport === "stdio"
                  ? "Absolute path or executable on PATH"
                  : "https://example.com/mcp"
              }
              onChange={(event) =>
                onChange({ ...draft, commandOrUrl: event.target.value })
              }
            />
          </label>
          {draft.transport === "stdio" ? (
            <>
              <label className="mcp-editor-wide">
                <span>Arguments</span>
                <textarea
                  rows={2}
                  value={draft.argsText}
                  placeholder="One exact argument per line"
                  onChange={(event) =>
                    onChange({ ...draft, argsText: event.target.value })
                  }
                />
              </label>
              <label className="mcp-editor-wide">
                <span>Working directory</span>
                <input
                  value={draft.workingDirectory}
                  placeholder="Runtime default"
                  onChange={(event) =>
                    onChange({ ...draft, workingDirectory: event.target.value })
                  }
                />
              </label>
            </>
          ) : (
            <>
              <label className="mcp-editor-wide">
                <span>Static headers (non-secret JSON)</span>
                <JsonFieldEditor
                  label="Static headers"
                  value={draft.headers}
                  onChange={(headers) =>
                    onChange({
                      ...draft,
                      headers: isStringRecord(headers)
                        ? headers
                        : draft.headers,
                    })
                  }
                />
                <small>
                  Use credential headers below for every secret value.
                </small>
              </label>
              <label className="compact-switch mcp-editor-wide">
                <input
                  type="checkbox"
                  checked={draft.allowStateless}
                  onChange={(event) =>
                    onChange({ ...draft, allowStateless: event.target.checked })
                  }
                />
                <span>
                  Allow stateless HTTP
                  <small>
                    Use only when the endpoint documents stateless MCP support.
                  </small>
                </span>
              </label>
            </>
          )}
        </div>
      </section>

      <section
        className="mcp-editor-section"
        aria-labelledby="mcp-access-heading"
      >
        <div className="mcp-editor-section-heading">
          <h5 id="mcp-access-heading">Access</h5>
          <p>
            Limit available tools and bind secrets from the credential store.
          </p>
        </div>
        <div className="mcp-editor-section-grid">
          <label className="mcp-editor-wide">
            <span>Allowed tools</span>
            <textarea
              rows={2}
              value={draft.allowedToolsText}
              placeholder={
                "Use * for all tools, or enter one exact tool name per line"
              }
              onChange={(event) =>
                onChange({ ...draft, allowedToolsText: event.target.value })
              }
            />
          </label>
          {draft.transport === "stdio" ? (
            <CredentialBindingsEditor
              title="Environment credential bindings"
              nameLabel="Variable"
              bindings={draft.environmentCredentials}
              credentials={credentials}
              onChange={(environmentCredentials) =>
                onChange({ ...draft, environmentCredentials })
              }
            />
          ) : (
            <>
              <CredentialBindingsEditor
                title="Credential headers"
                nameLabel="Header"
                includeScheme
                disabled={draft.oauthEnabled}
                bindings={draft.credentialHeaders}
                credentials={credentials}
                onChange={(credentialHeaders) =>
                  onChange({ ...draft, credentialHeaders })
                }
              />
              <label className="compact-switch mcp-editor-wide">
                <input
                  type="checkbox"
                  checked={draft.oauthEnabled}
                  disabled={draft.credentialHeaders.length > 0}
                  onChange={(event) =>
                    onChange({ ...draft, oauthEnabled: event.target.checked })
                  }
                />
                <span>
                  OAuth client configuration
                  {draft.credentialHeaders.length > 0 ? (
                    <small>
                      Remove credential headers before enabling OAuth.
                    </small>
                  ) : null}
                </span>
              </label>
              {draft.oauthEnabled ? (
                <div className="mcp-oauth-editor mcp-editor-wide">
                  <label>
                    <span>Client ID</span>
                    <input
                      required
                      value={draft.oauthClientId}
                      onChange={(event) =>
                        onChange({
                          ...draft,
                          oauthClientId: event.target.value,
                        })
                      }
                    />
                  </label>
                  <label>
                    <span>Client secret credential</span>
                    <CredentialSelect
                      value={draft.oauthClientSecretCredentialId}
                      credentials={credentials}
                      onChange={(oauthClientSecretCredentialId) =>
                        onChange({ ...draft, oauthClientSecretCredentialId })
                      }
                    />
                  </label>
                  <label>
                    <span>Callback port</span>
                    <input
                      type="number"
                      min={1}
                      max={65_535}
                      value={draft.oauthCallbackPort}
                      onChange={(event) =>
                        onChange({
                          ...draft,
                          oauthCallbackPort: Number(event.target.value),
                        })
                      }
                    />
                  </label>
                  <label>
                    <span>Scopes</span>
                    <textarea
                      rows={2}
                      value={draft.oauthScopesText}
                      placeholder="One scope per line"
                      onChange={(event) =>
                        onChange({
                          ...draft,
                          oauthScopesText: event.target.value,
                        })
                      }
                    />
                  </label>
                </div>
              ) : null}
            </>
          )}
        </div>
      </section>

      <details className="mcp-editor-advanced">
        <summary>
          <span>
            <strong>Advanced settings</strong>
            <small>Display name, limits, and Research settings</small>
          </span>
        </summary>
        <div className="mcp-editor-section-grid">
          <label>
            <span>Display label (optional)</span>
            <input
              aria-label="Display label (optional)"
              value={draft.label === draft.name ? "" : draft.label}
              placeholder={draft.name || "Same as server name"}
              onChange={(event) =>
                onChange({
                  ...draft,
                  label: event.target.value || draft.name,
                })
              }
            />
            <small>
              Use a friendlier name in settings. The server name still
              identifies it in configuration.
            </small>
          </label>
          <div className="mcp-editor-limit-grid">
            <label>
              <span>Timeout (ms)</span>
              <input
                type="number"
                min={1}
                value={draft.timeoutMs ?? ""}
                placeholder="Runtime default"
                onChange={(event) =>
                  onChange({
                    ...draft,
                    timeoutMs:
                      event.target.value === ""
                        ? null
                        : Number(event.target.value),
                  })
                }
              />
            </label>
            <label>
              <span>Maximum output (bytes)</span>
              <input
                type="number"
                min={1}
                value={draft.maxOutputBytes ?? ""}
                placeholder="Runtime default"
                onChange={(event) =>
                  onChange({
                    ...draft,
                    maxOutputBytes:
                      event.target.value === ""
                        ? null
                        : Number(event.target.value),
                  })
                }
              />
            </label>
          </div>
          <label className="mcp-editor-wide">
            <span>Research tool projections</span>
            <JsonFieldEditor
              label="Research tool projections"
              value={draft.researchTools}
              onChange={(researchTools) =>
                onChange({
                  ...draft,
                  researchTools: Array.isArray(researchTools)
                    ? (researchTools as ManagedMcpResearchTool[])
                    : draft.researchTools,
                })
              }
            />
          </label>
        </div>
      </details>
      <div className="mcp-editor-actions">
        <button className="button secondary" type="button" onClick={onCancel}>
          Cancel
        </button>
        <button className="button primary" type="submit" disabled={busy}>
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add server"}
        </button>
      </div>
    </form>
  );
}

export function updateMcpDraftName(
  draft: McpEditorDraft,
  name: string,
): McpEditorDraft {
  const labelFollowsName = draft.label === "" || draft.label === draft.name;
  return {
    ...draft,
    name,
    label: labelFollowsName ? name : draft.label,
  };
}

function CredentialSelect({
  value,
  credentials,
  required = false,
  onChange,
}: {
  value: string;
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  required?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <DropdownSelect
      value={value}
      required={required}
      onChange={(event) => onChange(event.target.value)}
    >
      <option value="">None</option>
      {credentials.map((credential) => (
        <option key={credential.id} value={credential.id}>
          {credential.label}
        </option>
      ))}
    </DropdownSelect>
  );
}

function CredentialBindingsEditor({
  title,
  nameLabel,
  includeScheme = false,
  disabled = false,
  bindings,
  credentials,
  onChange,
}: {
  title: string;
  nameLabel: string;
  includeScheme?: boolean;
  disabled?: boolean;
  bindings: McpCredentialBindingDraft[];
  credentials: ManagedSettingsSnapshot["globalConfiguration"]["credentials"];
  onChange: (bindings: McpCredentialBindingDraft[]) => void;
}) {
  function update(index: number, patch: Partial<McpCredentialBindingDraft>) {
    onChange(
      bindings.map((binding, candidate) =>
        candidate === index ? { ...binding, ...patch } : binding,
      ),
    );
  }
  return (
    <fieldset className="mcp-bindings-editor mcp-editor-wide">
      <legend>{title}</legend>
      {bindings.map((binding, index) => (
        <div className="mcp-binding-row" key={`${binding.name}-${index}`}>
          <label>
            <span>{nameLabel}</span>
            <input
              required
              value={binding.name}
              onChange={(event) => update(index, { name: event.target.value })}
            />
          </label>
          {includeScheme ? (
            <label>
              <span>Scheme</span>
              <input
                value={binding.scheme}
                placeholder="Bearer"
                onChange={(event) =>
                  update(index, { scheme: event.target.value })
                }
              />
            </label>
          ) : null}
          <label>
            <span>Credential</span>
            <CredentialSelect
              value={binding.credentialId}
              credentials={credentials}
              required
              onChange={(credentialId) => update(index, { credentialId })}
            />
          </label>
          <button
            className="icon-button"
            type="button"
            aria-label={`Remove ${nameLabel.toLowerCase()} binding ${index + 1}`}
            onClick={() =>
              onChange(bindings.filter((_, candidate) => candidate !== index))
            }
          >
            <IconTrash size={16} />
          </button>
        </div>
      ))}
      <button
        className="button secondary"
        type="button"
        disabled={disabled}
        onClick={() =>
          onChange([...bindings, { name: "", credentialId: "", scheme: "" }])
        }
      >
        <IconPlus size={15} /> Add binding
      </button>
    </fieldset>
  );
}

export function mcpDraft(
  entry: CatalogEntry<ManagedMcpServer>,
): McpEditorDraft {
  const server = currentValue(entry);
  return {
    resourceId: entry.id,
    label: entry.label,
    name: server.name,
    transport: server.transport,
    commandOrUrl: server.command ?? server.url ?? "",
    argsText: server.args.join("\n"),
    workingDirectory: server.workingDirectory ?? "",
    environmentCredentials: Object.entries(server.environmentCredentials).map(
      ([name, credentialId]) => ({ name, credentialId, scheme: "" }),
    ),
    headers: server.headers,
    credentialHeaders: Object.entries(server.credentialHeaders).map(
      ([name, value]) => ({
        name,
        credentialId: value.credentialId,
        scheme: value.scheme ?? "",
      }),
    ),
    allowStateless: server.allowStateless,
    oauthEnabled: server.oauth !== null,
    oauthClientId: server.oauth?.clientId ?? "",
    oauthClientSecretCredentialId: server.oauth?.clientSecretCredentialId ?? "",
    oauthCallbackPort: server.oauth?.callbackPort ?? 0,
    oauthScopesText: server.oauth?.scopes.join("\n") ?? "",
    allowedToolsText: server.allowedTools.join("\n"),
    researchTools: server.researchTools,
    timeoutMs: server.timeoutMs,
    maxOutputBytes: server.maxOutputBytes,
  };
}

export function managedMcpServer(draft: McpEditorDraft): ManagedMcpServer {
  return {
    name: draft.name,
    transport: draft.transport,
    command: draft.transport === "stdio" ? draft.commandOrUrl : null,
    args: draft.transport === "stdio" ? splitLines(draft.argsText) : [],
    workingDirectory:
      draft.transport === "stdio" && draft.workingDirectory.trim()
        ? draft.workingDirectory.trim()
        : null,
    environmentCredentials:
      draft.transport === "stdio"
        ? credentialBindingRecord(draft.environmentCredentials)
        : {},
    url: draft.transport === "streamable_http" ? draft.commandOrUrl : null,
    headers: draft.transport === "streamable_http" ? draft.headers : {},
    credentialHeaders:
      draft.transport === "streamable_http"
        ? Object.fromEntries(
            draft.credentialHeaders
              .filter(({ name, credentialId }) => name && credentialId)
              .map(({ name, scheme, credentialId }) => [
                name,
                { scheme: scheme.trim() || null, credentialId },
              ]),
          )
        : {},
    allowStateless:
      draft.transport === "streamable_http" && draft.allowStateless,
    oauth:
      draft.transport === "streamable_http" && draft.oauthEnabled
        ? {
            clientId: draft.oauthClientId,
            clientSecretCredentialId:
              draft.oauthClientSecretCredentialId || null,
            callbackPort: draft.oauthCallbackPort,
            scopes: splitLines(draft.oauthScopesText),
          }
        : null,
    allowedTools: splitLines(draft.allowedToolsText),
    researchTools: draft.researchTools,
    timeoutMs: draft.timeoutMs,
    maxOutputBytes: draft.maxOutputBytes,
  };
}

export function managedProvider(
  draft: ProviderEditorDraft,
): ManagedProviderCatalogValue {
  const codex = draft.kind === "open_ai_codex";
  return {
    profile: draft.profile,
    kind: draft.kind,
    baseUrl: codex ? "https://chatgpt.com/backend-api/codex" : draft.baseUrl,
    credentialId: codex || !draft.credentialId ? null : draft.credentialId,
    timeoutMs: draft.timeoutMs,
  };
}

export function managedModel(
  draft: ModelEditorDraft,
): ManagedModelCatalogValue {
  return {
    profile: draft.profile,
    providerProfile: draft.providerProfile,
    model: draft.model,
    contextWindowTokens: draft.contextWindowTokens,
    maxOutputTokens: draft.maxOutputTokens,
    capabilities: {
      toolCalls: draft.toolCalls,
      streaming: draft.streaming,
      imageInputs: draft.imageInputs,
    },
    reasoningEffort: draft.reasoningEffort,
  };
}

export function managedTelemetry(
  draft: TelemetryEditorDraft,
): ManagedTelemetryProfile {
  const resourceAttributes = Object.fromEntries(
    draft.resourceAttributesText
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const separator = line.indexOf("=");
        return separator > 0
          ? [line.slice(0, separator).trim(), line.slice(separator + 1).trim()]
          : [line, ""];
      }),
  );
  return {
    name: draft.name,
    endpoint: draft.endpoint || null,
    protocol: draft.protocol,
    timeoutMs: draft.timeoutMs,
    tracesEnabled: draft.tracesEnabled,
    traceSampleRatioMillionths: draft.traceSampleRatioMillionths,
    metricsEnabled: draft.metricsEnabled,
    metricExportIntervalMs: draft.metricExportIntervalMs,
    logsOtlp: draft.logsOtlp,
    logsStdoutJson: draft.logsStdoutJson,
    journalPayloads: draft.journalPayloads,
    acknowledgeSensitiveContent: draft.acknowledgeSensitiveContent,
    acknowledgeInsecureTransport: draft.acknowledgeInsecureTransport,
    resourceAttributes,
  };
}

export function providerDraft(
  entry: CatalogEntry<ManagedProviderCatalogValue>,
): ProviderEditorDraft {
  const provider = currentValue(entry);
  return {
    resourceId: entry.id,
    label: entry.label,
    profile: provider.profile,
    kind: provider.kind,
    baseUrl: provider.baseUrl,
    credentialId: provider.credentialId ?? "",
    timeoutMs: provider.timeoutMs ?? null,
  };
}

function searchDraft(
  entry: CatalogEntry<ManagedSearchProvider>,
): SearchEditorDraft {
  const search = currentValue(entry);
  return {
    resourceId: entry.id,
    label: entry.label,
    profile: search.profile,
    kind: search.kind,
    endpoint: search.endpoint,
    credentialId: search.credentialId ?? "",
    authHeader: search.authHeader ?? "",
    timeoutMs: search.timeoutMs,
  };
}

export function telemetryDraft(
  entry: CatalogEntry<ManagedTelemetryProfile>,
): TelemetryEditorDraft {
  const telemetry = currentValue(entry);
  return {
    ...telemetry,
    resourceId: entry.id,
    label: entry.label,
    resourceAttributesText: Object.entries(telemetry.resourceAttributes)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
  };
}

export function modelDraft(
  entry: CatalogEntry<ManagedModelCatalogValue>,
): ModelEditorDraft {
  const model = currentValue(entry);
  return {
    resourceId: entry.id,
    label: entry.label,
    profile: model.profile,
    providerProfile: model.providerProfile,
    model: model.model,
    contextWindowTokens: model.contextWindowTokens,
    maxOutputTokens: model.maxOutputTokens,
    toolCalls: model.capabilities.toolCalls,
    streaming: model.capabilities.streaming,
    imageInputs: model.capabilities.imageInputs,
    reasoningEffort: model.reasoningEffort,
  };
}

function DesktopSettings(
  props: Omit<ManagedSettingsPaneProps, "desktop"> & {
    desktop: DesktopStatus;
    externalTargets: RuntimeTarget[];
  },
) {
  const {
    desktop,
    connecting,
    updateChecking,
    updateMessage,
    externalTargets,
  } = props;
  const terminalAvailable = desktop.targets.some(
    (target) => target.kind === "managed_local" && target.terminalAvailable,
  );
  const managedStateLabel = desktop.managedState
    .replaceAll("_", " ")
    .replace(/^./, (character) => character.toUpperCase());
  const releaseChannelLabel = desktop.releaseChannel
    .replaceAll("_", " ")
    .replace(/^./, (character) => character.toUpperCase());
  const terminalReadyCount = desktop.targets.filter(
    (target) => target.terminalAvailable,
  ).length;
  return (
    <section
      className="managed-settings-body desktop-settings"
      aria-labelledby="desktop-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <p className="eyebrow">Desktop app</p>
          <h3 id="desktop-settings-heading">Desktop settings</h3>
          <p className="managed-heading-copy">
            Manage the workspace and local services that belong to this Desktop
            installation.
          </p>
        </div>
        <div className="desktop-native-note" aria-label="Desktop-only controls">
          <IconLock size={18} aria-hidden="true" />
          <span>
            <strong>Desktop-only controls</strong>
            <small>
              Workspace paths, trust settings, and terminal access stay in the
              native app.
            </small>
          </span>
        </div>
      </div>
      <div
        className="managed-metric-strip desktop-metric-strip"
        aria-label="Desktop summary"
      >
        <Metric
          icon={<IconDatabase size={19} />}
          value={desktop.workspace ? 1 : 0}
          label="Managed workspace"
        />
        <Metric
          icon={<IconNetwork size={19} />}
          value={externalTargets.length}
          label="External runtimes"
        />
        <Metric
          icon={<IconTerminal2 size={19} />}
          value={terminalReadyCount}
          label="Terminal-ready runtimes"
        />
      </div>

      <AppearanceSettings />

      <section
        className="desktop-control-panel"
        aria-labelledby="desktop-control-panel-heading"
      >
        <div className="desktop-panel-heading">
          <div>
            <h4 id="desktop-control-panel-heading">This Desktop</h4>
            <p>
              Control the local workspace, application updates, trust store, and
              terminal access.
            </p>
          </div>
          <span
            className={`status-chip${desktop.workspace ? " tone-success" : " tone-attention"}`}
          >
            {desktop.workspace ? "Workspace configured" : "Setup needed"}
          </span>
        </div>
        <div className="managed-list desktop-control-list" role="list">
          <div className="managed-list-row desktop-control-row" role="listitem">
            <span className="resource-icon">
              <IconDatabase size={18} aria-hidden="true" />
            </span>
            <div>
              <strong>
                {desktop.workspace?.displayName ?? "Managed workspace"}
              </strong>
              <small>
                {desktop.workspace?.displayPath ??
                  "Choose the repository this Desktop installation manages."}
              </small>
            </div>
            <span
              className={`status-chip${desktop.managedState === "ready" ? " tone-success" : desktop.managedState === "failed" ? " tone-attention" : " tone-neutral"}`}
            >
              {managedStateLabel}
            </span>
            <div className="resource-actions">
              <button
                className="button secondary"
                type="button"
                disabled={connecting}
                onClick={props.onChooseWorkspace}
              >
                Choose workspace
              </button>
              <button
                className="button secondary"
                type="button"
                disabled={connecting || !desktop.workspace}
                onClick={props.onConfigureManaged}
              >
                Configure runtime
              </button>
              <button
                className="icon-button"
                type="button"
                aria-label="Restart Managed Local"
                title="Restart Managed Local"
                disabled={connecting || !desktop.workspace}
                onClick={props.onRestartManaged}
              >
                <IconRefresh size={17} aria-hidden="true" />
              </button>
            </div>
          </div>
          <div className="managed-list-row desktop-control-row" role="listitem">
            <span className="resource-icon">
              <IconRefresh size={18} aria-hidden="true" />
            </span>
            <div>
              <strong>Desktop updates</strong>
              <small>
                {updateMessage ||
                  `Check for new builds from the ${releaseChannelLabel.toLowerCase()} channel.`}
              </small>
            </div>
            <span
              className={`status-chip${desktop.capabilities.updateAvailable ? " tone-attention" : " tone-neutral"}`}
            >
              {desktop.capabilities.updateAvailable
                ? "Update available"
                : `${releaseChannelLabel} channel`}
            </span>
            <div className="resource-actions">
              <button
                className="button secondary"
                type="button"
                disabled={updateChecking}
                onClick={props.onCheckForUpdates}
              >
                {updateChecking ? "Checking…" : "Check for updates"}
              </button>
              {desktop.capabilities.updateAvailable ? (
                <button
                  className="button primary"
                  type="button"
                  onClick={props.onInstallUpdate}
                >
                  Install update
                </button>
              ) : null}
            </div>
          </div>
          <div className="managed-list-row desktop-control-row" role="listitem">
            <span className="resource-icon">
              <IconShield size={18} aria-hidden="true" />
            </span>
            <div>
              <strong>Trusted certificates</strong>
              <small>
                {desktop.additionalCaBundle.configured
                  ? `${desktop.additionalCaBundle.certificateCount} additional certificate${desktop.additionalCaBundle.certificateCount === 1 ? "" : "s"} extend the system trust store.`
                  : "Use the system trust store, or import a PEM bundle for private services."}
              </small>
            </div>
            <span
              className={`status-chip${desktop.additionalCaBundle.configured ? " tone-success" : " tone-neutral"}`}
            >
              {desktop.additionalCaBundle.configured
                ? `${desktop.additionalCaBundle.certificateCount} imported`
                : "System trust"}
            </span>
            <div className="resource-actions">
              <button
                className="button secondary"
                type="button"
                onClick={props.onImportCaBundle}
              >
                Import PEM
              </button>
              {desktop.additionalCaBundle.configured ? (
                <button
                  className="button secondary"
                  type="button"
                  onClick={props.onRemoveCaBundle}
                >
                  Remove bundle
                </button>
              ) : null}
            </div>
          </div>
          <div className="managed-list-row desktop-control-row" role="listitem">
            <span className="resource-icon">
              <IconTerminal2 size={18} aria-hidden="true" />
            </span>
            <div>
              <strong>Local terminal</strong>
              <small>
                Open a shell or the Colossus TUI inside the managed workspace.
              </small>
            </div>
            <span
              className={`status-chip${desktop.terminalEnabled ? " tone-success" : " tone-neutral"}`}
            >
              {desktop.terminalEnabled ? "Enabled" : "Disabled"}
            </span>
            <div className="resource-actions desktop-terminal-actions">
              <label className="compact-switch">
                <input
                  className="switch-input"
                  type="checkbox"
                  checked={desktop.terminalEnabled}
                  disabled={!desktop.workspace || connecting}
                  onChange={(event) =>
                    props.onSetTerminalEnabled(event.target.checked)
                  }
                />
                <span>
                  {desktop.terminalEnabled ? "Enabled" : "Enable terminal"}
                </span>
              </label>
              {desktop.capabilities.shellTerminal ? (
                <button
                  className="button secondary"
                  type="button"
                  disabled={!desktop.terminalEnabled}
                  onClick={() => props.onOpenTerminal("shell")}
                >
                  Open Shell
                </button>
              ) : null}
              {desktop.capabilities.tui ? (
                <button
                  className="button secondary"
                  type="button"
                  disabled={!desktop.terminalEnabled || !terminalAvailable}
                  onClick={() => props.onOpenTerminal("colossus_tui")}
                >
                  Open Colossus TUI
                </button>
              ) : null}
            </div>
          </div>
        </div>
      </section>

      <div className="desktop-target-toolbar">
        <div>
          <h4>External runtimes</h4>
          <p>Connect to Colossus runtimes managed outside this Desktop app.</p>
        </div>
        <button
          className="button primary"
          type="button"
          onClick={props.onAddExternalTarget}
        >
          <IconPlus size={15} aria-hidden="true" />
          Add external runtime
        </button>
      </div>
      <section
        className="desktop-target-inventory"
        aria-labelledby="desktop-target-inventory-heading"
      >
        <div className="desktop-panel-heading">
          <div>
            <h4 id="desktop-target-inventory-heading">Saved runtimes</h4>
            <p>
              Review connection state and remove runtimes you no longer use.
            </p>
          </div>
          <span
            className="credential-count"
            aria-label={`${externalTargets.length} external runtimes`}
          >
            {externalTargets.length}
          </span>
        </div>
        <div
          className="managed-list desktop-target-list"
          role={externalTargets.length > 0 ? "list" : undefined}
        >
          {externalTargets.map((target) => (
            <div
              className="managed-list-row desktop-target-row"
              key={target.targetId}
              role="listitem"
            >
              <span className="resource-icon">
                <IconNetwork size={18} aria-hidden="true" />
              </span>
              <div>
                <strong>{target.label}</strong>
                <small>{target.message || "External Colossus runtime"}</small>
              </div>
              <span
                className={`status-chip${target.state === "ready" || target.state === "available" ? " tone-success" : target.state === "failed" || target.state === "unreachable" ? " tone-attention" : " tone-neutral"}`}
              >
                {target.state
                  .replaceAll("_", " ")
                  .replace(/^./, (character) => character.toUpperCase())}
              </span>
              <div className="resource-actions">
                <button
                  className="icon-button danger-icon-button"
                  type="button"
                  aria-label={`Remove ${target.label}`}
                  title={`Remove ${target.label}`}
                  onClick={() => props.onRemoveExternalTarget(target.targetId)}
                >
                  <IconTrash size={16} aria-hidden="true" />
                </button>
              </div>
            </div>
          ))}
          {externalTargets.length === 0 ? (
            <EmptySettings
              icon={<IconNetwork size={24} />}
              title="No external runtimes"
            />
          ) : null}
        </div>
      </section>
    </section>
  );
}

function Metric({
  icon,
  value,
  label,
}: {
  icon: React.ReactNode;
  value: number;
  label: string;
}) {
  return (
    <div>
      <span className="resource-icon">{icon}</span>
      <strong>{value}</strong>
      <small>{label}</small>
    </div>
  );
}

function EmptySettings({
  icon,
  title,
}: {
  icon: React.ReactNode;
  title: string;
}) {
  return (
    <div className="managed-empty">
      {icon}
      <strong>{title}</strong>
    </div>
  );
}

function SettingsSearchResults({
  results,
  onOpen,
}: {
  results: Array<{
    id: string;
    title: string;
    meta: string;
    scope: "field" | GlobalTab;
  }>;
  onOpen: (result: {
    id: string;
    title: string;
    meta: string;
    scope: "field" | GlobalTab;
  }) => void;
}) {
  return (
    <section className="managed-settings-body">
      <div className="managed-section-heading">
        <div>
          <p className="eyebrow">All configuration</p>
          <h3>Search results</h3>
        </div>
        <span>{results.length} matches</span>
      </div>
      <div className="managed-list">
        {results.map((result) => (
          <button
            className="managed-list-row search-result-row"
            type="button"
            key={`${result.scope}:${result.id}`}
            onClick={() => onOpen(result)}
          >
            <span className="resource-icon">
              <IconSearch size={17} />
            </span>
            <div>
              <strong>{result.title}</strong>
              <small>{result.meta}</small>
            </div>
            <IconChevronDown className="search-result-arrow" size={17} />
          </button>
        ))}
        {results.length === 0 ? (
          <EmptySettings
            icon={<IconSearch size={24} />}
            title="No matching settings"
          />
        ) : null}
      </div>
    </section>
  );
}

function removeDraftField(
  draft: SpaceDraft,
  setDraft: (draft: SpaceDraft) => void,
  id: string,
) {
  const fields = { ...draft.fields };
  delete fields[id];
  setDraft({ ...draft, fields });
}

function splitLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function credentialBindingRecord(
  bindings: McpCredentialBindingDraft[],
): Record<string, string> {
  return Object.fromEntries(
    bindings
      .filter(({ name, credentialId }) => name && credentialId)
      .map(({ name, credentialId }) => [name, credentialId]),
  );
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.values(value).every((entry) => typeof entry === "string")
  );
}

function fixtureImportProposal(
  spaceId: string,
): RepositoryConfigurationProposal {
  return {
    spaceId,
    relativePath: ".colossus/config.yaml",
    sha256: "8fda23c8409f3e724b4b5db9e30ddce2c834783c5820835de920de1765802137",
    previousSha256: null,
    changedSinceImport: false,
    resources: [
      {
        kind: "provider",
        sourceId: "openapi",
        label: "openapi",
        detail: "open ai compatible",
        conflict: false,
        existingResourceId: null,
      },
      {
        kind: "model",
        sourceId: "primary",
        label: "primary",
        detail: "model",
        conflict: true,
        existingResourceId: "fixture-model-primary",
      },
      {
        kind: "mcp",
        sourceId: "github-local",
        label: "github-local",
        detail: "stdio",
        conflict: false,
        existingResourceId: null,
      },
      {
        kind: "telemetry",
        sourceId: "observability",
        label: "colossus-desktop",
        detail: "OTLP telemetry profile",
        conflict: false,
        existingResourceId: null,
      },
    ],
    credentialSlots: [
      {
        slotId: "env:OPENAI_API_KEY",
        label: "OPENAI_API_KEY",
        consumers: ["runtime.providers.profiles.openapi.credentialReference"],
      },
    ],
    fieldOverrides: ["agent.maxTurns", "research.maxSources"],
    lockedFields: ["storage.path", "sandbox.backend"],
    warnings: [],
  };
}

function fixtureExtensionInventory(): ManagedExtensionInventory {
  return {
    skills: [
      {
        name: "incident-response",
        version: "1.3.0",
        description: "Structured incident triage and evidence handling.",
        source: "repository:incident-response",
        offlineCompatible: true,
      },
      {
        name: "release-review",
        version: "2.1.0",
        description: "Release readiness and regression review.",
        source: "bundled:release-review",
        offlineCompatible: true,
      },
    ],
    packs: [
      {
        name: "engineering-tools",
        version: "4.2.1",
        publisher: "Obscurity Labs",
        status: "enabled",
        manifestSha256: "5d8c0f12f65de8a2a7e61a3b4a8dd204",
        trusted: true,
      },
    ],
    workflows: [
      {
        name: "release",
        version: "3.2.1",
        status: "revised",
        updatedAt: "2026-08-20T12:00:00Z",
        revisionHash: "d7f184c2e2b0ac019f34c1cbec9e15e8",
      },
    ],
  };
}

function fixtureRuntimeDiagnostic(
  kind: "provider" | "model" | "search" | "telemetry",
  profile: string,
): ManagedRuntimeDiagnostic {
  return {
    kind,
    profile,
    ready: true,
    checks: [
      {
        name: "fixture",
        status: "pass",
        detail: `${kind} ${profile} completed its bounded readiness probe.`,
      },
    ],
    resultCount: kind === "search" ? 1 : null,
  };
}
