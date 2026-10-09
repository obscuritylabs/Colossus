import { Button } from "../components/Controls";
import { useEffect, useRef, useState } from "react";
import { useWorkflowHost } from "./host";
import {
  recurrence,
  canonicalJson,
  firstOccurrence,
  MISFIRE_GUIDANCE,
  occurrence,
  scheduleInputs,
  workflowFailure,
} from "./types";
import type {
  CreateScheduleRequest,
  MisfirePolicy,
  RegisteredWorkflow,
  WorkflowContext,
  WorkflowSchedule,
} from "./types";
import { DropdownSelect } from "../components/DropdownSelect";
import { WorkflowInputs } from "./WorkflowInputs";
import {
  defaultCalendarDraft,
  prepareCalendar,
  ScheduleTiming,
} from "./ScheduleTiming";
import { WorkflowDialog } from "./WorkflowDialog";

export function ScheduleCreate({
  targetId,
  context,
  initialWorkflowId = "",
  onClose,
  onCreated,
}: {
  targetId: string;
  initialWorkflowId?: string;
  context: WorkflowContext;
  onClose: () => void;
  onCreated: (schedule: WorkflowSchedule) => void;
}) {
  const {
    createWorkflowSchedule,
    getRegisteredWorkflow,
    getWorkflowSchedule,
    listRegisteredWorkflows,
    isOutcomeUnknown,
  } = useWorkflowHost();

  const [workflows, setWorkflows] = useState<RegisteredWorkflow[]>([]);
  const [after, setAfter] = useState<string | null>(null);
  const [workflowId, setWorkflowId] = useState(initialWorkflowId);
  const [workflow, setWorkflow] = useState<RegisteredWorkflow | null>(null);
  const [id, setId] = useState("");
  const [inputs, setInputs] = useState("{}");
  const [calendarMode, setCalendarMode] = useState(
    !!context.calendar_schedules,
  );
  const [calendarDraft, setCalendarDraft] = useState(defaultCalendarDraft);
  const [preset, setPreset] = useState("3600");
  const [custom, setCustom] = useState("1");
  const [unit, setUnit] = useState("3600");
  const [start, setStart] = useState("");
  const [zone, setZone] = useState<"local" | "utc">("local");
  const [misfire, setMisfire] = useState<MisfirePolicy>("fire_once");
  const [enabled, setEnabled] = useState(false);
  const [review, setReview] = useState<CreateScheduleRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [uncertain, setUncertain] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    let cancelled = false;
    void listRegisteredWorkflows(targetId, context.selection_epoch, null)
      .then((page) => {
        if (!cancelled) {
          setWorkflows(page.items);
          setAfter(page.next_cursor);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) setError(workflowFailure(error));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
      alive.current = false;
    };
  }, [targetId, context.selection_epoch]);
  useEffect(() => {
    let cancelled = false;
    setWorkflow(null);
    if (workflowId)
      void getRegisteredWorkflow(targetId, context.selection_epoch, workflowId)
        .then((value) => {
          if (!cancelled) setWorkflow(value);
        })
        .catch((error: unknown) => {
          if (!cancelled) setError(workflowFailure(error));
        });
    return () => {
      cancelled = true;
    };
  }, [targetId, context.selection_epoch, workflowId]);
  async function more() {
    setBusy(true);
    setError("");
    try {
      const page = await listRegisteredWorkflows(
        targetId,
        context.selection_epoch,
        after,
      );
      if (alive.current) {
        setWorkflows((items) => [...items, ...page.items]);
        setAfter(page.next_cursor);
      }
    } catch (error) {
      if (alive.current) setError(workflowFailure(error));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  function prepare() {
    try {
      if (!workflow)
        throw new Error(
          "Choose a registered workflow and wait for its schema.",
        );
      if (!workflow.scheduling_eligible)
        throw new Error(
          workflow.unavailable_reason ||
            "This definition is not eligible for scheduling.",
        );
      if (!/^[a-z0-9][a-z0-9.-]{0,127}$/.test(id))
        throw new Error(
          "Use a schedule ID of 1–128 lowercase letters, digits, dots, or hyphens.",
        );
      const seconds =
        preset === "custom" ? Number(custom) * Number(unit) : Number(preset);
      if (
        !calendarMode &&
        (!Number.isSafeInteger(seconds) || seconds < 60 || seconds > 2678400)
      )
        throw new Error(
          "Cadence must be a whole number of seconds from 1 minute through 31 days.",
        );
      const request: CreateScheduleRequest = {
        schedule_id: id,
        workflow_id: workflow.workflow_id,
        expected_hash: workflow.workflow_hash,
        inputs: scheduleInputs(inputs),
        cadence_seconds: calendarMode ? 0 : seconds,
        ...(calendarMode
          ? prepareCalendar(calendarDraft)
          : { calendar: null, starts_at: firstOccurrence(start, zone) }),
        task: null,
        misfire_policy: misfire,
        enabled,
        idempotency_key: `desktop-schedule-${crypto.randomUUID()}`,
      };
      setError("");
      setReview(request);
    } catch (error) {
      setError(workflowFailure(error));
    }
  }
  async function submit() {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      const schedule = await createWorkflowSchedule(
        targetId,
        context.selection_epoch,
        review,
      );
      if (alive.current) onCreated(schedule);
    } catch (error) {
      if (alive.current) {
        setError(workflowFailure(error));
        setUncertain(isOutcomeUnknown(error));
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function reconcile() {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      const schedule = await getWorkflowSchedule(
        targetId,
        context.selection_epoch,
        review.schedule_id,
      );
      const record = schedule.record;
      if (
        record.workflow_hash !== review.expected_hash ||
        `${record.workflow_name}:${record.workflow_version}` !==
          review.workflow_id ||
        record.cadence_seconds !== review.cadence_seconds ||
        canonicalJson(record.calendar ?? null) !==
          canonicalJson(review.calendar ?? null) ||
        new Date(record.starts_at).getTime() !==
          new Date(review.starts_at).getTime() ||
        record.misfire_policy !== review.misfire_policy ||
        canonicalJson(record.inputs) !== canonicalJson(review.inputs)
      )
        throw new Error(
          "This ID has different stored intent. Keep the original retry key and review the stored schedule before creating another.",
        );
      if (alive.current) onCreated(schedule);
    } catch (error) {
      if (alive.current)
        setError(
          `${workflowFailure(error)} The result is still unconfirmed. An explicit retry below uses the same reviewed request and retry key.`,
        );
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <WorkflowDialog
      title={review ? "Review workflow schedule" : "Create workflow schedule"}
      busy={busy}
      error={error}
      onClose={onClose}
    >
      {error && <p role="alert">{error}</p>}
      {review ? (
        <>
          <dl className="workflow-facts">
            <dt>Schedule</dt>
            <dd>{review.schedule_id}</dd>
            <dt>Workflow</dt>
            <dd>{review.workflow_id}</dd>
            <dt>Definition hash</dt>
            <dd>
              <code>{review.expected_hash}</code>
            </dd>
            <dt>Cadence</dt>
            <dd>
              {recurrence(review)}
              {!review.calendar && " — fixed elapsed time"}
            </dd>
            <dt>First occurrence</dt>
            <dd>{occurrence(review.starts_at)}</dd>
            <dt>Multiple overdue occurrences</dt>
            <dd>
              {review.misfire_policy === "fire_once"
                ? "Queue the latest once"
                : "Skip catch-up runs"}
            </dd>
            <dt>Initial state</dt>
            <dd>{review.enabled ? "Enabled" : "Paused"}</dd>
          </dl>
          <h4>Immutable inputs</h4>
          <pre>{JSON.stringify(review.inputs, null, 2)}</pre>
          <p>{MISFIRE_GUIDANCE}</p>
          <p>
            Inputs are validated against the registered schema by the runtime
            before allocation. A changed definition requires fresh review.
          </p>
          <p>
            A running Workspace worker is required. Future schedules do not keep
            a sleeping Workspace awake.{" "}
            {review.calendar
              ? "Calendar schedules keep their selected local time through clock changes."
              : "Every 24 hours may shift its local hour across daylight saving changes."}
          </p>
          {uncertain && (
            <p role="status">
              Creation is unconfirmed. Check the stored schedule before doing
              anything else. Keep retry key{" "}
              <code>{review.idempotency_key}</code>.
            </p>
          )}
          <footer className="workflow-actions">
            <Button disabled={busy} onClick={onClose}>
              Close
            </Button>
            <Button
              disabled={busy || uncertain}
              onClick={() => {
                setReview(null);
                setError("");
              }}
            >
              Edit review
            </Button>
            {uncertain && (
              <Button
                variant="primary"
                disabled={busy}
                onClick={() => void reconcile()}
              >
                Check stored schedule
              </Button>
            )}
            <Button
              variant={uncertain ? "secondary" : "primary"}
              disabled={busy}
              onClick={() => void submit()}
            >
              {busy
                ? "Saving…"
                : uncertain
                  ? "Retry same reviewed request"
                  : "Create schedule"}
            </Button>
          </footer>
        </>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            prepare();
          }}
        >
          <label>
            Registered workflow
            <DropdownSelect
              aria-label="Registered workflow"
              value={workflowId}
              onChange={(event) => {
                setWorkflowId(event.target.value);
                setError("");
              }}
              required
            >
              <option value="">
                {loading ? "Loading workflows…" : "Choose a workflow"}
              </option>
              {workflows.map((workflow) => (
                <option key={workflow.workflow_id} value={workflow.workflow_id}>
                  {workflow.name} · {workflow.version}
                </option>
              ))}
            </DropdownSelect>
          </label>
          {!loading && !workflows.length && (
            <p>
              Import and register an existing workflow YAML in this Workspace
              first. CLI registrations use separate state.
            </p>
          )}
          {after && (
            <Button type="button" disabled={busy} onClick={() => void more()}>
              Load more workflows
            </Button>
          )}
          {workflow && (
            <details>
              <summary>Registered input schema</summary>
              <pre>{JSON.stringify(workflow.input_schema, null, 2)}</pre>
              <code>{workflow.workflow_hash}</code>
            </details>
          )}
          <label>
            Schedule ID
            <input
              value={id}
              maxLength={128}
              pattern="[a-z0-9][a-z0-9.\-]*"
              required
              onChange={(event) => setId(event.target.value)}
              autoComplete="off"
            />
          </label>
          <WorkflowInputs
            schema={workflow?.input_schema ?? null}
            value={inputs}
            onChange={setInputs}
          />
          {context.calendar_schedules && (
            <label>
              Timing
              <DropdownSelect
                aria-label="Timing"
                value={calendarMode ? "calendar" : "interval"}
                onChange={(event) =>
                  setCalendarMode(event.target.value === "calendar")
                }
              >
                <option value="calendar">Calendar — daily or weekly</option>
                <option value="interval">Fixed elapsed interval</option>
              </DropdownSelect>
            </label>
          )}
          {calendarMode ? (
            <ScheduleTiming value={calendarDraft} onChange={setCalendarDraft} />
          ) : (
            <>
              <label>
                Cadence
                <DropdownSelect
                  aria-label="Cadence"
                  value={preset}
                  onChange={(event) => setPreset(event.target.value)}
                >
                  <option value="60">Every minute</option>
                  <option value="900">Every 15 minutes</option>
                  <option value="3600">Every hour</option>
                  <option value="86400">Every 24 hours</option>
                  <option value="custom">Custom</option>
                </DropdownSelect>
              </label>
              {preset === "custom" && (
                <div className="workflow-columns">
                  <label>
                    Amount
                    <input
                      type="number"
                      min="1"
                      step="1"
                      required
                      value={custom}
                      onChange={(event) => setCustom(event.target.value)}
                    />
                  </label>
                  <label>
                    Unit
                    <DropdownSelect
                      aria-label="Unit"
                      value={unit}
                      onChange={(event) => setUnit(event.target.value)}
                    >
                      <option value="60">Minutes</option>
                      <option value="3600">Hours</option>
                      <option value="86400">Days</option>
                    </DropdownSelect>
                  </label>
                </div>
              )}
              <div className="workflow-columns">
                <label>
                  Time zone
                  <DropdownSelect
                    aria-label="Time zone"
                    value={zone}
                    onChange={(event) =>
                      setZone(event.target.value as "local" | "utc")
                    }
                  >
                    <option value="local">
                      Local ({Intl.DateTimeFormat().resolvedOptions().timeZone})
                    </option>
                    <option value="utc">UTC — exact occurrence</option>
                  </DropdownSelect>
                </label>
                <label>
                  First occurrence
                  <input
                    type="datetime-local"
                    value={start}
                    required
                    onChange={(event) => setStart(event.target.value)}
                  />
                </label>
              </div>
            </>
          )}
          <label>
            Multiple overdue occurrences
            <DropdownSelect
              aria-label="Multiple overdue occurrences"
              value={misfire}
              onChange={(event) =>
                setMisfire(event.target.value as MisfirePolicy)
              }
            >
              <option value="fire_once">Fire once — queue the latest</option>
              <option value="skip">Skip — no catch-up run</option>
            </DropdownSelect>
          </label>
          <p>{MISFIRE_GUIDANCE}</p>
          <label className="workflow-checkbox">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => setEnabled(event.target.checked)}
            />
            Enable future ticks immediately
          </label>
          <footer className="workflow-actions">
            <Button type="button" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              type="submit"
              disabled={busy || !workflow}
            >
              Review schedule
            </Button>
          </footer>
        </form>
      )}
    </WorkflowDialog>
  );
}
