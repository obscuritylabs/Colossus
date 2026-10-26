import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { IconTerminal2 } from "@tabler/icons-react";
import type { FleetNode, Permission } from "./api";
import "./fleet-terminal.css";

const FleetTerminal = lazy(() => import("./FleetTerminal"));

/** Presence is an authenticated advertisement, never a grant of authority. */
export function fleetShellAvailable(
  project: string,
  agent: FleetNode | undefined,
  permissions: readonly Permission[],
) {
  return Boolean(
    permissions.includes("terminal") &&
    agent?.node.project_id === project &&
    !agent.node.revoked &&
    agent.node.execution_kind !== "hosted" &&
    agent.presence?.ready &&
    agent.presence.capabilities.includes("human_terminal.v1"),
  );
}

export function FleetShell({
  project,
  agent,
  permissions,
}: {
  project: string;
  agent: FleetNode | undefined;
  permissions: Permission[];
}) {
  const [openedScope, setOpenedScope] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const available = fleetShellAvailable(project, agent, permissions);
  const scope = JSON.stringify([
    project,
    agent?.node.node_id,
    agent?.node.workspace_id,
  ]);
  useEffect(() => {
    setOpenedScope((opened) => (available && opened === scope ? opened : ""));
  }, [available, scope]);
  if (!available || !agent) return null;
  return (
    <>
      <Button
        ref={trigger}
        variant="secondary"
        onClick={() => setOpenedScope(scope)}
      >
        <IconTerminal2 size={16} aria-hidden="true" />
        Open Shell
      </Button>
      {openedScope === scope ? (
        <Suspense fallback={<p role="status">Loading shell…</p>}>
          <FleetTerminal
            key={scope}
            project={project}
            node={agent.node.node_id}
            workspace={agent.node.workspace_label ?? agent.node.label}
            scope={scope}
            onDetach={() => {
              setOpenedScope("");
              requestAnimationFrame(() => trigger.current?.focus());
            }}
          />
        </Suspense>
      ) : null}
    </>
  );
}
