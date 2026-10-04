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
  watching: boolean;
  status: string;
  error: string;
  mode: "plan" | "execute";
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
    (input.mode === "plan" || input.mode === "execute") &&
    Object.keys(input).length === 3
  )
    return { type: "send", text: input.text, mode: input.mode };
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
