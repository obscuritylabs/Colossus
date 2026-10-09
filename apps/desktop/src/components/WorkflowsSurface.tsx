import type { ComponentProps } from "react";
import { WorkflowsSurface as SharedWorkflowsSurface } from "@colossus/ui/automations/WorkflowsSurface";
import { WorkflowHostProvider } from "@colossus/ui/automations/host";
import { desktopWorkflowHost } from "../workflow-host";
import "@colossus/ui/styles/automations.css";
import "@colossus/ui/styles/workflows.css";
import "@colossus/ui/styles/workflow-logic.css";
export function WorkflowsSurface(
  props: ComponentProps<typeof SharedWorkflowsSurface>,
) {
  return (
    <WorkflowHostProvider host={desktopWorkflowHost}>
      <SharedWorkflowsSurface {...props} />
    </WorkflowHostProvider>
  );
}
