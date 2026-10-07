import { lazy, Suspense, useState } from "react";
import type { RoleModel } from "./ModelRoleRouting";
import "./model-routing-overview.css";

export { modelRouteGroups } from "./model-routing-map";

const ModelRoutingGraph = lazy(() =>
  import("./ModelRoutingGraph").then((module) => ({
    default: module.ModelRoutingGraph,
  })),
);

/** A configuration projection; selecting a route never invokes a provider. */
export function ModelRoutingOverview({
  roles,
  models,
  onChange,
  disabled = false,
}: {
  roles: Record<string, string>;
  models: RoleModel[];
  onChange?: (roles: Record<string, string>) => void;
  disabled?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <details
      className="model-routing-overview"
      onToggle={(event) => setExpanded(event.currentTarget.open)}
    >
      <summary>Routing map</summary>
      {expanded ? (
        <Suspense fallback={<p role="status">Loading routing map…</p>}>
          <ModelRoutingGraph
            roles={roles}
            models={models}
            onChange={onChange}
            disabled={disabled}
          />
        </Suspense>
      ) : null}
    </details>
  );
}
