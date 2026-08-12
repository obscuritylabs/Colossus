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
  managedState: ManagedRuntimeState;
  workspace: WorkspaceSummary | null;
  provider: ProviderSummary;
  codexAuth: CodexAuthStatus;
  managedModelConfiguration: ManagedConfiguration;
  accessProfile: "minimal" | "development";
  approvalMode: ApprovalMode;
  terminalEnabled: boolean;
  additionalCaBundle: CaBundleStatus;
  capabilities: DesktopCapabilities;
}

export type ApprovalMode = "deny" | "ask" | "risk_auto" | "full_access";

export interface CaBundleStatus {
  configured: boolean;
  certificateCount: number;
  fingerprintsSha256: string[];
}

export interface DesktopCapabilities {
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
  accessProfile: "minimal" | "development";
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
  accessProfile: "minimal" | "development";
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

export type RunMode = "execute" | "plan";

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
}

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
  | "failed"
  | "outcome_unknown";

export interface ToolActivity {
  callId: string;
  toolName: string;
  state: ToolActivityState;
  summary: string;
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
  planAction?: PlanRunAction;
  /** Positive override, or USE_CONFIGURED_MAX_TURNS for the server default. */
  maxTurns: number;
  idempotencyKey: string;
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
}

export interface WatchRunRequest {
  runId: string;
  afterSequence: number;
}

export interface CancelRunRequest {
  runId: string;
  idempotencyKey: string;
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
