import type { ComposerModelSelection } from "./composer-model";
import type { ChatState } from "./state";
import { MAX_CACHED_IDEMPOTENCY_ATTEMPTS, withBoundedEntry } from "./state";
import type { CommandError, Run } from "./types";
import { isTerminalStatus } from "./types";

/** A new continuation is only offered against the latest confirmed failure. */
export function selectedProviderRetryRun(chat: ChatState): Run | null {
  if (chat.activeRunId === null) return null;
  const view = chat.views.get(chat.activeRunId);
  const run = view?.run;
  if (
    view === undefined ||
    run === undefined ||
    view.streamState === "error" ||
    run.status !== "failed" ||
    run.mode !== "execute" ||
    run.archived ||
    run.continuable === false ||
    run.pendingInteractionCount > 0 ||
    view.pendingInteractions.length > 0 ||
    run.terminal?.type !== "failure"
  ) {
    return null;
  }

  const { failure } = run.terminal;
  if (
    failure.reason !== "provider.temporarily_unavailable" ||
    failure.outcomeCertainty !== "known" ||
    failure.recoverable !== true ||
    (failure.httpStatus !== 500 &&
      failure.httpStatus !== 502 &&
      failure.httpStatus !== 503 &&
      failure.httpStatus !== 504)
  ) {
    return null;
  }

  const blocksRetry = (other: Run) =>
    other.sessionId === run.sessionId &&
    other.runId !== run.runId &&
    (!isTerminalStatus(other.status) ||
      other.createdAt > run.createdAt ||
      (other.createdAt === run.createdAt && other.runId > run.runId));
  if (chat.recentRuns.some(blocksRetry)) return null;
  for (const { run: other } of chat.views.values()) {
    if (blocksRetry(other)) return null;
  }
  return run;
}

interface ProviderRetrySelection {
  readonly modelSelection: Readonly<ComposerModelSelection>;
  readonly pluginSkillIds: readonly string[];
}

interface ProviderRetryRequest extends ProviderRetrySelection {
  readonly role: string;
  readonly sessionId: string;
  readonly idempotencyKey: string;
}

type RetryAdmission =
  | { type: "accepted" }
  | { type: "failed"; error: CommandError }
  | { type: "stale"; dispatched: boolean };

function isAdmissionRejection(error: CommandError): boolean {
  if (error.outcomeUnknown) return false;
  switch (error.code) {
    case "busy":
    case "not_configured":
    case "unauthenticated":
    case "permission_denied":
    case "not_found":
      return true;
    case "invalid_argument":
      // Settings validation can also fail after admission. Only release on
      // the CreateRun model validation that precedes allocation.
      return error.violations.some(({ field }) => field === "model_profile");
    default:
      // Transport, unclassified bridge failures, and identity conflicts do
      // not establish that the native command never admitted the run.
      return false;
  }
}

function retryScope(targetId: string, runId: string): string {
  return JSON.stringify([targetId, runId]);
}

function freezeSelection(
  selection: ProviderRetrySelection,
): ProviderRetrySelection {
  return Object.freeze({
    modelSelection: Object.freeze({ ...selection.modelSelection }),
    pluginSkillIds: Object.freeze([...selection.pluginSkillIds]),
  });
}

/** Renderer-only records of admitted selections and unresolved admissions. */
export class ProviderRetryRequests {
  private admittedSelections = new Map<string, ProviderRetrySelection>();
  private readonly pending = new Map<
    string,
    { request: ProviderRetryRequest; uncertain: boolean }
  >();

  rememberAccepted(
    targetId: string,
    runId: string,
    selection: ProviderRetrySelection,
  ): void {
    this.admittedSelections = withBoundedEntry(
      this.admittedSelections,
      retryScope(targetId, runId),
      freezeSelection(selection),
    );
  }

  prepare(
    targetId: string,
    source: Run,
    selection: ProviderRetrySelection,
  ): ProviderRetryRequest | null {
    const scope = retryScope(targetId, source.runId);
    const pending = this.pending.get(scope);
    if (pending !== undefined) return pending.request;
    if (this.pending.size >= MAX_CACHED_IDEMPOTENCY_ATTEMPTS) return null;

    const request = Object.freeze({
      ...freezeSelection(this.admittedSelections.get(scope) ?? selection),
      role: source.role,
      sessionId: source.sessionId,
      idempotencyKey: `desktop-provider-retry:${source.runId}`,
    });
    this.pending.set(scope, { request, uncertain: false });
    return request;
  }

  description(targetId: string, runId: string): string {
    const scope = retryScope(targetId, runId);
    if (this.pending.has(scope))
      return "Send the same continuation request again";
    if (this.admittedSelections.has(scope))
      return "Continue from saved history with the failed run’s model selection";
    return "Continue from saved history with the selected conversation model";
  }

  settle(targetId: string, runId: string, result: RetryAdmission): void {
    const scope = retryScope(targetId, runId);
    const pending = this.pending.get(scope);
    if (pending === undefined) return;
    if (result.type === "accepted") {
      this.pending.delete(scope);
      return;
    }

    const uncertain =
      result.type === "stale"
        ? result.dispatched
        : !isAdmissionRejection(result.error);
    if (uncertain) pending.uncertain = true;
    // A later rejection cannot resolve the uncertainty of an earlier request.
    if (!pending.uncertain) this.pending.delete(scope);
  }
}
