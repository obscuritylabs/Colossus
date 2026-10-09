import { useEffect, useState } from "react";
import { projectPath, request, type FleetNode } from "./api";

/** Capability facts stay in the host and belong to the exact authenticated connection. */
export function useRunModeCapabilities(
  target: FleetNode | undefined,
  disabled: boolean,
) {
  const scope = target?.presence
    ? JSON.stringify([
        target.node.project_id,
        target.node.node_id,
        target.presence.connection_id,
      ])
    : "";
  const ready = Boolean(target?.presence?.ready && !target.node.revoked);
  const advertisedResearch = Boolean(
    target?.presence?.capabilities.includes("research.create"),
  );
  const advertisedGoal = Boolean(
    target?.presence?.capabilities.includes("goal.create"),
  );
  const [state, setState] = useState({
    scope: "",
    research: false,
    goal: false,
  });
  useEffect(() => {
    if (!target || !ready || disabled || (advertisedResearch && advertisedGoal))
      return;
    const abort = new AbortController();
    void request<{ kind: string; value?: { capabilities?: unknown } }>(
      `${projectPath(target.node.project_id)}/nodes/${encodeURIComponent(target.node.node_id)}/resources`,
      {
        connection_id: target.presence!.connection_id,
        operation: { operation: "context" },
      },
      abort.signal,
    )
      .then((reply) => {
        const capabilities = reply.value?.capabilities;
        if (!abort.signal.aborted)
          setState({
            scope,
            research:
              reply.kind === "result" &&
              Array.isArray(capabilities) &&
              capabilities.includes("research.create"),
            goal:
              reply.kind === "result" &&
              Array.isArray(capabilities) &&
              capabilities.includes("goal.create"),
          });
      })
      .catch(() => {
        if (!abort.signal.aborted)
          setState({ scope, research: false, goal: false });
      });
    return () => abort.abort();
  }, [scope, ready, disabled, advertisedResearch, advertisedGoal]);
  const current = ready && !disabled;
  const resolved = state.scope === scope;
  return {
    research: current && (advertisedResearch || (resolved && state.research)),
    goal: current && (advertisedGoal || (resolved && state.goal)),
  };
}
