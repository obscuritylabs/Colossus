import { useEffect, useRef, useState } from "react";
import { useWorkflowHost } from "./host";
import { Button } from "../index";
import { AutomationOverview } from "../automations";
import { IconPlayerPlay, IconCalendarTime } from "@tabler/icons-react";
import {
  canonicalJson,
  occurrence,
  recurrence,
  workflowFailure,
} from "./types";
import type {
  WorkflowContext,
  StartWorkflowRunRequest,
  ScheduleRunAttempt,
  WorkflowRun,
  WorkflowSchedule,
} from "./types";
import { ScheduledRunDetail } from "./ScheduledRunDetail";

export function ScheduleDetail({
  targetId,
  context,
  schedule,
  busy,
  runAttempts,
  attemptScope,
  onControl,
  onDelete,
  onLogic,
}: {
  targetId: string;
  context: WorkflowContext;
  schedule: WorkflowSchedule;
  busy: boolean;
  runAttempts: Map<string, ScheduleRunAttempt>;
  attemptScope: string;
  onControl: () => void;
  onDelete: () => void;
  onLogic: () => void;
}) {
  const { listWorkflowRuns, startWorkflowRun, isOutcomeUnknown } =
    useWorkflowHost();

  const { record } = schedule;
  const attemptKey = JSON.stringify([attemptScope, record.schedule_id]);
  const retainedAttempt = runAttempts.get(attemptKey);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [runId, setRunId] = useState<string | null>(record.last_run_id);
  const lastRun = useRef(record.last_run_id);
  const [after, setAfter] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [historyRevision, setHistoryRevision] = useState(0);
  const [starting, setStarting] = useState(false);
  const [uncertain, setUncertain] = useState(
    retainedAttempt?.uncertain ?? false,
  );
  const [startError, setStartError] = useState("");
  const runRequest = useRef<StartWorkflowRunRequest | null>(
    retainedAttempt?.request ?? null,
  );
  const startInFlight = useRef(false);
  const alive = useRef(true);
  const generation = useRef(0);
  const workflowId = `${record.workflow_name}:${record.workflow_version}`;
  useEffect(() => {
    const previous = lastRun.current;
    setRunId((selected) =>
      selected === previous ? record.last_run_id : selected,
    );
    lastRun.current = record.last_run_id;
  }, [record.last_run_id]);
  useEffect(() => {
    alive.current = true;
    const current = ++generation.current;
    setRuns([]);
    setAfter(null);
    setError("");
    setLoading(false);
    if (
      schedule.controllable &&
      context.workflow_runs_read &&
      context.workflow_run_history
    ) {
      setLoading(true);
      void listWorkflowRuns(targetId, context.selection_epoch, workflowId, null)
        .then((page) => {
          if (alive.current && current === generation.current) {
            setRuns(page.items);
            setAfter(page.next_cursor);
          }
        })
        .catch((error: unknown) => {
          if (alive.current && current === generation.current)
            setError(workflowFailure(error));
        })
        .finally(() => {
          if (alive.current && current === generation.current)
            setLoading(false);
        });
    }
    return () => {
      alive.current = false;
      generation.current++;
    };
  }, [
    targetId,
    context.selection_epoch,
    context.workflow_run_history,
    context.workflow_runs_read,
    workflowId,
    schedule.controllable,
    record.last_run_id,
    historyRevision,
  ]);
  async function more() {
    if (!after) return;
    const current = generation.current;
    setLoading(true);
    try {
      const page = await listWorkflowRuns(
        targetId,
        context.selection_epoch,
        workflowId,
        after,
      );
      if (alive.current && current === generation.current) {
        setRuns((runs) => [...runs, ...page.items]);
        setAfter(page.next_cursor);
        setError("");
      }
    } catch (error) {
      if (alive.current && current === generation.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && current === generation.current) setLoading(false);
    }
  }
  const canRun =
    schedule.controllable &&
    context.workflow_runs_start &&
    context.workflow_runs_read &&
    record.inputs !== null &&
    !record.blocked_reason;
  async function runNow() {
    if (!canRun || busy || startInFlight.current) return;
    const previous = runAttempts.get(attemptKey);
    const request = previous?.request ??
      runRequest.current ?? {
        workflow_id: workflowId,
        expected_hash: record.workflow_hash,
        inputs: structuredClone(record.inputs!),
        idempotency_key: `desktop-task-run-${crypto.randomUUID()}`,
      };
    if (
      request.workflow_id !== workflowId ||
      request.expected_hash !== record.workflow_hash ||
      canonicalJson(request.inputs) !== canonicalJson(record.inputs)
    ) {
      setStartError(
        "Reconcile the pending run before executing a changed schedule.",
      );
      return;
    }
    if (!previous && runAttempts.size >= 128) {
      setStartError("Reconcile pending task runs before starting more.");
      return;
    }
    const wasUncertain = uncertain || !!previous?.uncertain;
    const attempt: ScheduleRunAttempt = { request, uncertain: true };
    runAttempts.set(attemptKey, attempt);
    runRequest.current = request;
    startInFlight.current = true;
    setStarting(true);
    setStartError("");
    try {
      const run = await startWorkflowRun(
        targetId,
        context.selection_epoch,
        request,
      );
      if (runAttempts.get(attemptKey) === attempt)
        runAttempts.delete(attemptKey);
      if (alive.current) {
        runRequest.current = null;
        setUncertain(false);
        setRuns((items) => [
          run,
          ...items.filter((item) => item.run_id !== run.run_id),
        ]);
        setRunId(run.run_id);
      }
    } catch (error) {
      const unknown = wasUncertain || isOutcomeUnknown(error);
      attempt.uncertain = unknown;
      if (alive.current) {
        setUncertain(unknown);
        setStartError(workflowFailure(error));
      }
    } finally {
      startInFlight.current = false;
      if (alive.current) setStarting(false);
    }
  }
  const nextOccurrence = new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
    ...(record.calendar ? { timeZone: record.calendar.timezone } : {}),
  }).format(new Date(record.next_fire_at));
  return (
    <div className="schedule-detail-page">
      <AutomationOverview
        items={[
          {
            id: "repeat",
            label: (
              <>
                <IconCalendarTime size={16} aria-hidden="true" /> Repeat
              </>
            ),
            value: recurrence(record),
          },
          {
            id: "next",
            label: (
              <>
                <IconPlayerPlay size={16} aria-hidden="true" />{" "}
                {record.enabled ? "Next run" : "Retained occurrence"}
              </>
            ),
            value: nextOccurrence,
            title: occurrence(record.next_fire_at),
          },
          {
            id: "misfire",
            label: (
              <>
                <IconCalendarTime size={16} aria-hidden="true" /> Missed runs
              </>
            ),
            value:
              record.misfire_policy === "fire_once"
                ? "Run latest once"
                : "Skip catch-up runs",
          },
        ]}
      />
      <div className="schedule-run-toolbar">
        <div>
          <h3>Test your {record.task ? "task" : "workflow"}</h3>
          <p>
            Run once with the saved instructions and inputs. The recurring
            schedule stays the same.
          </p>
        </div>
        <Button
          variant="primary"
          disabled={!canRun || busy || starting}
          onClick={() => void runNow()}
        >
          <IconPlayerPlay size={16} aria-hidden="true" />{" "}
          {starting
            ? "Starting…"
            : uncertain
              ? "Confirm same run request"
              : "Run now"}
        </Button>
      </div>
      {!context.workflow_runs_start && (
        <p className="workflow-help">
          This runtime does not support manual runs.
        </p>
      )}
      {startError && <p role="alert">{startError}</p>}
      {uncertain && (
        <p role="status">
          The run is unconfirmed. Confirm the same request to recover its result
          without creating another run.
        </p>
      )}
      <div className="schedule-detail-layout">
        <section className="workflow-detail" aria-label="Selected schedule">
          <div className="workflow-detail-header">
            <h3>{record.task ? "Task instructions" : "Workflow schedule"}</h3>
            <span className="workflow-status">
              {record.blocked_reason
                ? "Blocked"
                : record.enabled
                  ? "Enabled"
                  : "Paused"}
            </span>
          </div>
          {record.task ? (
            <section aria-label="Task instructions">
              <pre className="workflow-task-instructions">
                {record.task.instructions}
              </pre>
            </section>
          ) : (
            <p>
              {record.workflow_name} · {record.workflow_version}
            </p>
          )}
          <dl className="workflow-facts">
            {record.task && (
              <>
                <dt>Model</dt>
                <dd>
                  {record.task.options.model_profile || "Workspace primary"}
                </dd>
                <dt>Effort</dt>
                <dd>
                  {record.task.options.reasoning_effort || "Model default"}
                </dd>
              </>
            )}
            {record.task && (
              <>
                <dt>Allowed tools</dt>
                <dd>{record.task.tools.join(", ") || "None"}</dd>
              </>
            )}
          </dl>
          {record.blocked_reason && (
            <p role="status">{record.blocked_reason}</p>
          )}
          <div className="workflow-actions">
            {schedule.controllable && context.schedules_control && (
              <Button disabled={busy || starting} onClick={onControl}>
                {record.enabled ? "Review pause" : "Review enable"}
              </Button>
            )}
            {schedule.controllable && context.schedules_delete && (
              <Button
                className="button secondary schedule-delete-action"
                disabled={busy || starting}
                onClick={onDelete}
              >
                {record.task ? "Delete task" : "Delete schedule"}
              </Button>
            )}
            {context.workflows_read && !record.task && (
              <Button disabled={busy} onClick={onLogic}>
                View workflow logic
              </Button>
            )}
          </div>
          <details className="schedule-metadata">
            <summary>Schedule details</summary>
            <dl className="workflow-facts">
              <dt>Schedule ID</dt>
              <dd>
                <code>{record.schedule_id}</code>
              </dd>
              <dt>First occurrence</dt>
              <dd>{occurrence(record.starts_at)}</dd>
              <dt>Last evaluated</dt>
              <dd>{occurrence(record.last_scheduled_at)}</dd>
              <dt>Last dispatch</dt>
              <dd>{schedule.last_dispatch || "None yet"}</dd>
              <dt>Created</dt>
              <dd>{occurrence(record.created_at)}</dd>
              <dt>Updated</dt>
              <dd>{occurrence(record.updated_at)}</dd>
              <dt>Pinned hash</dt>
              <dd>
                <code>{record.workflow_hash}</code>
              </dd>
              <dt>Origin</dt>
              <dd>{schedule.origin?.owner.id || "Unknown — legacy record"}</dd>
              {schedule.origin?.session_id && (
                <>
                  <dt>Originating session</dt>
                  <dd>
                    <code>{schedule.origin.session_id}</code>
                  </dd>
                </>
              )}
              {schedule.origin?.run_id && (
                <>
                  <dt>Originating chat run</dt>
                  <dd>
                    <code>{schedule.origin.run_id}</code>
                  </dd>
                </>
              )}
            </dl>
            {!record.task && schedule.controllable && (
              <pre className="workflow-json">
                {JSON.stringify(record.inputs, null, 2)}
              </pre>
            )}
            <p>
              Pausing or deleting stops future ticks. Already queued or running
              workflows continue, and run history is retained. To change
              instructions or timing, create a reviewed replacement.
            </p>
            {!schedule.controllable && (
              <p>
                Legacy ownership cannot be claimed. Inputs, runs, and
                application control are unavailable.
              </p>
            )}
          </details>
        </section>
        <aside
          className="workflow-output-panel"
          aria-label="Selected run output"
        >
          {schedule.controllable && context.workflow_runs_read && runId ? (
            <ScheduledRunDetail
              taskResult={!!record.task}
              key={runId}
              targetId={targetId}
              context={context}
              runId={runId}
            />
          ) : (
            <>
              <h3>Run output</h3>
              <p>
                {!schedule.controllable || !context.workflow_runs_read
                  ? "Run output is unavailable for this schedule."
                  : "Run this task now to see its first result, or come back after its next scheduled run."}
              </p>
            </>
          )}
          {schedule.controllable && context.workflow_runs_read && (
            <section className="workflow-history" aria-label="Task run history">
              <div className="workflow-detail-header">
                <h4>Run history</h4>
                {context.workflow_run_history && (
                  <Button
                    className="compact"
                    disabled={loading || starting}
                    onClick={() => setHistoryRevision((value) => value + 1)}
                  >
                    Refresh history
                  </Button>
                )}
              </div>
              {!record.task && context.workflow_run_history && (
                <p>
                  Runs for this workflow, including manual executions and other
                  schedules.
                </p>
              )}
              {error && <p role="alert">{error}</p>}
              {loading && <p role="status">Loading run history…</p>}
              {!loading && !runs.length && (
                <p>
                  {context.workflow_run_history
                    ? "No runs in the loaded history."
                    : "Run history is unavailable on this runtime."}
                </p>
              )}
              <ul className="workflow-list">
                {runs.map((run) => (
                  <li key={run.run_id}>
                    <button
                      aria-pressed={runId === run.run_id}
                      title={run.run_id}
                      onClick={() => setRunId(run.run_id)}
                    >
                      <strong>{run.status.replaceAll("_", " ")}</strong>
                      <span>{occurrence(run.created_at)}</span>
                    </button>
                  </li>
                ))}
              </ul>
              {after && (
                <Button disabled={loading} onClick={() => void more()}>
                  Load older runs
                </Button>
              )}
              {record.last_run_id && (
                <Button onClick={() => setRunId(record.last_run_id)}>
                  Inspect last workflow run
                </Button>
              )}
            </section>
          )}
        </aside>
      </div>
    </div>
  );
}
