export type ConnectionState =
  | "connected"
  | "disconnected"
  | "not_configured"
  | "starting"
  | "restarting"
  | "stopping"
  | "failed";

export interface ConnectionStatus {
  state: ConnectionState;
  message: string;
  targetId: string | null;
}

export type RuntimeTargetKind = "managed_local" | "external_daemon";

export type ManagedRuntimeState =
  | "needs_workspace"
  | "needs_provider"
  | "starting"
  | "ready"
  | "restarting"
  | "stopping"
  | "failed";

export type RuntimeFailureCode =
  | "integrity"
  | "permission"
  | "workspace_busy"
  | "configuration"
  | "authentication"
  | "provider"
  | "crash_loop"
  | "transport"
  | "internal";

export interface WorkspaceSummary {
  workspaceId: string;
  displayName: string;
  displayPath: string;
}

export type SpaceRuntimeState = ManagedRuntimeState | "sleeping" | "archived";

export interface SpaceSummary {
  spaceId: string;
  targetId: string;
  displayName: string;
  displayPath: string;
  archived: boolean;
  lastOpenedAtMs: number;
  lastActivityAt: string | null;
  state: SpaceRuntimeState;
  message: string;
  selected: boolean;
  attentionCount: number;
  providerConfigured: boolean;
}

export interface SpaceSearchResult {
  spaceId: string;
  spaceName: string;
  targetId: string;
  runId: string;
  sessionId: string;
  title: string;
  mode: RunMode;
  status: RunStatus;
  updatedAt: string;
  archived: boolean;
  threadArchived: boolean;
  attention: boolean;
}

export interface SpaceSearchPage {
  results: SpaceSearchResult[];
  nextCursor: string;
}

export interface SpaceAttentionEvent {
  spaceId: string;
  targetId: string;
  attentionCount: number;
}

export interface SpaceStatusEvent {
  spaceId: string;
  targetId: string;
  displayName: string;
  archived: boolean;
  state: SpaceRuntimeState;
  selected: boolean;
  attentionCount: number;
  lastActivityAt: string | null;
}

export interface SearchSpaceThreadsRequest {
  query: string;
  spaceId?: string;
  includeArchived?: boolean;
  cursor?: string;
  pageSize?: number;
}

export type WorkspaceEntryKind = "directory" | "file";

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: WorkspaceEntryKind;
  sizeBytes: number | null;
}

export interface WorkspaceDirectory {
  path: string;
  entries: WorkspaceEntry[];
  truncated: boolean;
  excludedCount: number;
}

export interface WorkspaceFile {
  name: string;
  path: string;
  content: string;
  language: string;
  sizeBytes: number;
  lineCount: number;
}

export interface RuntimeTarget {
  targetId: string;
  kind: RuntimeTargetKind;
  label: string;
  state:
    | ManagedRuntimeState
    | "sleeping"
    | "archived"
    | "disconnected"
    | "checking"
    | "available"
    | "unreachable";
  message: string;
  selected: boolean;
  terminalAvailable: boolean;
  workspace: WorkspaceSummary | null;
  failureCode: RuntimeFailureCode | null;
}

export interface ProviderSummary {
  configured: boolean;
  kind: ProviderKind | null;
  model: string;
}

export type ProviderKind =
  "openai_responses" | "openai_compatible" | "open_ai_codex";

export type ReasoningEffort =
  "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra";

export interface CodexAuthStatus {
  state: "signed_in" | "signed_out" | "unavailable";
  message: string;
}

export interface ManagedProviderConfiguration {
  profile: string;
  providerKind: ProviderKind;
  baseUrl: string;
  hasCredential: boolean;
  timeoutMs: number | null;
  effectiveTimeoutMs: number;
}

export interface ManagedModelCapabilities {
  toolCalls: boolean;
  streaming: boolean;
}

export interface ManagedModelConfiguration {
  profile: string;
  providerProfile: string;
  model: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
  capabilities: ManagedModelCapabilities;
  reasoningEffort: ReasoningEffort | null;
}

export interface ManagedConfiguration {
  providers: ManagedProviderConfiguration[];
  models: ManagedModelConfiguration[];
  roles: Record<string, string>;
}

export type DesktopReleaseChannel =
  "development" | "stable" | "developer_preview" | "validation_only";

export interface DesktopReleaseMetadata {
  platform: "macos" | "windows" | "unsupported";
  architecture: string;
  channel: DesktopReleaseChannel;
  bundleIntegrity: "verified" | "failed";
  codeSigning:
    "development" | "verified" | "ad_hoc" | "unsigned" | "unsupported";
}

export interface DesktopUpdateCheck {
  configured: boolean;
  available: boolean;
  currentVersion: string;
  version: string | null;
  channel: DesktopReleaseChannel;
}

export interface DesktopStatus {
  releaseChannel: DesktopReleaseChannel;
  connection: ConnectionStatus;
  targets: RuntimeTarget[];
  selectedTargetId: string | null;
  spaces: SpaceSummary[];
  selectedSpaceId: string | null;
  managedState: ManagedRuntimeState;
  workspace: WorkspaceSummary | null;
  provider: ProviderSummary;
  codexAuth: CodexAuthStatus;
  managedModelConfiguration: ManagedConfiguration;
  accessProfile: AccessProfile;
  executionBoundary: ExecutionBoundary;
  approvalMode: ApprovalMode;
  terminalEnabled: boolean;
  additionalCaBundle: CaBundleStatus;
  capabilities: DesktopCapabilities;
}

export type ApprovalMode = "deny" | "ask" | "risk_auto" | "full_access";
export type AccessProfile = "minimal" | "pinned" | "development" | "allow_all";
export type ExecutionBoundary =
  "full_access" | "workspace_isolated" | "offline_isolated";

export interface CaBundleStatus {
  configured: boolean;
  certificateCount: number;
  fingerprintsSha256: string[];
}

export interface DesktopCapabilities {
  research?: boolean;
  delegation: boolean;
  skills: boolean;
  tui: boolean;
  shellTerminal: boolean;
  files: boolean;
  artifacts: boolean;
  planContinuation: boolean;
  updateAvailable: boolean;
  agentWorkflows: boolean;
  attachments: boolean;
}

export interface ConfigureManagedRuntimeRequest {
  workspaceId: string;
  providerKind: ProviderKind;
  model: string;
  accessProfile: AccessProfile;
  executionBoundary: ExecutionBoundary;
  replaceCredential: boolean;
}

export type CredentialAction = "none" | "reuse" | "replace";

export interface ManagedProviderConfigurationInput {
  profile: string;
  providerKind: ProviderKind;
  baseUrl: string;
  timeoutMs: number | null;
  credentialAction: CredentialAction;
}

export interface ApplyManagedModelConfigurationRequest {
  workspaceId: string;
  providers: ManagedProviderConfigurationInput[];
  models: ManagedModelConfiguration[];
  roles: Record<string, string>;
  accessProfile: AccessProfile;
  executionBoundary: ExecutionBoundary;
}

export interface ManagedFieldOverride {
  fieldId: string;
  value: unknown;
}

export interface CatalogRevision<T> {
  revision: number;
  value: T;
}

export interface CatalogEntry<T> {
  id: string;
  label: string;
  currentRevision: number;
  archived: boolean;
  revisions: CatalogRevision<T>[];
}

export type ManagedCredentialKind =
  "api_key" | "bearer_token" | "client_secret" | "generic_secret";

export interface ManagedCredentialMetadata {
  id: string;
  label: string;
  kind: ManagedCredentialKind;
  backend: "desktop" | "legacy_provider";
  createdAtMs: number;
}

export interface ManagedMcpCredentialHeader {
  scheme: string | null;
  credentialId: string;
}

export interface ManagedMcpOAuth {
  clientId: string;
  clientSecretCredentialId: string | null;
  callbackPort: number;
  scopes: string[];
}

export interface ManagedMcpResearchTool {
  tool: string;
  title: string | null;
  arguments: Record<string, unknown>;
}

export interface ManagedMcpServer {
  name: string;
  transport: "stdio" | "streamable_http";
  command: string | null;
  args: string[];
  workingDirectory: string | null;
  environmentCredentials: Record<string, string>;
  url: string | null;
  headers: Record<string, string>;
  credentialHeaders: Record<string, ManagedMcpCredentialHeader>;
  allowStateless: boolean;
  oauth: ManagedMcpOAuth | null;
  allowedTools: string[];
  researchTools: ManagedMcpResearchTool[];
  timeoutMs: number | null;
  maxOutputBytes: number | null;
}

export interface ManagedMcpToolDiagnostic {
  server: string;
  name: string;
  title: string | null;
  description: string | null;
}

export interface ManagedMcpDiagnostic {
  server: string;
  healthy: boolean;
  tools: ManagedMcpToolDiagnostic[];
}

export interface ManagedMcpOAuthStatus {
  server: string;
  configured: boolean;
  authenticated: boolean;
}

export interface ManagedMcpOAuthLogin {
  server: string;
  authorizationUrl: string;
  callbackUrl: string;
}

export interface ManagedReadinessCheck {
  name: string;
  status: "pass" | "fail" | "not_checked" | "not_applicable";
  detail: string;
}

export interface ManagedRuntimeDiagnostic {
  kind: "provider" | "model" | "search" | "telemetry";
  profile: string;
  ready: boolean;
  checks: ManagedReadinessCheck[];
  resultCount: number | null;
}

export interface ManagedSkillCatalogEntry {
  name: string;
  version: string;
  description: string;
  source: string;
  offlineCompatible: boolean;
}

export interface ManagedPackCatalogEntry {
  name: string;
  version: string;
  publisher: string;
  status: "enabled" | "disabled" | "uninstalled" | "unknown";
  manifestSha256: string;
  trusted: boolean;
}

export interface ManagedWorkflowCatalogEntry {
  name: string;
  version: string;
  status: "registered" | "revised";
  updatedAt: string;
  revisionHash: string;
}

export interface ManagedExtensionInventory {
  skills: ManagedSkillCatalogEntry[];
  packs: ManagedPackCatalogEntry[];
  workflows: ManagedWorkflowCatalogEntry[];
}

export interface ManagedProviderCatalogValue {
  profile: string;
  kind: ProviderKind;
  baseUrl: string;
  credentialId?: string | null;
  timeoutMs?: number | null;
}

export type ManagedModelCatalogValue = ManagedModelConfiguration;

export interface ManagedSearchProvider {
  profile: string;
  kind: "searxng" | "serp_api";
  endpoint: string;
  credentialId: string | null;
  authHeader: string | null;
  timeoutMs: number;
}

export interface ManagedTelemetryProfile {
  name: string;
  endpoint: string | null;
  protocol: "grpc" | "http_protobuf";
  timeoutMs: number;
  tracesEnabled: boolean;
  traceSampleRatioMillionths: number;
  metricsEnabled: boolean;
  metricExportIntervalMs: number;
  logsOtlp: boolean;
  logsStdoutJson: boolean;
  journalPayloads: "disabled" | "metadata" | "full";
  acknowledgeSensitiveContent: boolean;
  acknowledgeInsecureTransport: boolean;
  resourceAttributes: Record<string, string>;
}

export interface ManagedDefaultOverrides {
  accessProfile: AccessProfile | null;
  executionBoundary: ExecutionBoundary | null;
  terminalEnabled: boolean | null;
  fieldOverrides: ManagedFieldOverride[];
}

export interface ManagedGlobalConfiguration {
  revision: number;
  providers: CatalogEntry<ManagedProviderCatalogValue>[];
  models: CatalogEntry<ManagedModelCatalogValue>[];
  mcpServers: CatalogEntry<ManagedMcpServer>[];
  searchProviders: CatalogEntry<ManagedSearchProvider>[];
  telemetryProfiles: CatalogEntry<ManagedTelemetryProfile>[];
  credentials: ManagedCredentialMetadata[];
  defaults: {
    currentRevision: number;
    revisions: CatalogRevision<ManagedDefaultOverrides>[];
  };
}

export interface ManagedCatalogReference {
  resourceId: string;
  revision: number;
}

export interface ManagedSpaceConfiguration {
  acceptedGlobalRevision: number;
  catalogRevisions: Record<string, ManagedCatalogReference>;
  credentialOverrides: Record<string, string>;
  searchRoles: Record<string, string>;
  modelRoles: Record<string, string>;
  accessProfileOverride: AccessProfile | null;
  executionBoundaryOverride: ExecutionBoundary | null;
  terminalEnabledOverride: boolean | null;
  fieldOverrides: ManagedFieldOverride[];
  import: {
    relativePath: string;
    sha256: string;
    importedAtMs: number;
  } | null;
}

export interface ManagedEffectiveValue {
  fieldId: string;
  value: unknown;
  source: "built_in" | "global" | "space";
}

export interface ManagedSpaceConfigurationSnapshot {
  id: string;
  name: string;
  displayPath: string;
  archived: boolean;
  status:
    | "active"
    | "update_available"
    | "draining"
    | "starting"
    | "restarting"
    | "validation_failed"
    | "runtime_failed";
  statusMessage: string;
  pendingGlobalRevision: number | null;
  configuration: ManagedSpaceConfiguration;
  effectiveValues: ManagedEffectiveValue[];
  effectiveYaml: string;
}

export interface ManagedFieldDescriptor {
  id: string;
  section: string;
  title: string;
  description: string;
  scope: "global" | "space" | "both";
  risk: "low" | "medium" | "high";
  control: "toggle" | "number" | "text" | "string_list" | "select" | "json";
  advanced: boolean;
  defaultValue: unknown;
  minimum: number | null;
  maximum: number | null;
  options: string[];
}

export interface ManagedLockedInvariant {
  id: string;
  title: string;
  owner: "Desktop";
  explanation: string;
}

export interface ManagedSettingsSnapshot {
  globalConfiguration: ManagedGlobalConfiguration;
  spaces: ManagedSpaceConfigurationSnapshot[];
  fieldDescriptors: ManagedFieldDescriptor[];
  lockedInvariants: ManagedLockedInvariant[];
}

export interface ImportResourceProposal {
  kind: "provider" | "model" | "search" | "mcp" | "telemetry";
  sourceId: string;
  label: string;
  detail: string;
  conflict: boolean;
  existingResourceId: string | null;
}

export interface ImportConflictDecision {
  action: "rename" | "replace" | "skip";
  renamedSourceId: string | null;
}

export interface ImportCredentialSlot {
  slotId: string;
  label: string;
  consumers: string[];
}

export interface RepositoryConfigurationProposal {
  spaceId: string;
  relativePath: string;
  sha256: string;
  previousSha256: string | null;
  changedSinceImport: boolean;
  resources: ImportResourceProposal[];
  credentialSlots: ImportCredentialSlot[];
  fieldOverrides: string[];
  lockedFields: string[];
  warnings: string[];
}

export interface SaveGlobalDefaultsRequest {
  expectedRevision: number;
  accessProfile: AccessProfile | null;
  executionBoundary: ExecutionBoundary | null;
  terminalEnabled: boolean | null;
  fieldOverrides: ManagedFieldOverride[];
}

export interface SaveSpaceConfigurationRequest {
  expectedGlobalRevision: number;
  spaceId: string;
  accessProfileOverride: AccessProfile | null;
  executionBoundaryOverride: ExecutionBoundary | null;
  terminalEnabledOverride: boolean | null;
  fieldOverrides: ManagedFieldOverride[];
  selectedProviderResourceIds: string[];
  selectedModelResourceIds: string[];
  selectedMcpResourceIds: string[];
  selectedSearchResourceIds: string[];
  selectedTelemetryResourceId: string | null;
  searchRoles: Record<string, string>;
  modelRoles: Record<string, string>;
  credentialOverrides: Record<string, string>;
}

export type TerminalKind = "colossus_tui" | "shell";

export interface TerminalPlanContext {
  sessionId: string;
  planId: string;
}

export type TerminalSignal = "interrupt" | "terminate";

export interface TerminalContext {
  enabled: boolean;
  shellEnabled: boolean;
  tuiEnabled: boolean;
  contextGeneration: number;
  launchRequestId: number;
  workspaceId: string | null;
  workspaceName: string | null;
  requestedKind: TerminalKind | null;
  requestedPlanSessionId: string | null;
  requestedPlanId: string | null;
}

export type TerminalEvent =
  | {
      type: "output";
      sessionId: string;
      dataBase64: string;
    }
  | {
      type: "exited";
      sessionId: string;
      exitCode: number | null;
      signal: string | null;
    }
  | {
      type: "error";
      sessionId: string;
      code: string;
      message: string;
    };

export interface OpenTerminalResponse {
  sessionId: string;
}

export interface CommandViolation {
  field: string;
  description: string;
}

export interface CommandError {
  code: string;
  message: string;
  retryable: boolean;
  outcomeUnknown: boolean;
  violations: CommandViolation[];
}

export type RunMode = "execute" | "plan" | "research";
export type ResearchDepth = "quick" | "standard" | "deep";
export type ResearchSourceKind = "repo" | "web" | "mcp";

export type RunStatus =
  | "queued"
  | "running"
  | "waiting"
  | "cancelling"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted"
  | "outcome_unknown";

export type OutcomeCertainty = "known" | "unknown";

export type PlanStatus = "draft" | "approved" | "executed" | "discarded";

export type PlanExecutionStrategy =
  { type: "direct" } | { type: "goal"; maxIterations: number };

export type PlanRunAction =
  | {
      type: "revise";
      sourceRunId: string;
      expectedRevision: number;
    }
  | {
      type: "execute";
      sourceRunId: string;
      expectedRevision: number;
      strategy: PlanExecutionStrategy;
    };

export interface RunResult {
  output: string;
  planId?: string;
  planRevision?: number;
  planStatus?: PlanStatus;
  goalId?: string;
  profile: string;
  modelProfile: string;
  providerProfile: string;
  model: string;
  elapsedSeconds: number;
}

export interface RunFailure {
  reason: string;
  message: string;
  outcomeCertainty: OutcomeCertainty;
  recoverable?: boolean;
  httpStatus?: number | null;
  retryAfterMs?: number | null;
}

export interface RunCancellation {
  turn: number;
  message: string;
  planId?: string;
  planRevision?: number;
  planStatus?: PlanStatus;
  goalId?: string;
}

export type RunTerminal =
  | { type: "result"; result: RunResult }
  | { type: "failure"; failure: RunFailure }
  | { type: "cancellation"; cancellation: RunCancellation };

export interface Run {
  runId: string;
  sessionId: string;
  title: string;
  role: string;
  mode: RunMode;
  status: RunStatus;
  createdAt: string;
  updatedAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  lastSequence: number;
  pendingInteractionCount: number;
  terminal: RunTerminal | null;
  etag: string;
  selectedSkills: string[];
  archived: boolean;
}

export type ThreadDelegateStatus =
  "queued" | "running" | "completed" | "failed" | "cancelled" | "interrupted";

export type ThreadDelegateActivityState =
  "started" | "completed" | "cancelled" | "failed";

export interface ThreadDelegateActivity {
  callId: string;
  toolName: string;
  state: ThreadDelegateActivityState;
  summary: string;
  input?: string;
  preview?: string;
  startedAt: string;
  completedAt?: string;
}

export interface ThreadDelegateInspection {
  jobId: string;
  parentRunId: string;
  childSessionId: string;
  childRunId?: string;
  task: string;
  role: string;
  status: ThreadDelegateStatus;
  finalOutput: string;
  error: string;
  createdAt: string;
  updatedAt: string;
  startedAt?: string;
  completedAt?: string;
  activities: ThreadDelegateActivity[];
}

export interface SessionMapDelegate {
  jobId: string;
  parentRunId: string;
  childSessionId: string;
  childRunId?: string;
  task: string;
  role: string;
  status: ThreadDelegateStatus;
  finalOutput: string;
  error: string;
  createdAt: string;
  updatedAt: string;
  startedAt?: string;
  completedAt?: string;
}

export interface SessionMapTask {
  id: string;
  title: string;
  description: string;
  status: "pending" | "in_progress" | "completed" | "blocked" | "cancelled";
  createdAt: string;
  updatedAt: string;
}

export interface SessionMapPlan {
  id: string;
  prompt: string;
  status: "draft" | "approved" | "executed" | "discarded";
  revision: number;
  content: string;
  stepCount: number;
  executedRunId?: string;
  createdAt: string;
  updatedAt: string;
}

export interface SessionMapGoal {
  id: string;
  objective: string;
  sourcePlanId?: string;
  status: "active" | "complete" | "blocked";
  summary: string;
  blockedReason: string;
  iterationBudget: number;
  iterationsCompleted: number;
  createdAt: string;
  updatedAt: string;
}

export interface SessionMapDecision {
  id: string;
  goalId?: string;
  planId?: string;
  source: "user" | "agent";
  status: "active" | "archived" | "superseded";
  priority: "critical" | "high" | "normal";
  title: string;
  decision: string;
  intent: string;
  appliesWhen: string;
  rationale: string;
  createdAt: string;
  updatedAt: string;
}

export interface SessionMapMemory {
  id: string;
  scope: "global" | "repository" | "session";
  kind: string;
  confidence: number;
  source: string;
  status: "active" | "archived" | "superseded";
  text: string;
  rationale: string;
  createdAt: string;
  updatedAt: string;
  expiresAt?: string;
  supersededBy?: string;
}

export interface SessionMapResearchRun {
  id: string;
  question: string;
  depth: ResearchDepth;
  sourceKinds: ResearchSourceKind[];
  status: "running" | "completed" | "failed" | "interrupted";
  queryCount: number;
  sourceCount: number;
  limitationCount: number;
  report: string;
  error: string;
  createdAt: string;
  updatedAt: string;
  completedAt?: string;
}

export interface SessionMapResearchSource {
  id: string;
  runId: string;
  label: string;
  kind: ResearchSourceKind;
  title: string;
  uri: string;
  query: string;
  createdAt: string;
}

export interface SessionMap {
  sessionId: string;
  delegates: SessionMapDelegate[];
  goals: SessionMapGoal[];
  tasks: SessionMapTask[];
  plans: SessionMapPlan[];
  decisions: SessionMapDecision[];
  memories: SessionMapMemory[];
  researchRuns: SessionMapResearchRun[];
  researchSources: SessionMapResearchSource[];
}

export type SessionMapResource =
  | { family: "delegates"; value: SessionMapDelegate }
  | { family: "goals"; value: SessionMapGoal }
  | { family: "tasks"; value: SessionMapTask }
  | { family: "plans"; value: SessionMapPlan }
  | { family: "decisions"; value: SessionMapDecision }
  | { family: "memories"; value: SessionMapMemory }
  | { family: "research"; value: SessionMapResearchRun }
  | { family: "sources"; value: SessionMapResearchSource };

export interface PromptChoice {
  choiceId: string;
  label: string;
}

export interface UserPromptContent {
  type: "user_prompt";
  question: string;
  choices: PromptChoice[];
  allowFreeForm: boolean;
}

export type ApprovalRisk = "low" | "medium" | "high";

export interface ApprovalContent {
  type: "approval";
  reason: string;
  action: string;
  resource: string;
  risk: ApprovalRisk | null;
  requestHash: string;
}

export type InteractionContent = UserPromptContent | ApprovalContent;
export type InteractionKind = "user_prompt" | "approval";
export type InteractionStatus =
  "pending" | "answered" | "expired" | "cancelled";

export interface Interaction {
  interactionId: string;
  runId: string;
  kind: InteractionKind;
  status: InteractionStatus;
  createdAt: string;
  expiresAt: string;
  respondableByCaller: boolean;
  etag: string;
  content: InteractionContent;
}

export type ToolActivityState =
  | "requested"
  | "waiting_approval"
  | "started"
  | "completed"
  | "cancelled"
  | "failed"
  | "outcome_unknown";

export interface ToolActivity {
  callId: string;
  toolName: string;
  state: ToolActivityState;
  summary: string;
  input?: string | null;
  preview?: string | null;
}

export interface TokenUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cachedInputTokens: number | null;
  reasoningTokens: number | null;
}

export type MessageRole = "user" | "assistant" | "tool" | "system";

export interface ArtifactReference {
  artifactId: string;
  fileName: string;
  mediaType: string;
  sizeBytes: number;
  sha256: string;
  purpose: "run_input" | "run_output" | "workflow" | "extension" | "archive";
  state: "uploading" | "quarantined" | "available" | "rejected" | "expired";
  createdAt: string;
}

export type MessageContentPart =
  | { type: "text"; text: string }
  | { type: "artifact"; artifact: ArtifactReference };

export interface SessionMessage {
  sessionId: string;
  runId: string;
  sequence: number;
  role: MessageRole;
  content: MessageContentPart[];
  createdAt: string;
}

export type RunUpdateKind =
  | { type: "state"; status: RunStatus }
  | { type: "output_delta"; delta: string }
  | { type: "reasoning_summary"; summary: string }
  | { type: "tool_activity"; activity: ToolActivity }
  | { type: "usage"; usage: TokenUsage }
  | { type: "interaction"; interaction: Interaction }
  | { type: "message"; message: SessionMessage }
  | { type: "notice"; reason: string; message: string }
  | { type: "result"; result: RunResult }
  | { type: "failure"; status: RunStatus; failure: RunFailure }
  | { type: "cancellation"; cancellation: RunCancellation };

export interface RunUpdate {
  runId: string;
  sequence: number;
  createdAt: string;
  update: RunUpdateKind;
}

export type WatchEvent =
  | { type: "update"; update: RunUpdate }
  | { type: "complete"; runId: string }
  | { type: "error"; error: CommandError };

export interface RunDetails {
  run: Run;
  pendingInteractions: Interaction[];
}

export interface RunPage {
  runs: Run[];
  nextPageToken: string;
}

/** Create-run API sentinel that selects the server's configured turn bound. */
export const USE_CONFIGURED_MAX_TURNS = 0;

export interface CreateRunRequest {
  prompt: string;
  artifactIds?: string[];
  sessionId?: string;
  role: string;
  mode: RunMode;
  researchDepth?: ResearchDepth;
  researchSources?: ResearchSourceKind[];
  planAction?: PlanRunAction;
  branch?: {
    sourceRunId: string;
  };
  /** Positive override, or USE_CONFIGURED_MAX_TURNS for the server default. */
  maxTurns: number;
  idempotencyKey: string;
}

export interface Aside {
  parentSessionId: string;
  sourceRunId: string;
  createdAt: string;
  closed: boolean;
  run: Run;
}

export interface ArtifactContent {
  artifact: ArtifactReference;
  text: string;
}

export interface GetRunRequest {
  runId: string;
}

export interface ListRunsRequest {
  sessionId?: string;
  pageToken: string;
  includeArchived?: boolean;
}

export interface WatchRunRequest {
  runId: string;
  afterSequence: number;
}

export interface CancelRunRequest {
  runId: string;
  idempotencyKey: string;
}

export interface ThreadLifecycleRequest {
  runId: string;
  idempotencyKey: string;
}

export interface ThreadLifecycle {
  sessionId: string;
  archived: boolean;
}

export type InteractionAnswer =
  | { type: "prompt_choice"; choiceId: string; label: string }
  | { type: "prompt_text"; text: string }
  | { type: "approval"; approved: boolean; requestHash: string };

export interface RespondInteractionRequest {
  runId: string;
  interactionId: string;
  etag: string;
  idempotencyKey: string;
  response: InteractionAnswer;
}

export function isTerminalStatus(status: RunStatus): boolean {
  return (
    status === "completed" ||
    status === "failed" ||
    status === "cancelled" ||
    status === "interrupted" ||
    status === "outcome_unknown"
  );
}
