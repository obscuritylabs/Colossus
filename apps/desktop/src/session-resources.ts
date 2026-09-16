import type { ResearchSource } from "./components/ResearchSourcesPanel";
import { researchSources } from "./components/ResearchSourcesPanel";
import type { RunView } from "./state";
import type { PlanStatus, RunTerminal } from "./types";
import { isTerminalStatus } from "./types";

export interface SessionPlanReference {
  planId: string;
  revision: number;
  status: PlanStatus | null;
  sourceRunId: string;
  sourceRunTitle: string;
  runIndex: number;
  createdAt: string;
  cancelled: boolean;
  output: string;
  continuationPending?: boolean;
}

export interface SessionResearchSource extends ResearchSource {
  sourceRunId: string;
  sourceRunTitle: string;
}

export interface AutomaticPlanSelection {
  plan: SessionPlanReference | null;
  observedKeys: ReadonlySet<string>;
}

/** Whether released metadata supports displaying draft continuation controls. */
export function canContinuePlan(
  plan: {
    status?: PlanStatus | null;
    revision?: number;
    continuationPending?: boolean;
  },
  continuationAvailable: boolean,
): boolean {
  return (
    continuationAvailable &&
    !plan.continuationPending &&
    plan.status === "draft" &&
    (plan.revision ?? 0) > 0
  );
}

/** Older run cards remain readable but cannot act on a superseded draft. */
export function canContinuePlanFromRun(
  sourceRunId: string,
  plans: readonly SessionPlanReference[],
  continuationAvailable: boolean,
): boolean {
  return plans.some(
    (plan) =>
      plan.sourceRunId === sourceRunId &&
      canContinuePlan(plan, continuationAvailable),
  );
}

function terminalPlan(
  terminal: RunTerminal | null,
): Pick<
  SessionPlanReference,
  "planId" | "revision" | "status" | "cancelled"
> | null {
  if (terminal?.type === "result" && terminal.result.planId !== undefined) {
    return {
      planId: terminal.result.planId,
      revision: terminal.result.planRevision ?? 0,
      status: terminal.result.planStatus ?? null,
      cancelled: false,
    };
  }
  if (
    terminal?.type === "cancellation" &&
    terminal.cancellation.planId !== undefined
  ) {
    return {
      planId: terminal.cancellation.planId,
      revision: terminal.cancellation.planRevision ?? 0,
      status: terminal.cancellation.planStatus ?? null,
      cancelled: true,
    };
  }
  return null;
}

export function selectSessionPlans(
  views: readonly RunView[],
): readonly SessionPlanReference[] {
  const plans = new Map<string, SessionPlanReference>();
  const pendingRevisions = new Map<string, number>();
  const planningResponses = new Map<
    string,
    { revision: number; output: string }
  >();
  for (const [runIndex, view] of views.entries()) {
    const reference = terminalPlan(view.run.terminal);
    const continuation = view.localPlanContinuation;
    if (
      continuation !== undefined &&
      (!isTerminalStatus(view.run.status) || reference === null)
    ) {
      pendingRevisions.set(
        continuation.planId,
        Math.max(
          pendingRevisions.get(continuation.planId) ?? 0,
          continuation.revision,
        ),
      );
    }
    if (reference === null) {
      continue;
    }
    const candidate = {
      ...reference,
      sourceRunId: view.run.runId,
      sourceRunTitle: view.run.title,
      runIndex: runIndex + 1,
      createdAt: view.run.updatedAt,
      output: view.output,
    };
    const current = plans.get(reference.planId);
    if (current === undefined || candidate.revision >= current.revision) {
      plans.set(reference.planId, candidate);
    }
    if (view.run.mode === "plan") {
      const response = planningResponses.get(reference.planId);
      if (response === undefined || reference.revision >= response.revision) {
        planningResponses.set(reference.planId, {
          revision: reference.revision,
          output: view.output,
        });
      }
    }
  }
  return [...plans.values()]
    .map((plan) => ({
      ...plan,
      output: planningResponses.get(plan.planId)?.output ?? "",
      continuationPending:
        (pendingRevisions.get(plan.planId) ?? -1) >= plan.revision,
    }))
    .sort(
      (left, right) =>
        right.createdAt.localeCompare(left.createdAt) ||
        left.planId.localeCompare(right.planId),
    );
}

function automaticPlanKey(
  sessionId: string,
  plan: SessionPlanReference,
): string {
  return `${sessionId}:${plan.planId}:${plan.revision}`;
}

export function selectPlanForAutomaticDetails(
  sessionId: string,
  plans: readonly SessionPlanReference[],
  observedKeys: ReadonlySet<string>,
): AutomaticPlanSelection {
  const eligiblePlans = plans.filter(
    (plan) => !plan.cancelled && canContinuePlan(plan, true),
  );
  const plan =
    eligiblePlans.find(
      (candidate) => !observedKeys.has(automaticPlanKey(sessionId, candidate)),
    ) ?? null;
  return {
    plan,
    observedKeys: new Set([
      ...observedKeys,
      ...eligiblePlans.map((candidate) =>
        automaticPlanKey(sessionId, candidate),
      ),
    ]),
  };
}

export function selectSessionSources(
  views: readonly RunView[],
): readonly SessionResearchSource[] {
  const sources = new Map<string, SessionResearchSource>();
  for (const view of views) {
    for (const source of researchSources(view.output)) {
      const key = `${source.label}:${source.uri}`;
      if (!sources.has(key)) {
        sources.set(key, {
          ...source,
          sourceRunId: view.run.runId,
          sourceRunTitle: view.run.title,
        });
      }
    }
  }
  return [...sources.values()];
}

export function sessionActionCount(views: readonly RunView[]): number {
  const calls = new Set<string>();
  for (const view of views) {
    for (const update of view.updates) {
      if (update.update.type === "tool_activity") {
        calls.add(`${view.run.runId}:${update.update.activity.callId}`);
      }
    }
  }
  return calls.size;
}
