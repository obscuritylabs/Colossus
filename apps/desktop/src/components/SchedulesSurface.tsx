import { useCallback, useEffect, useRef, useState } from "react";
import {
  getWorkflowSchedule,
  deleteWorkflowSchedule,
  listWorkflowSchedules,
  setWorkflowScheduleEnabled,
  workflowContext,
} from "../api";
import { occurrence, workflowFailure } from "../workflows";
import type { WorkflowContext, WorkflowSchedule } from "../workflows";
import { ScheduleCreate } from "./ScheduleCreate";
import { ScheduleDetail } from "./ScheduleDetail";
import { WorkflowDialog } from "./WorkflowDialog";
import { ScheduleTaskCreate } from "./ScheduleTaskCreate";
import { WorkflowLogicDialog } from "./WorkflowLogicDialog";
import { ScheduleExamples } from "./ScheduleExamples";
import { ScheduleInventory } from "./ScheduleInventory";
import type { ScheduleExample } from "./schedule-examples";
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
  const [deleting, setDeleting] = useState(false);
  const [logicOpen, setLogicOpen] = useState(false);
  const [dialog, setDialog] = useState<"create" | "task" | null>(null);
  const [example, setExample] = useState<ScheduleExample | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [detailError, setDetailError] = useState("");
  const [message, setMessage] = useState("");
  const request = useRef(0);
  const alive = useRef(true);
  const overviewHeading = useRef<HTMLHeadingElement>(null);
  const controlReturnFocus = useRef<HTMLElement | null>(null);
  const load = useCallback(async () => {
    const generation = ++request.current;
    setContext(null);
    setItems([]);
    setDetail(null);
    setAfter(null);
    setControl(null);
    setDialog(null);
    setLogicOpen(false);
    setBusy(false);
    setError("");
    setDetailError("");
    setMessage("");
    if (!targetId || !runtimeReady) {
      setLoading(false);
      return;
    }
    setLoading(true);
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
    const generation = request.current;
    let cancelled = false;
    void getWorkflowSchedule(
      targetId,
      context.selection_epoch,
      initialInspection.scheduleId,
    )
      .then((schedule) => {
        if (!cancelled && generation === request.current) {
          setDetail(schedule);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled && generation === request.current)
          setDetailError(workflowFailure(error));
      });
    return () => {
      cancelled = true;
    };
  }, [targetId, context, initialInspection]);
  async function more() {
    if (!targetId || !context || !after) return;
    const generation = request.current;
    setBusy(true);
    setError("");
    try {
      const page = await listWorkflowSchedules(
        targetId,
        context.selection_epoch,
        after,
      );
      if (alive.current && generation === request.current) {
        setItems((items) => [...items, ...page.items]);
        setAfter(page.next_cursor);
      }
    } catch (error) {
      if (alive.current && generation === request.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && generation === request.current) setBusy(false);
    }
  }
  async function inspect(id: string) {
    if (!targetId || !context) return;
    const generation = ++request.current;
    setBusy(true);
    setDetailError("");
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
  async function reviewControl(remove = false) {
    if (!targetId || !context || !detail) return;
    controlReturnFocus.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const generation = request.current;
    setBusy(true);
    setDetailError("");
    try {
      const schedule = await getWorkflowSchedule(
        targetId,
        context.selection_epoch,
        detail.record.schedule_id,
      );
      if (alive.current && generation === request.current) {
        setDetail(schedule);
        setDeleting(remove);
        setControl(schedule);
      }
    } catch (error) {
      if (alive.current && generation === request.current)
        setDetailError(workflowFailure(error));
    } finally {
      if (alive.current && generation === request.current) setBusy(false);
    }
  }
  async function applyControl() {
    if (!targetId || !context || !control) return;
    setBusy(true);
    setDetailError("");
    const generation = request.current;
    try {
      if (deleting) {
        const deleted = await deleteWorkflowSchedule(
          targetId,
          context.selection_epoch,
          control.record.schedule_id,
          control.etag,
        );
        if (alive.current && generation === request.current) {
          setItems((items) =>
            items.filter(
              (item) => item.record.schedule_id !== deleted.schedule_id,
            ),
          );
          setDetail(null);
          setControl(null);
          setMessage(
            `${control.record.task?.name || deleted.schedule_id} was deleted. Existing run history is retained.`,
          );
          requestAnimationFrame(() => overviewHeading.current?.focus());
        }
        return;
      }
      const updated = await setWorkflowScheduleEnabled(
        targetId,
        context.selection_epoch,
        control.record.schedule_id,
        !control.record.enabled,
        control.etag,
      );
      if (alive.current && generation === request.current) {
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
      if (alive.current && generation === request.current) {
        setDetailError(
          `${workflowFailure(error)} Refresh schedules to reconcile the stored state, then review again; controls are never retried automatically.`,
        );
        setControl(null);
      }
    } finally {
      if (alive.current && generation === request.current) setBusy(false);
    }
  }
  const record = detail?.record;
  useEffect(() => {
    if (record) requestAnimationFrame(() => overviewHeading.current?.focus());
  }, [record?.schedule_id]);
  return (
    <section className="workflow-surface" aria-label="Workspace schedules">
      <header className="workflow-page-header">
        <div>
          <p className="surface-breadcrumb">
            Workspace / {workspaceName}
            {detail ? " / Schedules" : ""}
          </p>
          <h2 ref={overviewHeading} tabIndex={-1}>
            {detail
              ? detail.record.task?.name || detail.record.schedule_id
              : "Schedules"}
          </h2>
          <p>
            {detail
              ? `${detail.record.task ? "Scheduled agent task" : "Workflow schedule"} · ${detail.record.enabled ? "Enabled" : "Paused"}`
              : "Schedule an agent task or run an existing workflow on repeat."}
          </p>
        </div>
        <div className="workflow-actions">
          {detail && (
            <button
              className="button secondary"
              disabled={busy}
              onClick={() => {
                request.current++;
                setDetail(null);
                setDetailError("");
                setLogicOpen(false);
                requestAnimationFrame(() => overviewHeading.current?.focus());
              }}
            >
              Back to schedules
            </button>
          )}
          <button
            className="button secondary"
            disabled={loading || busy || !runtimeReady}
            onClick={() =>
              detail ? void inspect(detail.record.schedule_id) : void load()
            }
          >
            Refresh
          </button>
          {!detail && (
            <>
              <button
                className="button primary"
                disabled={
                  loading ||
                  busy ||
                  !context?.task_schedules ||
                  !context.calendar_schedules
                }
                onClick={() => {
                  setExample(null);
                  setDialog("task");
                }}
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
            </>
          )}
        </div>
      </header>
      {!detail && (
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
                capacity. Closing the window keeps Colossus in the macOS menu
                bar or Windows system tray. Shut Down Colossus stops workers and
                ticks. Resume reconciles missed occurrences with the selected
                policy.
              </p>
            </details>
          )}
        </aside>
      )}
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
      {!detail &&
        !loading &&
        context?.schedules_read &&
        !items.length &&
        !error && (
          <div className="workflow-empty">
            <h3>No schedules in this Workspace</h3>
            <p>
              Schedule a task with instructions, or choose a reusable workflow
              from the Workflows library.
            </p>
          </div>
        )}
      {detailError && (
        <div>
          <p role="alert">{detailError}</p>
          <button
            className="button secondary"
            disabled={busy || loading}
            onClick={() =>
              void load().then(() =>
                requestAnimationFrame(() => overviewHeading.current?.focus()),
              )
            }
          >
            Refresh schedules
          </button>
        </div>
      )}
      {context?.schedules_read && !detail && items.length > 0 && (
        <div className="workflow-inventory-view">
          <ScheduleInventory
            items={items}
            selectedId={undefined}
            busy={busy || loading}
            onInspect={(id) => void inspect(id)}
          />
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
      )}
      {detail && targetId && context && (
        <ScheduleDetail
          key={`${context.selection_epoch}:${detail.record.schedule_id}`}
          targetId={targetId}
          context={context}
          schedule={detail}
          busy={busy || loading}
          onRefresh={() => void inspect(detail.record.schedule_id)}
          onControl={() => void reviewControl()}
          onDelete={() => void reviewControl(true)}
          onLogic={() => setLogicOpen(true)}
        />
      )}
      {!detail && (
        <ScheduleExamples
          disabled={
            loading ||
            busy ||
            !runtimeReady ||
            !context?.task_schedules ||
            !context.calendar_schedules
          }
          onUse={(example) => {
            setExample(example);
            setDialog("task");
          }}
        />
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
          key={example?.id || "custom"}
          initial={example}
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
          returnFocus={controlReturnFocus.current}
          title={`${deleting ? "Delete" : control.record.enabled ? "Pause" : "Enable"} ${control.record.task?.name || control.record.schedule_id}?`}
          busy={busy}
          onClose={() => setControl(null)}
        >
          {!control.record.task && (
            <p>
              Workflow: {control.record.workflow_name} ·{" "}
              {control.record.workflow_version}
            </p>
          )}
          {!control.record.task && (
            <p>
              Pinned hash: <code>{control.record.workflow_hash}</code>
            </p>
          )}
          <p>Next boundary: {occurrence(control.record.next_fire_at)}</p>
          <p>
            {deleting
              ? "Deleting stops future ticks and removes this schedule from the list. Already queued or running workflows continue, and run history is retained."
              : control.record.enabled
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
              className={`button primary ${deleting ? "schedule-delete-action" : ""}`}
              disabled={busy}
              onClick={() => void applyControl()}
            >
              {busy
                ? "Saving…"
                : deleting
                  ? "Confirm deletion"
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
