import type { ComponentProps } from "react";
import { SchedulesSurface as SharedSchedulesSurface } from "@colossus/ui/automations/SchedulesSurface";
import { WorkflowHostProvider } from "@colossus/ui/automations/host";
import { desktopWorkflowHost } from "../workflow-host";
import "@colossus/ui/styles/automations.css";
import "@colossus/ui/styles/workflows.css";
import "@colossus/ui/styles/workflow-logic.css";
export function SchedulesSurface(
  props: ComponentProps<typeof SharedSchedulesSurface>,
) {
  return (
    <WorkflowHostProvider host={desktopWorkflowHost}>
      <SharedSchedulesSurface {...props} />
    </WorkflowHostProvider>
  );
}
