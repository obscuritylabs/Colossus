import { useEffect, useRef, useState } from "react";
import { listWorkflowRuns } from "../api";
import { occurrence, recurrence, workflowFailure } from "../workflows";
import type {
  WorkflowContext,
  WorkflowRun,
  WorkflowSchedule,
} from "../workflows";
import { ScheduledRunDetail } from "./ScheduledRunDetail";

export function ScheduleDetail({
  targetId,
  context,
  schedule,
  busy,
  onRefresh,
  onControl,
  onDelete,
  onLogic,
}: {
  targetId: string;
  context: WorkflowContext;
  schedule: WorkflowSchedule;
  busy: boolean;
  onRefresh: () => void;
  onControl: () => void;
  onDelete: () => void;
  onLogic: () => void;
}) {
  const { record } = schedule;
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [runId, setRunId] = useState<string | null>(record.last_run_id);
  const lastRun = useRef(record.last_run_id);
  const [after, setAfter] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [historyRevision, setHistoryRevision] = useState(0);
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
  return (
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
          <dt>Repeat</dt>
          <dd>{recurrence(record)}</dd>
          <dt>{record.enabled ? "Next occurrence" : "Retained occurrence"}</dt>
          <dd>{occurrence(record.next_fire_at)}</dd>
          <dt>Missed runs</dt>
          <dd>
            {record.misfire_policy === "fire_once"
              ? "Run latest once"
              : "Skip catch-up runs"}
          </dd>
          {record.task && (
            <>
              <dt>Model</dt>
              <dd>
                {record.task.options.model_profile || "Workspace primary"}
              </dd>
              <dt>Effort</dt>
              <dd>{record.task.options.reasoning_effort || "Model default"}</dd>
            </>
          )}
        </dl>
        {record.blocked_reason && <p role="status">{record.blocked_reason}</p>}
        <div className="workflow-actions">
          <button
            className="button secondary"
            disabled={busy}
            onClick={onRefresh}
          >
            Refresh detail
          </button>
          {schedule.controllable && context.schedules_control && (
            <button
              className="button secondary"
              disabled={busy}
              onClick={onControl}
            >
              {record.enabled ? "Review pause" : "Review enable"}
            </button>
          )}
          {schedule.controllable && context.schedules_delete && (
            <button
              className="button secondary schedule-delete-action"
              disabled={busy}
              onClick={onDelete}
            >
              {record.task ? "Delete task" : "Delete schedule"}
            </button>
          )}
          {context.workflows_read && !record.task && (
            <button
              className="button secondary"
              disabled={busy}
              onClick={onLogic}
            >
              View workflow logic
            </button>
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
            {record.task && (
              <>
                <dt>Allowed tools</dt>
                <dd>{record.task.tools.join(", ") || "None"}</dd>
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
              Legacy ownership cannot be claimed. Inputs, runs, and application
              control are unavailable.
            </p>
          )}
        </details>
        {schedule.controllable && context.workflow_runs_read && (
          <section className="workflow-history" aria-label="Task run history">
            <div className="workflow-detail-header">
              <h4>Run history</h4>
              {context.workflow_run_history && (
                <button
                  className="button secondary compact"
                  disabled={loading}
                  onClick={() => setHistoryRevision((value) => value + 1)}
                >
                  Refresh history
                </button>
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
                    onClick={() => setRunId(run.run_id)}
                  >
                    <strong>{run.status.replaceAll("_", " ")}</strong>
                    <span>{occurrence(run.created_at)}</span>
                    <span>{run.run_id}</span>
                  </button>
                </li>
              ))}
            </ul>
            {after && (
              <button
                className="button secondary"
                disabled={loading}
                onClick={() => void more()}
              >
                Load older runs
              </button>
            )}
            {record.last_run_id && (
              <button
                className="button secondary"
                onClick={() => setRunId(record.last_run_id)}
              >
                Inspect last workflow run
              </button>
            )}
          </section>
        )}
      </section>
      <aside className="workflow-output-panel" aria-label="Selected run output">
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
                : "Select a run to view its output. Your first result will appear after the task runs."}
            </p>
          </>
        )}
      </aside>
    </div>
  );
}
