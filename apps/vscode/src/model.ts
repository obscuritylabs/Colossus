export interface MessageView {
  id: string;
  runId?: string;
  role: "user" | "assistant" | "notice";
  text: string;
  summary?: boolean;
}
export interface ToolView {
  id: string;
  runId?: string;
  name: string;
  state: string;
  summary: string;
  sequence?: string;
  startedAt?: string;
  updatedAt?: string;
  input?: string;
  preview?: string;
  history?: { state: string; summary: string; at: string }[];
}
export interface SessionView {
  id: string;
  title: string;
  status?: string;
  updatedAt?: string;
}
export interface RunView {
  id: string;
  sessionId: string;
  title: string;
  role: string;
  mode: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  startedAt: string;
  finishedAt: string;
  sequence: string;
  pendingInteractions: number;
}
export interface PlanView {
  id: string;
  sourceRunId: string;
  sessionId: string;
  title: string;
  revision: string;
  status: string;
  goalId: string;
}
export interface CapabilityView {
  name: string;
  enabled: boolean;
  detail: string;
}
export interface ActivityView {
  id: string;
  title: string;
  summary: string;
  kind: string;
  lane: string;
  status: string;
  startedAt: string;
  completedAt: string;
  result: string;
}
export interface InspectionView {
  agentInboxesAvailable?: boolean;
  run: RunView;
  plan?: PlanView | undefined;
  output: string;
  model: string;
  provider: string;
  activities: ActivityView[];
  activityState: string;
  activityHasMore: boolean;
  observedAt: string;
}
export interface InteractionView {
  id: string;
  title: string;
  kind: "approval" | "question";
  respondable: boolean;
}
export interface ContextView {
  label: string;
  text: string;
}
export interface WorkView {
  connected: boolean;
  connecting: boolean;
  workspace: string;
  version: string;
  sessionId: string;
  sessions: SessionView[];
  runs: RunView[];
  plans: PlanView[];
  capabilities: CapabilityView[];
  historyHasMore: boolean;
  historyLoading: boolean;
  inspection?: InspectionView | undefined;
  inspectionLoading: boolean;
  messages: MessageView[];
  tools: ToolView[];
  interactions: InteractionView[];
  context: ContextView[];
  busy: boolean;
  reconnectable?: boolean;
  watching: boolean;
  status: string;
  error: string;
  mode: WorkMode;
}
export type ViewAction =
  | {
      type:
        | "ready"
        | "connect"
        | "disconnect"
        | "newSession"
        | "stop"
        | "resume"
        | "addSelection"
        | "addFile"
        | "clearContext"
        | "reviewChanges"
        | "refreshSessions"
        | "loadMoreSessions"
        | "openWorkspace"
        | "openWork";
    }
  | {
      type: "openSettings";
    }
  | { type: "send"; text: string; mode: "plan" | "execute" }
  | ({ type: "send"; text: string; mode: "research" } & ResearchOptions)
  | { type: "selectSession" | "inspectRun" | "inspectPlan"; id: string }
  | { type: "respond"; id: string };

export function parseAction(value: unknown): ViewAction | undefined {
  if (value === null || typeof value !== "object" || !("type" in value))
    return undefined;
  const input = value as Record<string, unknown>;
  const simple = [
    "ready",
    "connect",
    "disconnect",
    "newSession",
    "stop",
    "resume",
    "addSelection",
    "addFile",
    "clearContext",
    "reviewChanges",
    "refreshSessions",
    "openSettings",
    "loadMoreSessions",
    "openWorkspace",
    "openWork",
  ];
  if (
    typeof input.type === "string" &&
    simple.includes(input.type) &&
    Object.keys(input).length === 1
  )
    return input as ViewAction;
  if (
    input.type === "send" &&
    typeof input.text === "string" &&
    input.text.trim().length > 0 &&
    new TextEncoder().encode(input.text).length <= 64 * 1024 &&
    isWorkMode(input.mode)
  ) {
    if (
      (input.mode === "plan" || input.mode === "execute") &&
      Object.keys(input).length === 3
    )
      return { type: "send", text: input.text, mode: input.mode };
    if (
      input.mode === "research" &&
      Object.keys(input).length === 5 &&
      isResearchOptions(input)
    )
      return {
        type: "send",
        text: input.text,
        mode: "research",
        researchDepth: input.researchDepth,
        researchSources: [...input.researchSources],
      };
  }
  if (
    ["selectSession", "respond", "inspectRun", "inspectPlan"].includes(
      String(input.type),
    ) &&
    typeof input.id === "string" &&
    /^[A-Za-z0-9:_-]{1,256}$/u.test(input.id) &&
    Object.keys(input).length === 2
  )
    return { type: input.type, id: input.id } as ViewAction;
  return undefined;
}

export function initialView(workspace: string): WorkView {
  return {
    connected: false,
    connecting: false,
    workspace,
    version: "",
    sessionId: "",
    sessions: [],
    runs: [],
    plans: [],
    capabilities: [],
    historyHasMore: false,
    historyLoading: false,
    inspectionLoading: false,
    messages: [],
    tools: [],
    interactions: [],
    context: [],
    busy: false,
    watching: false,
    status: "Connect a Colossus worker to start.",
    error: "",
    mode: "plan",
  };
}
export type WorkMode = "plan" | "execute" | "research";
export type ResearchDepth = "quick" | "standard" | "deep";
export type ResearchSource = "repo" | "web" | "mcp";
export interface ResearchOptions {
  researchDepth: ResearchDepth;
  researchSources: ResearchSource[];
}

export function isWorkMode(value: unknown): value is WorkMode {
  return value === "plan" || value === "execute" || value === "research";
}

export function isResearchDepth(value: unknown): value is ResearchDepth {
  return value === "quick" || value === "standard" || value === "deep";
}

export function isResearchSources(value: unknown): value is ResearchSource[] {
  return (
    Array.isArray(value) &&
    value.length <= 3 &&
    [...value].every(
      (source) => source === "repo" || source === "web" || source === "mcp",
    ) &&
    new Set(value).size === value.length
  );
}

export function isResearchOptions(value: unknown): value is ResearchOptions {
  if (!value || typeof value !== "object") return false;
  const input = value as Record<string, unknown>;
  return (
    isResearchDepth(input.researchDepth) &&
    isResearchSources(input.researchSources) &&
    input.researchSources.length > 0
  );
}

export function supportsResearch(
  capabilities: readonly CapabilityView[],
): boolean {
  return capabilities.some(
    (capability) => capability.name === "research.create" && capability.enabled,
  );
}

export type InboxInspectionRequest = {
  type: "agentInbox";
  requestId: string;
  participantId: string | null;
  afterSequence: number;
};
export function parseInboxInspectionRequest(
  value: unknown,
): InboxInspectionRequest | undefined {
  if (!value || typeof value !== "object") return;
  const input = value as Record<string, unknown>;
  if (
    Object.keys(input).length !== 4 ||
    input.type !== "agentInbox" ||
    typeof input.requestId !== "string" ||
    !/^[A-Za-z0-9-]{1,64}$/u.test(input.requestId) ||
    !(
      input.participantId === null ||
      (typeof input.participantId === "string" &&
        /^[A-Za-z0-9._:-]{1,128}$/u.test(input.participantId))
    ) ||
    typeof input.afterSequence !== "number" ||
    !Number.isSafeInteger(input.afterSequence) ||
    input.afterSequence < 0 ||
    input.afterSequence > 4096
  )
    return;
  return input as InboxInspectionRequest;
}
