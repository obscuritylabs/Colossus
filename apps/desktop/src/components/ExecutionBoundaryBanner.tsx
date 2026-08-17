import type { ExecutionBoundary, ManagedRuntimeState } from "../types";

export function managedRuntimeBoundaryActive(state: ManagedRuntimeState) {
  return (
    state === "starting" ||
    state === "ready" ||
    state === "restarting" ||
    state === "stopping"
  );
}

export function executionBoundaryBannerVisible(
  state: ManagedRuntimeState,
  boundary: ExecutionBoundary,
) {
  return managedRuntimeBoundaryActive(state) && boundary === "full_access";
}

export function ExecutionBoundaryBanner({
  active,
  boundary,
}: {
  active: boolean;
  boundary: ExecutionBoundary;
}) {
  if (!active || boundary !== "full_access") {
    return null;
  }

  return (
    <aside
      className="unsafe-execution-banner"
      role="alert"
      aria-label="Unsafe Managed Local execution boundary"
    >
      <strong>Unsafe: Full access</strong>
      <span>
        Managed Local commands can use host files, environment, and network
        without Colossus isolation. Approval mode is separate.
      </span>
    </aside>
  );
}
