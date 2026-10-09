import { useMemo, useRef } from "react";
import { WorkflowHostProvider } from "@colossus/ui/automations/host";
import { WorkflowsSurface } from "@colossus/ui/automations/WorkflowsSurface";
import { SchedulesSurface } from "@colossus/ui/automations/SchedulesSurface";
import type { ScheduleRunAttempt } from "@colossus/ui/automations/types";
import { createWorkflowHost } from "./runtime-resources";
import type { FleetNode, Permission } from "./api";
import { workspaceName } from "./workspace-navigation";
import "@colossus/ui/styles/automations.css";
import "@colossus/ui/styles/workflows.css";
import "@colossus/ui/styles/workflow-logic.css";
export function WorkspaceAutomations({
  project,
  agent,
  permissions,
  view,
  onCreateWithAgent,
  busy,
}: {
  project: string;
  agent: FleetNode;
  permissions: Permission[];
  view: "workflows" | "schedules";
  onCreateWithAgent: (prompt: string) => void;
  busy: boolean;
}) {
  const permissionKey = permissions.join(",");
  const profiles =
    agent.node.policy?.models.map((model) => model.profile) ?? [];
  const models = useRef(profiles);
  models.current = profiles;
  const host = useMemo(
    () =>
      createWorkflowHost(
        project,
        permissionKey.split(",") as Permission[],
        () => models.current,
      ),
    [project, permissionKey, agent.node.node_id],
  );
  const attempts = useRef(new Map<string, ScheduleRunAttempt>());
  const props = {
    targetId: agent.node.node_id,
    workspaceName: workspaceName(agent),
    runtimeReady: Boolean(agent.presence?.ready && !agent.node.revoked),
    onCreateWithAgent,
    agentStarting: busy || !permissions.includes("execute"),
  };
  return (
    <WorkflowHostProvider key={permissionKey} host={host}>
      {view === "workflows" ? (
        <WorkflowsSurface {...props} />
      ) : (
        <SchedulesSurface
          {...props}
          runAttempts={attempts.current}
          attemptScope={project + ":" + agent.node.node_id}
        />
      )}
    </WorkflowHostProvider>
  );
}
