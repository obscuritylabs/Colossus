import { useEffect, useState } from "react";
import { getRegisteredWorkflow, getScheduledWorkflowRun } from "../api";
import {
  workflowFailure,
  type RegisteredWorkflow,
  type WorkflowContext,
  type WorkflowRun,
} from "../workflows";
import { WorkflowDialog } from "./WorkflowDialog";
import { WorkflowLogicGraph } from "./WorkflowLogicGraph";

export function WorkflowLogicDialog({
  targetId,
  context,
  workflowId,
  expectedHash,
  runId,
  onClose,
}: {
  targetId: string;
  context: WorkflowContext;
  workflowId: string;
  expectedHash: string;
  runId?: string | null;
  onClose: () => void;
}) {
  const [workflow, setWorkflow] = useState<RegisteredWorkflow | null>(null);
  const [run, setRun] = useState<WorkflowRun | null>(null);
  const [error, setError] = useState("");
  const [runError, setRunError] = useState("");
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    let cancelled = false;
    setWorkflow(null);
    setError("");
    void getRegisteredWorkflow(targetId, context.selection_epoch, workflowId)
      .then((value) => {
        if (cancelled) return;
        if (value.workflow_hash !== expectedHash)
          throw new Error(
            "The registered definition differs from this schedule's pinned hash. Its graph cannot be shown.",
          );
        setWorkflow(value);
      })
      .catch((error) => {
        if (!cancelled) setError(workflowFailure(error));
      });
    return () => {
      cancelled = true;
    };
  }, [targetId, context.selection_epoch, workflowId, expectedHash]);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let polls = 0;
    setRun(null);
    setRunError("");
    if (!runId || !context.workflow_runs_read) return;
    const load = async () => {
      try {
        const value = await getScheduledWorkflowRun(
          targetId,
          context.selection_epoch,
          runId,
        );
        if (cancelled) return;
        if (
          value.workflow_hash !== expectedHash ||
          value.workflow_id !== workflowId
        )
          throw new Error(
            "This run belongs to a different definition. Its step states cannot be applied to this graph.",
          );
        setRun((current) =>
          !current || value.last_sequence >= current.last_sequence
            ? value
            : current,
        );
        setRunError("");
        if (
          ["queued", "running", "waiting"].includes(value.status) &&
          ++polls < 60
        )
          timer = setTimeout(() => void load(), 5000);
      } catch (error) {
        if (!cancelled) setRunError(workflowFailure(error));
      }
    };
    void load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [
    targetId,
    context.selection_epoch,
    context.workflow_runs_read,
    workflowId,
    expectedHash,
    runId,
    refresh,
  ]);
  return (
    <WorkflowDialog
      title="Workflow logic"
      busy={false}
      error={error}
      onClose={onClose}
      className="workflow-logic-dialog"
      headerActions={
        <button className="button secondary" onClick={onClose}>
          Close graph
        </button>
      }
    >
      <div className="workflow-logic-heading">
        <div>
          <strong>{workflowId}</strong>
          <span>
            {workflow?.description ?? "Loading the pinned definition…"}
          </span>
        </div>
        {run && (
          <span className={`workflow-status status-${run.status}`}>
            {run.status}
          </span>
        )}
        {runId && context.workflow_runs_read && (
          <button
            className="button secondary"
            onClick={() => setRefresh((value) => value + 1)}
          >
            Refresh run
          </button>
        )}
      </div>
      {error && <p role="alert">{error}</p>}
      {runError && <p role="alert">{runError}</p>}
      {!workflow && !error && <p role="status">Loading workflow logic…</p>}
      {workflow && !workflow.logic && (
        <p role="status">
          This runtime does not provide a displayable workflow definition.
          Schedule and run details remain available.
        </p>
      )}
      {workflow?.logic && (
        <WorkflowLogicGraph logic={workflow.logic} run={run} />
      )}
    </WorkflowDialog>
  );
}
