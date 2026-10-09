import { useEffect, useState } from "react";
import { projectPath, request, type FleetNode } from "./api";

/** Capability facts stay in the host and belong to the exact authenticated connection. */
export function useResearchCapability(
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
  const advertised = Boolean(
    target?.presence?.capabilities.includes("research.create"),
  );
  const resources = Boolean(
    target?.presence?.capabilities.includes("runtime.resources.v1"),
  );
  const [state, setState] = useState({ scope: "", enabled: false });
  useEffect(() => {
    if (!target || !ready || disabled || !resources || advertised) return;
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
            enabled:
              reply.kind === "result" &&
              Array.isArray(capabilities) &&
              capabilities.includes("research.create"),
          });
      })
      .catch(() => {
        if (!abort.signal.aborted) setState({ scope, enabled: false });
      });
    return () => abort.abort();
  }, [scope, ready, disabled, resources, advertised]);
  return (
    ready &&
    !disabled &&
    (advertised || (resources && state.scope === scope && state.enabled))
  );
}
