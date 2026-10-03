import type { Page } from "@playwright/test";

export async function installWorkflowFixture(
  page: Page,
  options: { empty?: boolean; supported?: boolean } = {},
) {
  await page.addInitScript((options) => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      workflowCalls: { command: string; args: Record<string, unknown> }[];
      workflowConflict?: boolean;
      workflowUncertain?: boolean;
      workflowBadInput?: boolean;
      workflowRunStatus?: string;
    };
    host.workflowCalls = [];
    const hash = "a".repeat(64);
    const workflow = {
      workflow_id: "workspace-health:1.0.0",
      name: "workspace-health",
      version: "1.0.0",
      description: "Record a bounded Workspace health report.",
      workflow_hash: hash,
      input_schema: {
        type: "object",
        required: ["message"],
        properties: { message: { type: "string" } },
        additionalProperties: false,
      },
      scheduling_eligible: true,
      unavailable_reason: null,
    };
    const record = {
      schedule_id: "hourly-health",
      workflow_name: workflow.name,
      workflow_version: workflow.version,
      workflow_hash: hash,
      inputs: { message: "Check Workspace health" } as Record<
        string,
        unknown
      > | null,
      cadence_seconds: 3600,
      misfire_policy: "fire_once" as "fire_once" | "skip",
      enabled: true,
      starts_at: "2026-10-03T09:00:00Z",
      next_fire_at: "2026-10-04T10:00:00Z",
      last_scheduled_at: "2026-10-04T09:00:00Z" as string | null,
      last_run_id: "workflow-run-health-01" as string | null,
      blocked_reason: null,
      created_at: "2026-10-03T08:30:00Z",
      updated_at: "2026-10-04T09:00:00Z",
    };

    type Schedule = {
      record: typeof record;
      etag: string;
      controllable: boolean;
      last_dispatch: string | null;
      origin: {
        owner: { actor_type: string; id: string };
        session_id: string | null;
        run_id: string | null;
      } | null;
    };
    const schedules: Schedule[] = options.empty
      ? []
      : [
          {
            record,
            etag: "b".repeat(64),
            controllable: true,
            last_dispatch: "queued",
            origin: {
              owner: {
                actor_type: "application",
                id: "app:colossus-desktop-managed",
              },
              session_id: "chat-health",
              run_id: "chat-run-health",
            },
          },
          {
            record: {
              ...record,
              schedule_id: "legacy-report",
              enabled: false,
              last_run_id: null,
              inputs: null,
            },
            etag: "c".repeat(64),
            controllable: false,
            last_dispatch: "skipped",
            origin: null,
          },
        ];
    let registered = !options.empty;
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: Record<string, unknown>) => {
        host.workflowCalls.push({ command, args });
        if (command === "workflow_context")
          return {
            selection_epoch: 41,
            workflows_read: true,
            workflows_register: true,
            schedules_read: options.supported !== false,
            schedules_create: options.supported !== false,
            schedules_control: options.supported !== false,
            workflow_runs_read: true,
            managed: true,
          };
        if (args.selectionEpoch !== 41)
          throw {
            message: "The Workspace changed. Refresh and review again.",
            code: "selection_epoch",
            retryable: false,
            outcomeUnknown: false,
            violations: [],
          };
        if (command === "list_registered_workflows")
          return {
            items: registered ? [{ ...workflow, input_schema: null }] : [],
            next_cursor: null,
          };
        if (
          command === "get_registered_workflow" ||
          command === "validate_workflow_definition"
        )
          return workflow;
        if (command === "register_workflow_definition") {
          registered = true;
          return workflow;
        }
        if (command === "list_workflow_schedules")
          return {
            items: schedules.map((schedule) => ({
              ...schedule,
              record: { ...schedule.record, inputs: null },
            })),
            next_cursor: null,
          };
        if (command === "get_workflow_schedule") {
          const schedule = schedules.find(
            (schedule) => schedule.record.schedule_id === args.scheduleId,
          );
          if (!schedule)
            throw {
              message: "Schedule not found.",
              code: "not_found",
              retryable: false,
              outcomeUnknown: false,
              violations: [],
            };
          return structuredClone(schedule);
        }
        if (command === "create_workflow_schedule") {
          const request = args.request as {
            schedule_id: string;
            expected_hash: string;
            workflow_id: string;
            inputs: Record<string, unknown>;
            cadence_seconds: number;
            starts_at: string;
            misfire_policy: "fire_once" | "skip";
            enabled: boolean;
            idempotency_key: string;
          };
          if (
            host.workflowBadInput ||
            typeof request.inputs.message !== "string"
          )
            throw {
              code: "invalid_argument",
              message: "Inputs do not match the registered workflow schema.",
              retryable: false,
              outcomeUnknown: false,
              violations: [
                {
                  field: "inputs.message",
                  description: "A message string is required.",
                },
              ],
            };
          let schedule = schedules.find(
            (schedule) => schedule.record.schedule_id === request.schedule_id,
          );
          if (!schedule) {
            schedule = {
              record: {
                ...record,
                schedule_id: request.schedule_id,
                inputs: request.inputs,
                cadence_seconds: request.cadence_seconds,
                starts_at: request.starts_at,
                next_fire_at: request.starts_at,
                misfire_policy: request.misfire_policy,
                enabled: request.enabled,
                last_scheduled_at: null,
                last_run_id: null,
              },
              etag: "d".repeat(64),
              controllable: true,
              origin: {
                owner: {
                  actor_type: "application",
                  id: "app:colossus-desktop-managed",
                },
                session_id: null,
                run_id: null,
              },
              last_dispatch: null,
            };
            schedules.push(schedule);
          }
          if (host.workflowUncertain) {
            host.workflowUncertain = false;
            throw {
              code: "outcome_unknown",
              message: "The runtime did not confirm allocation.",
              retryable: false,
              outcomeUnknown: true,
              violations: [],
            };
          }
          return structuredClone(schedule);
        }
        if (command === "set_workflow_schedule_enabled") {
          const request = args.request as {
            schedule_id: string;
            enabled: boolean;
            etag: string;
          };
          const schedule = schedules.find(
            (schedule) => schedule.record.schedule_id === request.schedule_id,
          )!;
          if (host.workflowConflict || request.etag !== schedule.etag)
            throw {
              code: "conflict",
              message:
                "The canonical schedule changed. Inspect and review again.",
              retryable: false,
              outcomeUnknown: false,
              violations: [],
            };
          schedule.record.enabled = request.enabled;
          schedule.etag = "e".repeat(64);
          return structuredClone(schedule);
        }
        if (command === "get_scheduled_workflow_run")
          return {
            run_id: args.runId,
            workflow_id: workflow.workflow_id,
            workflow_hash: hash,
            status: host.workflowRunStatus || "waiting",
            created_at: record.last_scheduled_at,
            updated_at: record.updated_at,
            last_sequence: 4,
            failure_reason:
              host.workflowRunStatus === "failed"
                ? "Workflow failed; inspect authorized runtime evidence."
                : null,
            waiting_reason:
              "Workflow is waiting for operator input or a dependency.",
          };
        throw new Error(`Unexpected workflow command: ${command}`);
      },
    };
  }, options);
}
