import { useEffect, useRef, useState } from "react";
import { CommandFailure, startWorkflowRun } from "../api";
import {
  scheduleInputs,
  workflowFailure,
  type RegisteredWorkflow,
  type StartWorkflowRunRequest,
  type WorkflowContext,
  type WorkflowRun,
} from "../workflows";
import { WorkflowDialog } from "./WorkflowDialog";
import { WorkflowInputs } from "./WorkflowInputs";

export function WorkflowRunCreate({
  targetId,
  context,
  workflow,
  onClose,
  onStarted,
}: {
  targetId: string;
  context: WorkflowContext;
  workflow: RegisteredWorkflow;
  onClose: () => void;
  onStarted: (run: WorkflowRun) => void;
}) {
  const [inputs, setInputs] = useState("{}");
  const [review, setReview] = useState<StartWorkflowRunRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  function prepare() {
    try {
      setReview({
        workflow_id: workflow.workflow_id,
        expected_hash: workflow.workflow_hash,
        inputs: scheduleInputs(inputs),
        idempotency_key: `desktop-run-${crypto.randomUUID()}`,
      });
      setError("");
    } catch (error) {
      setError(workflowFailure(error));
    }
  }
  async function start() {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      const run = await startWorkflowRun(
        targetId,
        context.selection_epoch,
        review,
      );
      if (alive.current) onStarted(run);
    } catch (error) {
      if (alive.current) {
        setUncertain(
          error instanceof CommandFailure && error.detail.outcomeUnknown,
        );
        setError(workflowFailure(error));
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <WorkflowDialog
      title={review ? "Review workflow run" : "Run workflow"}
      busy={busy}
      onClose={onClose}
      error={error}
    >
      <div className="workflow-dialog-content">
        <h4>
          {workflow.name}{" "}
          <span className="workflow-status">{workflow.version}</span>
        </h4>
        <p>
          Run once in the selected Workspace. Each run has its own history and
          results.
        </p>
        {error && <p role="alert">{error}</p>}
        {review ? (
          <>
            <dl className="workflow-facts">
              <dt>Definition</dt>
              <dd>{review.workflow_id}</dd>
              <dt>Pinned hash</dt>
              <dd>
                <code>{review.expected_hash}</code>
              </dd>
            </dl>
            <h4>Input snapshot</h4>
            <pre className="workflow-json">
              {JSON.stringify(review.inputs, null, 2)}
            </pre>
            {uncertain && (
              <p role="status">
                The allocation is unconfirmed. Confirming this same request
                reuses its durable identity and cannot allocate a second run.
              </p>
            )}
            <div className="workflow-actions">
              <button
                className="button secondary"
                disabled={busy || uncertain}
                onClick={() => setReview(null)}
              >
                Back
              </button>
              <button
                className="button primary"
                disabled={busy}
                onClick={() => void start()}
              >
                {busy
                  ? "Starting…"
                  : uncertain
                    ? "Confirm same run request"
                    : "Start workflow"}
              </button>
            </div>
          </>
        ) : (
          <>
            <WorkflowInputs
              schema={workflow.input_schema}
              value={inputs}
              onChange={setInputs}
            />
            <div className="workflow-actions">
              <button className="button secondary" onClick={onClose}>
                Cancel
              </button>
              <button className="button primary" onClick={prepare}>
                Review run
              </button>
            </div>
          </>
        )}
      </div>
    </WorkflowDialog>
  );
}
