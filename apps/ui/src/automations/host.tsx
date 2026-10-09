import { createContext, useContext, type ReactNode } from "react";
import type {
  StartWorkflowRunRequest,
  CreateScheduleRequest,
  RegisteredWorkflow,
  WorkflowContext,
  WorkflowPage,
  WorkflowRun,
  WorkflowSchedule,
} from "./types";
/** Hosts own transport, identity, authority and request reconciliation. */
export interface WorkflowHost {
  createWorkflowSchedule(
    targetId: string,
    selectionEpoch: number,
    request: CreateScheduleRequest,
  ): Promise<WorkflowSchedule>;
  deleteWorkflowSchedule(
    targetId: string,
    selectionEpoch: number,
    scheduleId: string,
    etag: string,
  ): Promise<{ schedule_id: string }>;
  taskModelProfiles(targetId: string): Promise<string[]>;
  getRegisteredWorkflow(
    targetId: string,
    selectionEpoch: number,
    workflowId: string,
  ): Promise<RegisteredWorkflow>;
  getScheduledWorkflowRun(
    targetId: string,
    selectionEpoch: number,
    runId: string,
  ): Promise<WorkflowRun>;
  getWorkflowSchedule(
    targetId: string,
    selectionEpoch: number,
    scheduleId: string,
  ): Promise<WorkflowSchedule>;
  listRegisteredWorkflows(
    targetId: string,
    selectionEpoch: number,
    after: string | null,
  ): Promise<WorkflowPage<RegisteredWorkflow>>;
  listWorkflowRuns(
    targetId: string,
    selectionEpoch: number,
    workflowId: string,
    after: string | null,
  ): Promise<WorkflowPage<WorkflowRun>>;
  listWorkflowSchedules(
    targetId: string,
    selectionEpoch: number,
    after: string | null,
  ): Promise<WorkflowPage<WorkflowSchedule>>;
  registerWorkflowDefinition(
    targetId: string,
    selectionEpoch: number,
    yaml: string,
    expectedHash: string,
    idempotencyKey: string,
  ): Promise<RegisteredWorkflow>;
  setWorkflowScheduleEnabled(
    targetId: string,
    selectionEpoch: number,
    scheduleId: string,
    enabled: boolean,
    etag: string,
  ): Promise<WorkflowSchedule>;
  startWorkflowRun(
    targetId: string,
    selectionEpoch: number,
    request: StartWorkflowRunRequest,
  ): Promise<WorkflowRun>;
  validateWorkflowDefinition(
    targetId: string,
    selectionEpoch: number,
    yaml: string,
  ): Promise<RegisteredWorkflow>;
  workflowContext(targetId: string): Promise<WorkflowContext>;
  isOutcomeUnknown(error: unknown): boolean;
}
const HostContext = createContext<WorkflowHost | null>(null);
export function WorkflowHostProvider({
  host,
  children,
}: {
  host: WorkflowHost;
  children: ReactNode;
}) {
  return <HostContext.Provider value={host}>{children}</HostContext.Provider>;
}
export function useWorkflowHost(): WorkflowHost {
  const host = useContext(HostContext);
  if (!host) throw new Error("Workflow screens require a host adapter.");
  return host;
}
