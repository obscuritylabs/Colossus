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
  kind: "openai_responses" | "openai_compatible" | null;
  model: string;
}

export type DesktopReleaseChannel =
  "development" | "stable" | "developer_preview" | "validation_only";

export interface DesktopStatus {
  releaseChannel: DesktopReleaseChannel;
  connection: ConnectionStatus;
  targets: RuntimeTarget[];
  selectedTargetId: string | null;
  managedState: ManagedRuntimeState;
  workspace: WorkspaceSummary | null;
  provider: ProviderSummary;
  accessProfile: "minimal" | "development";
  terminalEnabled: boolean;
}

export interface ConfigureManagedRuntimeRequest {
  workspaceId: string;
  providerKind: "openai_responses" | "openai_compatible";
  model: string;
  accessProfile: "minimal" | "development";
  replaceCredential: boolean;
}

export type TerminalKind = "colossus_tui";

export type TerminalSignal = "interrupt" | "terminate";

export interface TerminalContext {
  enabled: boolean;
  contextGeneration: number;
  launchRequestId: number;
  workspaceId: string | null;
  workspaceName: string | null;
  requestedKind: TerminalKind | null;
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

export interface RunResult {
  output: string;
  profile: string;
  model: string;
  elapsedSeconds: number;
}

export interface RunFailure {
  reason: string;
  message: string;
  outcomeCertainty: OutcomeCertainty;
}

export interface RunCancellation {
  turn: number;
  message: string;
}

export type RunTerminal =
  | { type: "result"; result: RunResult }
  | { type: "failure"; failure: RunFailure }
  | { type: "cancellation"; cancellation: RunCancellation };

export interface Run {
  runId: string;
  sessionId: string;
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

export interface CreateRunRequest {
  prompt: string;
  sessionId?: string;
  role: string;
  mode: RunMode;
  maxTurns: number;
  idempotencyKey: string;
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
