import * as api from "./api";
import type { WorkflowHost } from "@colossus/ui/automations/host";
export const desktopWorkflowHost: WorkflowHost = {
  createWorkflowSchedule: api.createWorkflowSchedule,
  deleteWorkflowSchedule: api.deleteWorkflowSchedule,
  taskModelProfiles: async () => {
    const settings = await api.getManagedConfiguration();
    return settings.globalConfiguration.models
      .filter((model) => !model.archived)
      .flatMap((model) =>
        model.revisions
          .filter((revision) => revision.revision === model.currentRevision)
          .map((revision) => revision.value.profile),
      );
  },
  getRegisteredWorkflow: api.getRegisteredWorkflow,
  getScheduledWorkflowRun: api.getScheduledWorkflowRun,
  getWorkflowSchedule: api.getWorkflowSchedule,
  listRegisteredWorkflows: api.listRegisteredWorkflows,
  listWorkflowRuns: api.listWorkflowRuns,
  listWorkflowSchedules: api.listWorkflowSchedules,
  registerWorkflowDefinition: api.registerWorkflowDefinition,
  setWorkflowScheduleEnabled: api.setWorkflowScheduleEnabled,
  startWorkflowRun: api.startWorkflowRun,
  validateWorkflowDefinition: api.validateWorkflowDefinition,
  workflowContext: api.workflowContext,
  isOutcomeUnknown: (error) =>
    error instanceof api.CommandFailure && error.detail.outcomeUnknown,
};
