import { useCallback, useEffect, useRef, useState } from "react";
import { getScheduledWorkflowRun } from "../api";
import { occurrence, workflowFailure } from "../workflows";
import type { WorkflowContext, WorkflowRun } from "../workflows";
export function ScheduledRunDetail({
  targetId,
  context,
  runId,
}: {
  targetId: string;
  context: WorkflowContext;
  runId: string;
}) {
  const [run, setRun] = useState<WorkflowRun | null>(null);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const generation = useRef(0);
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
      <h4>Workflow run</h4>
      <p>
        <code>{runId}</code>
      </p>
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
          <dl className="workflow-facts">
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
          {run.waiting_reason && (
            <p>
              {run.waiting_reason} Use the authorized runtime operator interface
              to resolve the dependency.
            </p>
          )}
          {run.failure_reason && <p role="status">{run.failure_reason}</p>}
          {run.status === "interrupted" && (
            <p>
              The worker stopped before this workflow completed. Review its
              state before starting a replacement.
            </p>
          )}
          <p>
            This is an independent workflow execution. It does not resume the
            originating chat.
          </p>
        </>
      )}
      <button className="button secondary" onClick={() => void load()}>
        Refresh run state
      </button>
    </section>
  );
}
