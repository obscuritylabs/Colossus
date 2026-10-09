/** Development-only interaction fixture. No auth, runtime or provider calls. */
import { useMemo, useRef, useState } from "react";
import { WorkflowsSurface } from "@colossus/ui/automations/WorkflowsSurface";
import { SchedulesSurface } from "@colossus/ui/automations/SchedulesSurface";
import {
  WorkflowHostProvider,
  type WorkflowHost,
} from "@colossus/ui/automations/host";
import type {
  CreateScheduleRequest,
  RegisteredWorkflow,
  WorkflowContext,
  WorkflowRun,
  WorkflowSchedule,
  ScheduleRunAttempt,
} from "@colossus/ui/automations/types";
import { WorkspaceNavigation } from "@colossus/ui";
import "@colossus/ui/styles/automations.css";
import "@colossus/ui/styles/workflows.css";
import "@colossus/ui/styles/workflow-logic.css";
const hash = "a".repeat(64);
const definition: RegisteredWorkflow = {
  workflow_id: "workspace-health:1.0.0",
  name: "workspace-health",
  version: "1.0.0",
  workflow_hash: hash,
  description: "Record a bounded Workspace health report.",
  input_schema: {
    type: "object",
    properties: { message: { type: "string" } },
    required: ["message"],
    additionalProperties: false,
  },
  logic: {
    steps: [
      {
        id: "result",
        kind: "emit",
        summary: "Emit the health report",
        branches: [],
      },
    ],
    compensation: [],
  },
  scheduling_eligible: true,
  unavailable_reason: null,
};
const initial: WorkflowSchedule = {
  record: {
    schedule_id: "health-check",
    workflow_name: "workspace-health",
    workflow_version: "1.0.0",
    workflow_hash: hash,
    inputs: { message: "Check the workspace" },
    cadence_seconds: 3600,
    calendar: null,
    task: null,
    misfire_policy: "skip",
    enabled: true,
    starts_at: "2030-10-09T09:00:00Z",
    next_fire_at: "2030-10-09T09:00:00Z",
    last_scheduled_at: null,
    last_run_id: null,
    blocked_reason: null,
    created_at: "2026-10-08T20:00:00Z",
    updated_at: "2026-10-08T20:00:00Z",
  },
  origin: {
    owner: { actor_type: "application", id: "fixture-only" },
    session_id: null,
    run_id: null,
  },
  etag: "fixture-1",
  controllable: true,
  last_dispatch: null,
};
export default function WorkspaceManagementFixture() {
  const [view, setView] = useState<"workflows" | "schedules">("schedules");
  const [readonly, setReadonly] = useState(false);
  const [unknown, setUnknown] = useState(false);
  const [message, setMessage] = useState("");
  const data = useRef({
    definitions: [definition],
    schedules: [initial],
    runs: [] as WorkflowRun[],
    revision: 1,
  });
  const attempts = useRef(new Map<string, ScheduleRunAttempt>());
  const host = useMemo<WorkflowHost>(() => {
    const context: WorkflowContext = {
      selection_epoch: 1,
      managed: false,
      workflows_read: true,
      workflows_register: !readonly,
      schedules_read: true,
      schedules_create: !readonly,
      schedules_control: !readonly,
      schedules_delete: !readonly,
      workflow_runs_read: true,
      workflow_runs_start: !readonly,
      calendar_schedules: true,
      task_schedules: true,
      workflow_run_history: true,
    };
    const getSchedule = (id: string) => {
      const value = data.current.schedules.find(
        (item) => item.record.schedule_id === id,
      );
      if (!value) throw new Error("The schedule was not found.");
      return value;
    };
    const admitted = () => {
      if (readonly) throw new Error("Management permission is unavailable.");
    };
    const finish = <T,>(value: T): T => {
      if (unknown)
        throw Object.assign(
          new Error(
            "The response was lost. Inspect the resource before retrying.",
          ),
          { outcomeUnknown: true },
        );
      return value;
    };
    return {
      workflowContext: async () => context,
      listRegisteredWorkflows: async () => ({
        items: [...data.current.definitions],
        next_cursor: null,
      }),
      getRegisteredWorkflow: async () => definition,
      validateWorkflowDefinition: async () => definition,
      registerWorkflowDefinition: async () => {
        admitted();
        return finish(definition);
      },
      listWorkflowSchedules: async () => ({
        items: [...data.current.schedules],
        next_cursor: null,
      }),
      getWorkflowSchedule: async (_target, _epoch, id) => getSchedule(id),
      createWorkflowSchedule: async (
        _target,
        _epoch,
        request: CreateScheduleRequest,
      ) => {
        admitted();
        const existing = data.current.schedules.find(
          (item) => item.record.schedule_id === request.schedule_id,
        );
        if (existing) return existing;
        const schedule: WorkflowSchedule = {
          ...initial,
          record: {
            ...initial.record,
            ...request,
            workflow_name: request.task ? "task" : definition.name,
            workflow_version: definition.version,
            workflow_hash: request.task ? hash : request.expected_hash,
            next_fire_at: request.starts_at,
            last_scheduled_at: null,
            last_run_id: null,
            blocked_reason: null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
          etag: "fixture-" + ++data.current.revision,
        };
        data.current.schedules.push(schedule);
        return finish(schedule);
      },
      setWorkflowScheduleEnabled: async (
        _target,
        _epoch,
        id,
        enabled,
        etag,
      ) => {
        admitted();
        const old = getSchedule(id);
        if (old.etag !== etag)
          throw new Error("The schedule changed. Refresh and review again.");
        const value = {
          ...old,
          record: { ...old.record, enabled },
          etag: "fixture-" + ++data.current.revision,
        };
        data.current.schedules = data.current.schedules.map((item) =>
          item.record.schedule_id === id ? value : item,
        );
        return finish(value);
      },
      deleteWorkflowSchedule: async (_target, _epoch, id, etag) => {
        admitted();
        if (getSchedule(id).etag !== etag)
          throw new Error("The schedule changed.");
        data.current.schedules = data.current.schedules.filter(
          (item) => item.record.schedule_id !== id,
        );
        return finish({ schedule_id: id });
      },
      listWorkflowRuns: async () => ({
        items: [...data.current.runs],
        next_cursor: null,
      }),
      getScheduledWorkflowRun: async (_target, _epoch, id) => {
        const run = data.current.runs.find((run) => run.run_id === id);
        if (!run) throw new Error("The run was not found.");
        return run;
      },
      startWorkflowRun: async (_target, _epoch, request) => {
        admitted();
        const run: WorkflowRun = {
          run_id: request.idempotency_key,
          workflow_id: request.workflow_id,
          workflow_hash: request.expected_hash,
          status: "completed",
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          last_sequence: 2,
          failure_reason: null,
          waiting_reason: null,
          step_states: [
            { step_id: "result", status: "completed", completed_executions: 0 },
          ],
          result_json: '{"ok":true}',
        };
        if (!data.current.runs.some((item) => item.run_id === run.run_id))
          data.current.runs.push(run);
        return finish(run);
      },
      taskModelProfiles: async () => ["fixture-model"],
      isOutcomeUnknown: (error) =>
        Boolean(
          error && typeof error === "object" && "outcomeUnknown" in error,
        ),
    };
  }, [readonly, unknown]);
  const props = {
    targetId: "fixture-workspace",
    workspaceName: "pointbreak",
    runtimeReady: true,
    onCreateWithAgent: (text: string) =>
      setMessage("Agent prompt prepared: " + text),
    agentStarting: false,
  };
  return (
    <main className="management-fixture">
      <aside>
        <strong>Development fixture</strong>
        <p>No runtime calls</p>
        <WorkspaceNavigation
          active={view}
          onSelect={(id) => {
            if (id === "workflows" || id === "schedules") setView(id);
          }}
        />
        <label>
          <input
            type="checkbox"
            checked={readonly}
            onChange={(event) => setReadonly(event.target.checked)}
          />
          Read-only permission
        </label>
        <label>
          <input
            type="checkbox"
            checked={unknown}
            onChange={(event) => setUnknown(event.target.checked)}
          />
          Simulate lost response
        </label>
        <button
          type="button"
          onClick={() =>
            (document.documentElement.dataset.theme =
              document.documentElement.dataset.theme === "light"
                ? "dark"
                : "light")
          }
        >
          Switch fixture theme
        </button>
      </aside>
      <section>
        <div className="fixture-banner" role="status">
          Development workspace management fixture · disposable data
        </div>
        {message ? <p>{message}</p> : null}
        <WorkflowHostProvider
          key={String(readonly) + String(unknown)}
          host={host}
        >
          {view === "workflows" ? (
            <WorkflowsSurface {...props} />
          ) : (
            <SchedulesSurface
              {...props}
              runAttempts={attempts.current}
              attemptScope="fixture"
            />
          )}
        </WorkflowHostProvider>
      </section>
    </main>
  );
}
