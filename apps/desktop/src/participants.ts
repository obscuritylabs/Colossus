import type { AgentParticipant, AgentWorkState } from "./components/AgentFlow";
import { agentRoleLabel, safeDisplayLabel } from "./presenters";
import type { RunView } from "./state";
import type { RunStatus, ToolActivity } from "./types";

const MAX_DELEGATED_PARTICIPANTS = 8;
const MAX_SESSION_DELEGATED_PARTICIPANTS = 32;
const SUBAGENT_LIFECYCLE_TOOL = "agent.subagent_update";
const SUBAGENT_TOOLS = new Set([
  "agent.delegate",
  "agent.result",
  SUBAGENT_LIFECYCLE_TOOL,
]);
const SUBAGENT_STATUSES = new Set([
  "queued",
  "running",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
]);

interface ReleasedSubagent {
  id: string;
  parentRunId: string | null;
  childSessionId: string | null;
  childRunId: string | null;
  role: string;
  task: string;
  status: string;
  finalOutput: string;
  error: string;
  createdAt: string | null;
  updatedAt: string | null;
  startedAt: string | null;
  completedAt: string | null;
}

function primaryState(status: RunStatus): AgentWorkState {
  if (status === "running" || status === "queued") {
    return "working";
  }
  if (status === "waiting" || status === "cancelling") {
    return "waiting";
  }
  if (status === "completed") {
    return "completed";
  }
  if (status === "failed" || status === "outcome_unknown") {
    return "failed";
  }
  if (status === "cancelled" || status === "interrupted") {
    return "cancelled";
  }
  return "idle";
}

function delegatedState(status: string): AgentWorkState {
  switch (status) {
    case "running":
      return "working";
    case "queued":
      return "waiting";
    case "completed":
      return "completed";
    case "failed":
    case "interrupted":
      return "failed";
    case "cancelled":
      return "cancelled";
    default:
      return "idle";
  }
}

function stringField(
  record: Record<string, unknown>,
  field: string,
  maxLength: number,
): string | null {
  const value = record[field];
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (trimmed === "" || trimmed.length > maxLength) {
    return null;
  }
  return trimmed;
}

function releasedSubagent(activity: ToolActivity): ReleasedSubagent | null {
  if (
    !SUBAGENT_TOOLS.has(activity.toolName) ||
    (activity.toolName !== SUBAGENT_LIFECYCLE_TOOL &&
      activity.state !== "completed") ||
    activity.preview == null
  ) {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(activity.preview);
  } catch {
    return null;
  }
  if (parsed == null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }

  let record = parsed as Record<string, unknown>;
  if (activity.toolName === SUBAGENT_LIFECYCLE_TOOL) {
    const job = record.job;
    if (
      record.kind !== "subagent.lifecycle.v1" ||
      job == null ||
      typeof job !== "object" ||
      Array.isArray(job)
    ) {
      return null;
    }
    record = job as Record<string, unknown>;
  }
  const id = stringField(record, "id", 128);
  const status = stringField(record, "status", 32);
  if (id == null || status == null || !SUBAGENT_STATUSES.has(status)) {
    return null;
  }

  const parentRunId = stringField(record, "parent_run_id", 128);
  if (activity.toolName === SUBAGENT_LIFECYCLE_TOOL && parentRunId == null) {
    return null;
  }

  return {
    id,
    parentRunId,
    childSessionId: stringField(record, "child_session_id", 128),
    childRunId: stringField(record, "child_run_id", 128),
    role: stringField(record, "role", 128) ?? "subagent_default",
    task: stringField(record, "task", 512) ?? "",
    status,
    finalOutput: stringField(record, "final_output", 64 * 1024) ?? "",
    error: stringField(record, "error", 64 * 1024) ?? "",
    createdAt: stringField(record, "created_at", 64),
    updatedAt: stringField(record, "updated_at", 64),
    startedAt: stringField(record, "started_at", 64),
    completedAt: stringField(record, "completed_at", 64),
  };
}

/**
 * Derive participants only from the selected run and its bounded, released
 * tool previews. Private child-run state is never queried or guessed.
 */
export function selectAgentParticipants(
  view: RunView | undefined,
): readonly AgentParticipant[] {
  if (view === undefined) {
    return [];
  }

  const delegated = new Map<string, ReleasedSubagent>();
  for (const update of view.updates) {
    if (update.update.type !== "tool_activity") {
      continue;
    }
    const participant = releasedSubagent(update.update.activity);
    if (
      participant == null ||
      (participant.parentRunId != null &&
        participant.parentRunId !== view.run.runId)
    ) {
      continue;
    }
    if (!delegated.has(participant.id)) {
      if (delegated.size >= MAX_DELEGATED_PARTICIPANTS) {
        continue;
      }
      delegated.set(participant.id, participant);
      continue;
    }
    delegated.set(participant.id, participant);
  }

  const primary: AgentParticipant = {
    id: view.run.runId,
    name: agentRoleLabel(view.run.role),
    role: "Primary run",
    state: primaryState(view.run.status),
    icon: "lead",
    kind: "primary",
  };
  const children = [...delegated.values()].map((participant, index) => ({
    id: participant.id,
    name:
      delegated.size === 1 ? "Delegated agent" : `Delegated agent ${index + 1}`,
    role:
      participant.task === ""
        ? agentRoleLabel(participant.role)
        : safeDisplayLabel(
            participant.task,
            agentRoleLabel(participant.role),
            72,
          ),
    state: delegatedState(participant.status),
    icon: "builder" as const,
    kind: "delegate" as const,
    parentRunId: participant.parentRunId ?? view.run.runId,
    ...(participant.childSessionId === null
      ? {}
      : { childSessionId: participant.childSessionId }),
    ...(participant.childRunId === null
      ? {}
      : { childRunId: participant.childRunId }),
    modelRole: participant.role,
    task: participant.task,
    finalOutput: participant.finalOutput,
    error: participant.error,
    ...(participant.createdAt === null
      ? {}
      : { createdAt: participant.createdAt }),
    ...(participant.updatedAt === null
      ? {}
      : { updatedAt: participant.updatedAt }),
    ...(participant.startedAt === null
      ? {}
      : { startedAt: participant.startedAt }),
    ...(participant.completedAt === null
      ? {}
      : { completedAt: participant.completedAt }),
  }));

  return [primary, ...children];
}

/**
 * Builds the session-level participant projection shown by Desktop. Delegates
 * remain owned by their originating run, while the primary represents the
 * canonical session that contains those runs.
 */
export function selectSessionParticipants(
  views: readonly RunView[],
): readonly AgentParticipant[] {
  const first = views[0];
  const latest = views.at(-1);
  if (first === undefined || latest === undefined) {
    return [];
  }

  const delegates: AgentParticipant[] = [];
  for (const [runIndex, view] of views.entries()) {
    for (const participant of selectAgentParticipants(view).slice(1)) {
      if (delegates.length >= MAX_SESSION_DELEGATED_PARTICIPANTS) {
        break;
      }
      delegates.push({
        ...participant,
        parentRunIndex: runIndex + 1,
        parentRunTitle: safeDisplayLabel(
          view.run.title,
          `Run ${runIndex + 1}`,
          96,
        ),
      });
    }
  }

  const namedDelegates = delegates.map((participant, index) => ({
    ...participant,
    name:
      delegates.length === 1
        ? "Delegated agent"
        : `Delegated agent ${index + 1}`,
  }));
  return [
    {
      id: first.run.sessionId,
      name: agentRoleLabel(first.run.role),
      role: "Primary session",
      state: primaryState(latest.run.status),
      icon: "lead",
      kind: "primary",
    },
    ...namedDelegates,
  ];
}
