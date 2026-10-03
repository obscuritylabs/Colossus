import { useEffect, useRef, useState } from "react";
import {
  CommandFailure,
  registerWorkflowDefinition,
  validateWorkflowDefinition,
} from "../api";
import { workflowFailure } from "../workflows";
import type { RegisteredWorkflow, WorkflowContext } from "../workflows";
import { WorkflowDialog } from "./WorkflowDialog";

export function WorkflowImport({
  targetId,
  context,
  onClose,
  onRegistered,
}: {
  targetId: string;
  context: WorkflowContext;
  onClose: () => void;
  onRegistered: () => void;
}) {
  const [yaml, setYaml] = useState("");
  const [review, setReview] = useState<{
    workflow: RegisteredWorkflow;
    yaml: string;
    key: string;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [uncertain, setUncertain] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  async function validate() {
    setBusy(true);
    setError("");
    try {
      if (new TextEncoder().encode(yaml).length > 192 * 1024)
        throw new Error("Definition YAML must be no larger than 192 KiB.");
      const workflow = await validateWorkflowDefinition(
        targetId,
        context.selection_epoch,
        yaml,
      );
      if (alive.current)
        setReview({
          workflow,
          yaml,
          key: `desktop-import-${crypto.randomUUID()}`,
        });
    } catch (error) {
      if (alive.current) setError(workflowFailure(error));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function register() {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      await registerWorkflowDefinition(
        targetId,
        context.selection_epoch,
        review.yaml,
        review.workflow.workflow_hash,
        review.key,
      );
      if (alive.current) onRegistered();
    } catch (error) {
      if (alive.current) {
        setError(workflowFailure(error));
        setUncertain(
          error instanceof CommandFailure && error.detail.outcomeUnknown,
        );
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <WorkflowDialog
      title={
        review
          ? "Review workflow registration"
          : "Import existing workflow YAML"
      }
      busy={busy}
      error={error}
      onClose={onClose}
    >
      <p>
        This registers an existing declarative definition in the selected
        Workspace's runtime library. It does not run it. CLI state is separate.
      </p>
      {error && <p role="alert">{error}</p>}
      {review ? (
        <>
          <dl className="workflow-facts">
            <dt>Workflow</dt>
            <dd>{review.workflow.workflow_id}</dd>
            <dt>Exact definition hash</dt>
            <dd>
              <code>{review.workflow.workflow_hash}</code>
            </dd>
            <dt>Description</dt>
            <dd>{review.workflow.description || "No description"}</dd>
          </dl>
          <details>
            <summary>Exact reviewed YAML and input schema</summary>
            <pre>{review.yaml}</pre>
            <pre>{JSON.stringify(review.workflow.input_schema, null, 2)}</pre>
          </details>
          <p>
            Existing name/version definitions cannot be replaced with different
            content. Register a new version when the definition changes.
          </p>
          {uncertain && (
            <p role="status">
              Registration is unconfirmed. Retrying this reviewed request uses
              the same durable key: <code>{review.key}</code>.
            </p>
          )}
          <footer className="workflow-actions">
            <button
              className="button secondary"
              disabled={busy}
              onClick={onClose}
            >
              Close
            </button>
            <button
              className="button secondary"
              disabled={busy || uncertain}
              onClick={() => setReview(null)}
            >
              Edit YAML
            </button>
            <button
              className="button primary"
              disabled={busy}
              onClick={() => void register()}
            >
              {busy
                ? "Registering…"
                : uncertain
                  ? "Reconcile same registration"
                  : "Register workflow"}
            </button>
          </footer>
        </>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void validate();
          }}
        >
          <label>
            Existing definition YAML
            <textarea
              value={yaml}
              rows={15}
              required
              spellCheck={false}
              onChange={(event) => setYaml(event.target.value)}
            />
          </label>
          <footer className="workflow-actions">
            <button
              className="button secondary"
              type="button"
              disabled={busy}
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              className="button primary"
              disabled={busy || !yaml.trim()}
              type="submit"
            >
              {busy ? "Validating…" : "Validate and review"}
            </button>
          </footer>
        </form>
      )}
    </WorkflowDialog>
  );
}
