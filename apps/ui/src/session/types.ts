export type PlanStatus = "draft" | "approved" | "executed" | "discarded";
export type ResearchDepth = "quick" | "standard" | "deep";
export type ResearchSourceKind = "repo" | "web" | "mcp";
export type ThreadDelegateStatus =
  "queued" | "running" | "completed" | "failed" | "cancelled" | "interrupted";
export type RunTerminal =
  | {
      type: "result";
      result: {
        output: string;
        planId?: string;
        planRevision?: number;
        planStatus?: PlanStatus;
      };
    }
  | {
      type: "cancellation";
      cancellation: {
        planId?: string;
        planRevision?: number;
        planStatus?: PlanStatus;
      };
    }
  | { type: "failure"; failure: unknown };
/** Released data only. Hosts adapt their run state without sharing transport or caches. */
export interface RunView {
  run: {
    runId: string;
    sessionId: string;
    title: string;
    status: string;
    mode: string;
    createdAt: string;
    updatedAt: string;
    terminal: RunTerminal | null;
  };
  output: string;
  localPlanContinuation?: { planId: string; revision: number };
}
export interface ArtifactViewItem {
  id: string;
  fileName: string;
  mediaType: string;
  sizeLabel: string;
  stateLabel: string;
  createdLabel: string;
}
export type AgentWorkState =
  | "coordinating"
  | "working"
  | "reviewing"
  | "waiting"
  | "completed"
  | "failed"
  | "cancelled"
  | "idle";
export interface AgentParticipant {
  id: string;
  name: string;
  role: string;
  state: AgentWorkState;
  kind: "primary" | "delegate";
}
export function isTerminalStatus(status: string): boolean {
  return [
    "completed",
    "failed",
    "cancelled",
    "interrupted",
    "outcome_unknown",
  ].includes(status);
}
export function shortDateLabel(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isFinite(date.getTime())
    ? date.toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      })
    : "Recent";
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
  sessionId: string;
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

export interface SessionMapContextSnapshot {
  id: string;
  sourceStartSequence: number;
  sourceEndSequence: number;
  summary: string;
  pinnedFacts: string[];
  openTasks: string[];
  filesTouched: string[];
  notableToolResults: string[];
  strategy: string;
  createdAt: string;
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
  contextSnapshots: SessionMapContextSnapshot[];
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
  | { family: "snapshots"; value: SessionMapContextSnapshot }
  | { family: "research"; value: SessionMapResearchRun }
  | { family: "sources"; value: SessionMapResearchSource };
