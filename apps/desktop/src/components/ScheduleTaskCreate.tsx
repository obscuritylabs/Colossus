import { useEffect, useRef, useState } from "react";
import {
  CommandFailure,
  createWorkflowSchedule,
  getManagedConfiguration,
  getWorkflowSchedule,
} from "../api";
import {
  canonicalJson,
  MISFIRE_GUIDANCE,
  occurrence,
  recurrence,
  workflowFailure,
  type CreateScheduleRequest,
  type MisfirePolicy,
  type WorkflowContext,
  type WorkflowSchedule,
} from "../workflows";
import { DropdownSelect } from "./DropdownSelect";
import { WorkflowDialog } from "./WorkflowDialog";
import {
  defaultCalendarDraft,
  prepareCalendar,
  ScheduleTiming,
} from "./ScheduleTiming";

export function ScheduleTaskCreate({
  targetId,
  context,
  onClose,
  onCreated,
}: {
  targetId: string;
  context: WorkflowContext;
  onClose: () => void;
  onCreated: (schedule: WorkflowSchedule) => void;
}) {
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const [timing, setTiming] = useState(defaultCalendarDraft);
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [effort, setEffort] = useState("");
  const [tools, setTools] = useState("web.search");
  const [misfire, setMisfire] = useState<MisfirePolicy>("fire_once");
  const [enabled, setEnabled] = useState(true);
  const [review, setReview] = useState<CreateScheduleRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    if (context.managed)
      void getManagedConfiguration()
        .then((settings) => {
          if (alive.current)
            setModels(
              settings.globalConfiguration.models
                .filter((model) => !model.archived)
                .flatMap((model) =>
                  model.revisions
                    .filter(
                      (revision) => revision.revision === model.currentRevision,
                    )
                    .map((revision) => revision.value.profile),
                ),
            );
        })
        .catch(() => {
          /* Runtime default remains available. */
        });
    return () => {
      alive.current = false;
    };
  }, [context.managed]);
  function prepare() {
    try {
      if (!name.trim() || !instructions.trim())
        throw new Error("Add a task name and instructions.");
      if (new TextEncoder().encode(instructions).length > 65536)
        throw new Error("Instructions must be no larger than 64 KiB.");
      const task = {
        name: name.trim(),
        instructions,
        tools: [
          ...new Set(
            tools
              .split(",")
              .map((tool) => tool.trim())
              .filter(Boolean),
          ),
        ],
        options: {
          model_profile: model || null,
          reasoning_effort: effort || null,
        },
      };
      setReview({
        schedule_id: `task-${crypto.randomUUID()}`,
        workflow_id: "",
        expected_hash: "",
        inputs: {},
        cadence_seconds: 0,
        ...prepareCalendar(timing),
        task,
        misfire_policy: misfire,
        enabled,
        idempotency_key: `desktop-task-${crypto.randomUUID()}`,
      });
      setError("");
    } catch (error) {
      setError(workflowFailure(error));
    }
  }
  async function save(check = false) {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      const schedule = check
        ? await getWorkflowSchedule(
            targetId,
            context.selection_epoch,
            review.schedule_id,
          )
        : await createWorkflowSchedule(
            targetId,
            context.selection_epoch,
            review,
          );
      if (
        check &&
        (canonicalJson(schedule.record.task) !== canonicalJson(review.task) ||
          canonicalJson(schedule.record.calendar) !==
            canonicalJson(review.calendar) ||
          schedule.record.cadence_seconds !== 0 ||
          schedule.record.misfire_policy !== review.misfire_policy ||
          new Date(schedule.record.starts_at).getTime() !==
            new Date(review.starts_at).getTime())
      )
        throw new Error(
          "The stored schedule differs from this request. Keep the original retry identity and inspect it.",
        );
      if (alive.current) onCreated(schedule);
    } catch (error) {
      if (alive.current) {
        if (!check)
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
      title={review ? "Review scheduled task" : "Schedule a task"}
      busy={busy}
      onClose={onClose}
      error={error}
    >
      <div className="workflow-dialog-content">
        {error && <p role="alert">{error}</p>}
        {review ? (
          <>
            <h4>{review.task?.name}</h4>
            <pre className="workflow-task-instructions">
              {review.task?.instructions}
            </pre>
            <dl className="workflow-facts">
              <dt>Repeat</dt>
              <dd>{recurrence(review)}</dd>
              <dt>First occurrence</dt>
              <dd>{occurrence(review.starts_at)}</dd>
              <dt>Model</dt>
              <dd>
                {review.task?.options.model_profile ||
                  "Workspace primary model"}
              </dd>
              <dt>Effort</dt>
              <dd>
                {review.task?.options.reasoning_effort || "Model default"}
              </dd>
              <dt>Tools</dt>
              <dd>{review.task?.tools.join(", ") || "None"}</dd>
              <dt>Missed runs</dt>
              <dd>
                {review.misfire_policy === "fire_once"
                  ? "Run latest once"
                  : "Skip catch-up runs"}
              </dd>
              <dt>Initial state</dt>
              <dd>{review.enabled ? "Enabled" : "Paused"}</dd>
            </dl>
            <p>
              Each occurrence starts a fresh agent session in this Workspace.
              Its worker must be running.
            </p>
            {uncertain && (
              <p role="status">
                Creation is unconfirmed. Check the stored schedule or retry this
                exact request using the same durable identity.
              </p>
            )}
            <div className="workflow-actions">
              <button
                className="button secondary"
                disabled={busy || uncertain}
                onClick={() => setReview(null)}
              >
                Edit review
              </button>
              {uncertain && (
                <button
                  className="button secondary"
                  disabled={busy}
                  onClick={() => void save(true)}
                >
                  Check stored schedule
                </button>
              )}
              <button
                className="button primary"
                disabled={busy}
                onClick={() => void save()}
              >
                {busy
                  ? "Saving…"
                  : uncertain
                    ? "Retry same reviewed request"
                    : "Create task schedule"}
              </button>
            </div>
          </>
        ) : (
          <form
            className="workflow-form"
            onSubmit={(event) => {
              event.preventDefault();
              prepare();
            }}
          >
            <label>
              Task name
              <input
                required
                maxLength={128}
                value={name}
                placeholder="Monday cybersecurity briefing"
                onChange={(event) => setName(event.target.value)}
              />
            </label>
            <label>
              Instructions
              <textarea
                className="workflow-instructions-input"
                required
                rows={5}
                value={instructions}
                placeholder="Tell the agent what to do each time this task runs…"
                onChange={(event) => setInstructions(event.target.value)}
              />
            </label>
            <ScheduleTiming value={timing} onChange={setTiming} />
            <details className="workflow-advanced">
              <summary>Advanced</summary>
              <div className="workflow-form">
                <div className="workflow-columns">
                  <label>
                    Model
                    {context.managed ? (
                      <DropdownSelect
                        aria-label="Model"
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                      >
                        <option value="">Workspace primary model</option>
                        {models.map((profile) => (
                          <option key={profile} value={profile}>
                            {profile}
                          </option>
                        ))}
                      </DropdownSelect>
                    ) : (
                      <input
                        maxLength={128}
                        value={model}
                        placeholder="Runtime default, or a configured profile"
                        onChange={(event) => setModel(event.target.value)}
                      />
                    )}
                  </label>
                  <label>
                    Effort
                    <DropdownSelect
                      aria-label="Effort"
                      value={effort}
                      onChange={(event) => setEffort(event.target.value)}
                    >
                      <option value="">Model default</option>
                      {[
                        "none",
                        "minimal",
                        "low",
                        "medium",
                        "high",
                        "xhigh",
                        "max",
                        "ultra",
                      ].map((effort) => (
                        <option key={effort} value={effort}>
                          {effort}
                        </option>
                      ))}
                    </DropdownSelect>
                  </label>
                </div>
                <p className="workflow-help">
                  The selected model must support the chosen effort. Echo uses
                  Model default.
                </p>
                <label>
                  Allowed tools
                  <input
                    value={tools}
                    onChange={(event) => setTools(event.target.value)}
                    placeholder="web.search, web.fetch"
                  />
                </label>
                <p className="workflow-help">
                  Comma-separated registered tool names. Every tool request
                  still follows the Workspace's permissions and approval rules.
                </p>
                <label>
                  Missed runs
                  <DropdownSelect
                    aria-label="Missed runs"
                    value={misfire}
                    onChange={(event) =>
                      setMisfire(event.target.value as MisfirePolicy)
                    }
                  >
                    <option value="fire_once">Run latest once</option>
                    <option value="skip">Skip catch-up runs</option>
                  </DropdownSelect>
                </label>
                <p className="workflow-help">{MISFIRE_GUIDANCE}</p>
                <label className="workflow-checkbox">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={(event) => setEnabled(event.target.checked)}
                  />
                  Enable immediately
                </label>
                <p>
                  Execution:{" "}
                  {context.managed
                    ? "this computer, in the selected Workspace"
                    : "the selected external runtime"}
                  . Each run starts a fresh agent session.
                </p>
              </div>
            </details>
            <div className="workflow-actions">
              <button
                className="button secondary"
                type="button"
                onClick={onClose}
              >
                Cancel
              </button>
              <button className="button primary" type="submit">
                Review task
              </button>
            </div>
          </form>
        )}
      </div>
    </WorkflowDialog>
  );
}
