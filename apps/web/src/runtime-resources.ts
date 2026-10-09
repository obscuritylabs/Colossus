import type { WorkflowHost } from "@colossus/ui/automations/host";
import type { WorkflowContext } from "@colossus/ui/automations/types";
import { ApiFailure, projectPath, request, type Permission } from "./api";

export class RuntimeResourceFailure extends Error {
  constructor(
    message: string,
    public readonly outcomeUnknown: boolean,
  ) {
    super(message);
  }
}
type Reply<T> = { connection_id: string } & (
  | { kind: "result"; value: T }
  | {
      kind: "failed";
      error: {
        message: string;
        outcome: string;
        violations?: { description: string }[];
      };
    }
);
/** A page-scoped adapter fences reviewed forms to the exact live connection. */
export function createWorkflowHost(
  project: string,
  permissions: Permission[],
  models: () => string[],
): WorkflowHost {
  let nextEpoch = 0;
  const contextIntents = new Map<string, number>();
  const selections = new Map<string, { epoch: number; connection: string }>();
  async function operation<T>(
    targetId: string,
    epoch: number | null,
    input: Record<string, unknown>,
    mutation = false,
  ): Promise<{ value: T; connection: string }> {
    const selected = selections.get(targetId);
    if (epoch !== null && (!selected || epoch !== selected.epoch))
      throw new RuntimeResourceFailure(
        "The Workspace changed. Refresh and review this operation again.",
        false,
      );
    let reply: Reply<T>;
    try {
      reply = await request<Reply<T>>(
        `${projectPath(project)}/nodes/${encodeURIComponent(targetId)}/resources`,
        {
          connection_id: epoch === null ? null : selected!.connection,
          operation: input,
        },
      );
    } catch (error) {
      const uncertain =
        mutation &&
        !(
          error instanceof ApiFailure && [400, 409, 429].includes(error.status)
        );
      throw new RuntimeResourceFailure(
        uncertain
          ? "The runtime response was lost. Inspect the resource and reconcile the same request before retrying."
          : error instanceof Error
            ? error.message
            : "The runtime request failed.",
        uncertain,
      );
    }
    if (reply.kind === "failed")
      throw new RuntimeResourceFailure(
        reply.error.violations?.map((v) => v.description).join(" ") ||
          reply.error.message,
        reply.error.outcome === "unknown",
      );
    if (
      epoch !== null &&
      (reply.connection_id !== selected!.connection ||
        selections.get(targetId) !== selected)
    )
      throw new RuntimeResourceFailure(
        "The connection changed. Refresh and review again.",
        mutation,
      );
    return { value: reply.value, connection: reply.connection_id };
  }
  const call = async <T>(
    target: string,
    epoch: number,
    input: Record<string, unknown>,
    mutation = false,
  ) => (await operation<T>(target, epoch, input, mutation)).value;
  return {
    async workflowContext(targetId) {
      const epoch = ++nextEpoch;
      contextIntents.set(targetId, epoch);
      const result = await operation<{ capabilities: string[] }>(
        targetId,
        null,
        { operation: "context" },
      );
      if (contextIntents.get(targetId) !== epoch)
        throw new RuntimeResourceFailure(
          "The workspace was refreshed. Review the current view.",
          false,
        );
      selections.set(targetId, { epoch, connection: result.connection });
      const has = (cap: string, permission: Permission = "read") =>
        permissions.includes(permission) &&
        result.value.capabilities.includes(cap);
      const context: WorkflowContext = {
        selection_epoch: epoch,
        managed: false,
        workflows_read: has("workflows.read"),
        workflows_register: has("workflows.register", "control"),
        schedules_read: has("schedules.read"),
        schedules_create: has("schedules.create", "execute"),
        schedules_control: has("schedules.control", "control"),
        schedules_delete: has("schedules.delete", "control"),
        workflow_runs_read: has("workflow_runs.read"),
        workflow_runs_start: has("workflow_runs.start", "execute"),
        calendar_schedules: has("schedules.calendar"),
        task_schedules: has("schedules.tasks"),
        workflow_run_history: has("workflow_runs.history"),
      };
      return context;
    },
    listRegisteredWorkflows: (target, epoch, after) =>
      call(target, epoch, { operation: "list_workflows", after }),
    getRegisteredWorkflow: (target, epoch, id) =>
      call(target, epoch, { operation: "get_workflow", id }),
    validateWorkflowDefinition: (target, epoch, yaml) =>
      call(target, epoch, { operation: "validate_workflow", yaml }),
    registerWorkflowDefinition: (
      target,
      epoch,
      yaml,
      expected_hash,
      idempotency_key,
    ) =>
      call(
        target,
        epoch,
        {
          operation: "register_workflow",
          yaml,
          expected_hash,
          idempotency_key,
        },
        true,
      ),
    listWorkflowSchedules: (target, epoch, after) =>
      call(target, epoch, { operation: "list_schedules", after }),
    getWorkflowSchedule: (target, epoch, id) =>
      call(target, epoch, { operation: "get_schedule", id }),
    createWorkflowSchedule: (target, epoch, request) =>
      call(target, epoch, { operation: "create_schedule", request }, true),
    setWorkflowScheduleEnabled: (target, epoch, schedule_id, enabled, etag) =>
      call(
        target,
        epoch,
        {
          operation: "set_schedule_enabled",
          request: { schedule_id, enabled, etag },
        },
        true,
      ),
    deleteWorkflowSchedule: (target, epoch, schedule_id, etag) =>
      call(
        target,
        epoch,
        { operation: "delete_schedule", request: { schedule_id, etag } },
        true,
      ),
    listWorkflowRuns: (target, epoch, workflow_id, after) =>
      call(target, epoch, {
        operation: "list_workflow_runs",
        workflow_id,
        after,
      }),
    getScheduledWorkflowRun: (target, epoch, id) =>
      call(target, epoch, { operation: "get_workflow_run", id }),
    startWorkflowRun: (target, epoch, request) =>
      call(target, epoch, { operation: "start_workflow_run", request }, true),
    taskModelProfiles: async () => models(),
    isOutcomeUnknown: (error) =>
      error instanceof RuntimeResourceFailure && error.outcomeUnknown,
  };
}
