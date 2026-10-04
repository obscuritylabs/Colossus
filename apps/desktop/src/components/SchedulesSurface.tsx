import { useCallback, useEffect, useRef, useState } from "react";
import {
  getWorkflowSchedule,
  listWorkflowSchedules,
  setWorkflowScheduleEnabled,
  workflowContext,
} from "../api";
import {
  recurrence,
  MISFIRE_GUIDANCE,
  occurrence,
  workflowFailure,
} from "../workflows";
import type { WorkflowContext, WorkflowSchedule } from "../workflows";
import { ScheduleCreate } from "./ScheduleCreate";
import { ScheduledRunDetail } from "./ScheduledRunDetail";
import { WorkflowDialog } from "./WorkflowDialog";
import { ScheduleTaskCreate } from "./ScheduleTaskCreate";
import { WorkflowLogicDialog } from "./WorkflowLogicDialog";
import "./workflows.css";

export function SchedulesSurface({
  targetId,
  workspaceName,
  runtimeReady,
  initialInspection,
}: {
  targetId: string | null;
  workspaceName: string;
  runtimeReady: boolean;
  initialInspection?:
    { scheduleId: string; showRun: boolean } | null | undefined;
}) {
  const [context, setContext] = useState<WorkflowContext | null>(null);
  const [items, setItems] = useState<WorkflowSchedule[]>([]);
  const [after, setAfter] = useState<string | null>(null);
  const [detail, setDetail] = useState<WorkflowSchedule | null>(null);
  const [control, setControl] = useState<WorkflowSchedule | null>(null);
  const [runOpen, setRunOpen] = useState(false);
  const [logicOpen, setLogicOpen] = useState(false);
  const [dialog, setDialog] = useState<"create" | "task" | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [detailError, setDetailError] = useState("");
  const [message, setMessage] = useState("");
  const request = useRef(0);
  const alive = useRef(true);
  const controlTrigger = useRef<HTMLButtonElement>(null);
  const load = useCallback(async () => {
    if (!targetId || !runtimeReady) {
      setLoading(false);
      return;
    }
    const generation = ++request.current;
    setLoading(true);
    setError("");
    setControl(null);
    setDialog(null);
    setLogicOpen(false);
    try {
      const context = await workflowContext(targetId);
      const page = context.schedules_read
        ? await listWorkflowSchedules(targetId, context.selection_epoch, null)
        : { items: [], next_cursor: null };
      if (alive.current && generation === request.current) {
        setContext(context);
        setItems(page.items);
        setAfter(page.next_cursor);
        setDetail(null);
        setRunOpen(false);
      }
    } catch (error) {
      if (alive.current && generation === request.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && generation === request.current) setLoading(false);
    }
  }, [targetId, runtimeReady]);
  useEffect(() => {
    alive.current = true;
    void load();
    return () => {
      alive.current = false;
      request.current++;
    };
  }, [load]);
  useEffect(() => {
    if (!targetId || !context || !initialInspection) return;
    let cancelled = false;
    void getWorkflowSchedule(
      targetId,
      context.selection_epoch,
      initialInspection.scheduleId,
    )
      .then((schedule) => {
        if (!cancelled) {
          setDetail(schedule);
          setRunOpen(
            initialInspection.showRun && !!schedule.record.last_run_id,
          );
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) setDetailError(workflowFailure(error));
      });
    return () => {
      cancelled = true;
    };
  }, [targetId, context, initialInspection]);
  async function more() {
    if (!targetId || !context || !after) return;
    setBusy(true);
    setError("");
    try {
      const page = await listWorkflowSchedules(
        targetId,
        context.selection_epoch,
        after,
      );
      if (alive.current) {
        setItems((items) => [...items, ...page.items]);
        setAfter(page.next_cursor);
      }
    } catch (error) {
      if (alive.current) setError(workflowFailure(error));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function inspect(id: string) {
    if (!targetId || !context) return;
    const generation = ++request.current;
    setBusy(true);
    setDetailError("");
    setRunOpen(false);
    try {
      const schedule = await getWorkflowSchedule(
        targetId,
        context.selection_epoch,
        id,
      );
      if (alive.current && generation === request.current) setDetail(schedule);
    } catch (error) {
      if (alive.current && generation === request.current)
        setDetailError(workflowFailure(error));
    } finally {
      if (alive.current && generation === request.current) setBusy(false);
    }
  }
  async function reviewControl() {
    if (!targetId || !context || !detail) return;
    setBusy(true);
    setDetailError("");
    try {
      const schedule = await getWorkflowSchedule(
        targetId,
        context.selection_epoch,
        detail.record.schedule_id,
      );
      if (alive.current) {
        setDetail(schedule);
        setControl(schedule);
      }
    } catch (error) {
      if (alive.current) setDetailError(workflowFailure(error));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function applyControl() {
    if (!targetId || !context || !control) return;
    setBusy(true);
    setDetailError("");
    try {
      const updated = await setWorkflowScheduleEnabled(
        targetId,
        context.selection_epoch,
        control.record.schedule_id,
        !control.record.enabled,
        control.etag,
      );
      if (alive.current) {
        setDetail(updated);
        setItems((items) =>
          items.map((item) =>
            item.record.schedule_id === updated.record.schedule_id
              ? updated
              : item,
          ),
        );
        setControl(null);
        setMessage(
          `${updated.record.schedule_id} is ${updated.record.enabled ? "enabled" : "paused"}.`,
        );
      }
    } catch (error) {
      if (alive.current) {
        setDetailError(
          `${workflowFailure(error)} Refresh the schedule and review again; controls are never retried automatically.`,
        );
        setControl(null);
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  const record = detail?.record;
  return (
    <section className="workflow-surface" aria-label="Workspace schedules">
      <header className="workflow-page-header">
        <div>
          <p className="surface-breadcrumb">Workspace / {workspaceName}</p>
          <h2>Schedules</h2>
          <p>Schedule an agent task or run an existing workflow on repeat.</p>
        </div>
        <div className="workflow-actions">
          <button
            className="button secondary"
            disabled={loading || busy || !runtimeReady}
            onClick={() => void load()}
          >
            Refresh
          </button>
          <button
            className="button primary"
            disabled={
              loading ||
              busy ||
              !context?.task_schedules ||
              !context.calendar_schedules
            }
            onClick={() => setDialog("task")}
          >
            Schedule a task
          </button>
          <button
            className="button secondary"
            disabled={
              loading ||
              busy ||
              !context?.schedules_create ||
              !context.workflows_read
            }
            onClick={() => setDialog("create")}
          >
            Schedule a workflow
          </button>
        </div>
      </header>
      <aside className="workflow-availability">
        <strong>
          {!runtimeReady
            ? "Runtime unavailable"
            : loading
              ? "Checking runtime capabilities…"
              : context?.managed
                ? "Managed Local worker is running"
                : "Connected External runtime"}
        </strong>
        <p>
          {loading
            ? "Checking availability in the selected Workspace."
            : context?.managed
              ? "Schedules tick while this Workspace's worker is running. Future schedules do not pin or wake sleeping Workspaces."
              : "Scheduling follows this runtime's own availability. Desktop does not start or enroll an External runtime automatically."}
        </p>
        {context?.managed && (
          <details>
            <summary>Background and shutdown behavior</summary>
            <p>
              Retained unselected workers keep ticking. Desktop retains up to
              four workers; an idle Workspace can sleep when another needs
              capacity. Closing the window keeps Colossus in the macOS menu bar
              or Windows system tray. Shut Down Colossus stops workers and
              ticks. Resume reconciles missed occurrences with the selected
              policy.
            </p>
          </details>
        )}
      </aside>
      {!targetId || !runtimeReady ? (
        <p role="status">
          Select and connect this Workspace's runtime to inspect its schedules.
        </p>
      ) : loading ? (
        <p role="status">Loading schedules…</p>
      ) : !context?.schedules_read && !error ? (
        <p role="status">
          This runtime does not advertise schedule access. Its administrator
          must enable workflow resources and enroll the required scopes.
        </p>
      ) : null}
      {error && (
        <p role="alert">
          {error}{" "}
          {items.length > 0 &&
            "Showing previously confirmed data. Refresh before acting."}
        </p>
      )}
      {message && <p role="status">{message}</p>}
      {!loading && context?.schedules_read && !items.length && !error && (
        <div className="workflow-empty">
          <h3>No schedules in this Workspace</h3>
          <p>
            Schedule a task with instructions, or choose a reusable workflow
            from the Workflows library.
          </p>
        </div>
      )}
      {context?.schedules_read && (items.length > 0 || detailError) && (
        <div className="workflow-layout">
          <div>
            <ul className="workflow-list" aria-label="Schedules">
              {items.map((schedule) => (
                <li key={schedule.record.schedule_id}>
                  <button
                    aria-pressed={
                      detail?.record.schedule_id === schedule.record.schedule_id
                    }
                    disabled={busy || loading}
                    onClick={() => void inspect(schedule.record.schedule_id)}
                  >
                    <strong>
                      {schedule.record.task?.name ||
                        schedule.record.schedule_id}
                    </strong>
                    <span>
                      {schedule.record.task
                        ? "Agent task"
                        : `${schedule.record.workflow_name} · ${schedule.record.workflow_version}`}
                    </span>
                    <span className="workflow-status">
                      {schedule.record.blocked_reason
                        ? "Blocked"
                        : schedule.record.enabled
                          ? "Enabled"
                          : "Paused"}
                      {!schedule.controllable && " · Legacy"}
                    </span>
                    <span>{recurrence(schedule.record)}</span>
                    <span>
                      {schedule.record.enabled
                        ? "Next boundary"
                        : "Retained boundary"}
                      :{" "}
                      {new Date(schedule.record.next_fire_at).toLocaleString()}
                    </span>
                    <span>
                      Last dispatch: {schedule.last_dispatch || "None yet"}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            {after && (
              <button
                className="button secondary"
                disabled={busy}
                onClick={() => void more()}
              >
                Load more schedules
              </button>
            )}
          </div>
          <section className="workflow-detail" aria-label="Selected schedule">
            {detailError && <p role="alert">{detailError}</p>}
            {!detail && (
              <p>
                Select a schedule to inspect its canonical details and last
                workflow run.
              </p>
            )}
            {record && detail && (
              <>
                <header className="workflow-detail-header">
                  <h3>{record.task?.name || record.schedule_id}</h3>
                  <div className="workflow-actions">
                    {context.workflows_read && !record.task && (
                      <button
                        className="button secondary"
                        disabled={busy || loading}
                        onClick={() => setLogicOpen(true)}
                      >
                        View workflow logic
                      </button>
                    )}
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() => void inspect(record.schedule_id)}
                    >
                      Refresh detail
                    </button>
                  </div>
                </header>
                <strong className="workflow-status">
                  {record.blocked_reason
                    ? "Blocked"
                    : record.enabled
                      ? "Enabled"
                      : "Paused"}
                </strong>
                {record.task && (
                  <section aria-label="Task instructions">
                    <h4>Instructions</h4>
                    <pre className="workflow-task-instructions">
                      {record.task.instructions}
                    </pre>
                    <p>
                      Model:{" "}
                      {record.task.options.model_profile || "Workspace primary"}{" "}
                      · Effort:{" "}
                      {record.task.options.reasoning_effort || "Model default"}
                    </p>
                  </section>
                )}
                <dl className="workflow-facts">
                  {!record.task && (
                    <>
                      <dt>Workflow</dt>
                      <dd>
                        {record.workflow_name} · {record.workflow_version}
                      </dd>
                      <dt>Pinned hash</dt>
                      <dd>
                        <code>{record.workflow_hash}</code>
                      </dd>
                    </>
                  )}
                  <dt>{record.calendar ? "Repeat" : "Cadence"}</dt>
                  <dd>
                    {recurrence(record)}
                    {!record.calendar && " (fixed elapsed time)"}
                  </dd>
                  <dt>First occurrence</dt>
                  <dd>{occurrence(record.starts_at)}</dd>
                  <dt>Missed runs</dt>
                  <dd>
                    {record.misfire_policy === "fire_once"
                      ? "Run latest once"
                      : "Skip catch-up runs"}
                  </dd>
                  <dt>Next boundary</dt>
                  <dd>{occurrence(record.next_fire_at)}</dd>
                  <dt>Last evaluated</dt>
                  <dd>{occurrence(record.last_scheduled_at)}</dd>
                  <dt>Last dispatch</dt>
                  <dd>
                    {detail.last_dispatch || "None yet"}
                    {detail.last_dispatch === "skipped"
                      ? " — no run was queued for that occurrence"
                      : detail.last_dispatch === "queued"
                        ? " — inspect execution state below"
                        : ""}
                  </dd>
                  <dt>Created</dt>
                  <dd>{occurrence(record.created_at)}</dd>
                  <dt>Updated</dt>
                  <dd>{occurrence(record.updated_at)}</dd>
                  <dt>Origin</dt>
                  <dd>
                    {detail.origin ? (
                      <>
                        {detail.origin.owner.id}
                        {detail.origin.session_id && (
                          <>
                            <br />
                            Session: <code>{detail.origin.session_id}</code>
                          </>
                        )}
                        {detail.origin.run_id && (
                          <>
                            <br />
                            Chat run: <code>{detail.origin.run_id}</code>
                          </>
                        )}
                      </>
                    ) : (
                      "Unknown — legacy record; ownership cannot be claimed"
                    )}
                  </dd>
                </dl>
                {record.blocked_reason && (
                  <p role="status">
                    {record.blocked_reason} Restore the exact pinned definition
                    and review re-enabling, or create a schedule for a newly
                    registered version. This schedule will not be repinned.
                  </p>
                )}
                {detail.controllable && !record.task && (
                  <details>
                    <summary>Immutable input snapshot</summary>
                    <pre>{JSON.stringify(record.inputs, null, 2)}</pre>
                  </details>
                )}
                {!detail.controllable && (
                  <p>
                    Legacy inputs and run details are unavailable. Application
                    control is disabled.
                  </p>
                )}
                <p>{MISFIRE_GUIDANCE}</p>
                <p>
                  Pausing affects future ticks and does not cancel queued or
                  running workflows. Re-enabling retains the next boundary and
                  may reconcile missed occurrences. To change immutable fields,
                  create a new schedule and pause this one.
                </p>
                {detail.controllable && context.schedules_control && (
                  <button
                    ref={controlTrigger}
                    className="button secondary"
                    disabled={busy}
                    onClick={() => void reviewControl()}
                  >
                    {record.enabled ? "Review pause" : "Review enable"}
                  </button>
                )}
                {record.last_run_id && context.workflow_runs_read && (
                  <>
                    <button
                      className="button secondary"
                      onClick={() => setRunOpen((open) => !open)}
                    >
                      {runOpen
                        ? "Hide workflow run"
                        : "Inspect last workflow run"}
                    </button>
                    {runOpen && (
                      <ScheduledRunDetail
                        key={record.last_run_id}
                        targetId={targetId!}
                        context={context}
                        runId={record.last_run_id}
                      />
                    )}
                  </>
                )}
              </>
            )}
          </section>
        </div>
      )}
      {logicOpen && targetId && context && record && (
        <WorkflowLogicDialog
          targetId={targetId}
          context={context}
          workflowId={`${record.workflow_name}:${record.workflow_version}`}
          expectedHash={record.workflow_hash}
          runId={record.last_run_id}
          onClose={() => setLogicOpen(false)}
        />
      )}
      {targetId && context && dialog === "create" && (
        <ScheduleCreate
          targetId={targetId}
          context={context}
          onClose={() => setDialog(null)}
          onCreated={(schedule) => {
            setDialog(null);
            setItems((items) => [
              schedule,
              ...items.filter(
                (item) =>
                  item.record.schedule_id !== schedule.record.schedule_id,
              ),
            ]);
            setDetail(schedule);
            setMessage(`${schedule.record.schedule_id} was created.`);
          }}
        />
      )}
      {targetId && context && dialog === "task" && (
        <ScheduleTaskCreate
          targetId={targetId}
          context={context}
          onClose={() => setDialog(null)}
          onCreated={(schedule) => {
            setDialog(null);
            setItems((items) => [schedule, ...items]);
            setDetail(schedule);
            setMessage("Task schedule created.");
          }}
        />
      )}
      {control && (
        <WorkflowDialog
          returnFocus={controlTrigger.current}
          title={`${control.record.enabled ? "Pause" : "Enable"} ${control.record.schedule_id}?`}
          busy={busy}
          onClose={() => setControl(null)}
        >
          <p>
            Workflow: {control.record.workflow_name} ·{" "}
            {control.record.workflow_version}
          </p>
          <p>
            Pinned hash: <code>{control.record.workflow_hash}</code>
          </p>
          <p>Next boundary: {occurrence(control.record.next_fire_at)}</p>
          <p>
            {control.record.enabled
              ? "Pausing stops future ticks. Already queued or running workflows continue."
              : `Enabling preserves the next boundary. Multiple overdue occurrences use ${control.record.misfire_policy}; one due occurrence always queues a run. A running worker is required.`}
          </p>
          <footer className="workflow-actions">
            <button
              className="button secondary"
              disabled={busy}
              onClick={() => setControl(null)}
            >
              Cancel
            </button>
            <button
              className="button primary"
              disabled={busy}
              onClick={() => void applyControl()}
            >
              {busy
                ? "Saving…"
                : control.record.enabled
                  ? "Pause future ticks"
                  : "Enable schedule"}
            </button>
          </footer>
        </WorkflowDialog>
      )}
    </section>
  );
}
