import { useCallback, useEffect, useRef, useState } from "react";
import { getScheduledWorkflowRun } from "../api";
import { occurrence, scheduledTaskText, workflowFailure } from "../workflows";
import type { WorkflowContext, WorkflowRun } from "../workflows";
import { MarkdownContent } from "./MarkdownContent";
export function ScheduledRunDetail({
  targetId,
  context,
  runId,
  taskResult = false,
}: {
  targetId: string;
  context: WorkflowContext;
  runId: string;
  taskResult?: boolean;
}) {
  const [run, setRun] = useState<WorkflowRun | null>(null);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const generation = useRef(0);
  const resultJson =
    run?.result_json ||
    (run?.result ? JSON.stringify(run.result, null, 2) : null);
  const taskText =
    taskResult && resultJson ? scheduledTaskText(resultJson) : null;
  const load = useCallback(async () => {
    const currentGeneration = ++generation.current;
    try {
      const value = await getScheduledWorkflowRun(
        targetId,
        context.selection_epoch,
        runId,
      );
      if (alive.current && currentGeneration === generation.current) {
        setRun((current) =>
          !current || value.last_sequence >= current.last_sequence
            ? value
            : current,
        );
        setError("");
      }
      return value.status;
    } catch (error) {
      if (alive.current && currentGeneration === generation.current)
        setError(workflowFailure(error));
      return null;
    }
  }, [targetId, context.selection_epoch, runId]);
  useEffect(() => {
    alive.current = true;
    setRun(null);
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let polls = 0;
    const refresh = async () => {
      const status = await load();
      if (
        !cancelled &&
        alive.current &&
        status &&
        ["queued", "running", "waiting"].includes(status) &&
        ++polls < 60
      )
        timer = setTimeout(() => void refresh(), 5000);
    };
    void refresh();
    return () => {
      cancelled = true;
      alive.current = false;
      generation.current++;
      clearTimeout(timer);
    };
  }, [load]);
  return (
    <section className="workflow-run" aria-label="Independent workflow run">
      <h4>Run output</h4>
      {error && (
        <p role="alert">
          {error} {run && "Showing the last confirmed state."}
        </p>
      )}
      {!run && !error && <p role="status">Loading workflow state…</p>}
      {run && (
        <>
          <strong className={`workflow-status status-${run.status}`}>
            {run.status.replaceAll("_", " ")}
          </strong>
          {run.waiting_reason && (
            <p>
              {run.waiting_reason} Use the authorized runtime operator interface
              to resolve the dependency.
            </p>
          )}
          {run.failure_reason && <p role="status">{run.failure_reason}</p>}
          {taskText && (
            <section aria-label="Task result" className="workflow-task-result">
              <MarkdownContent content={taskText} />
            </section>
          )}
          {resultJson && (
            <details open={!taskText}>
              <summary>{taskText ? "Raw result (JSON)" : "Result"}</summary>
              <pre className="workflow-json">{resultJson}</pre>
            </details>
          )}
          {!run.result_json && !run.result && (
            <p className="workflow-help">
              {["queued", "running", "waiting"].includes(run.status)
                ? "Output will appear after this run completes."
                : "This run has no released output within the display limit."}
            </p>
          )}
          <details className="workflow-run-metadata">
            <summary>Run details</summary>
            <dl className="workflow-facts">
              <dt>Run ID</dt>
              <dd>
                <code>{runId}</code>
              </dd>
              <dt>Definition</dt>
              <dd>{run.workflow_id}</dd>
              <dt>Hash</dt>
              <dd>
                <code>{run.workflow_hash}</code>
              </dd>
              <dt>Queued</dt>
              <dd>{occurrence(run.created_at)}</dd>
              <dt>Updated</dt>
              <dd>{occurrence(run.updated_at)}</dd>
            </dl>
          </details>
          {run.status === "interrupted" && (
            <p>
              The worker stopped before this workflow completed. Review its
              state before starting a replacement.
            </p>
          )}
        </>
      )}
      <button className="button secondary" onClick={() => void load()}>
        Refresh run state
      </button>
    </section>
  );
}
