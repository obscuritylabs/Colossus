import type { FleetNode, Host } from "./api";

export function workspaceName(item: FleetNode): string {
  return item.node.workspace_label || item.node.label || "Workspace";
}
export function platformName(platform: string): string {
  return (
    (
      { macos: "macOS", linux: "Linux", windows: "Windows" } as Record<
        string,
        string
      >
    )[platform] ||
    platform ||
    "OS not reported"
  );
}
export function workspaceState(item: FleetNode): string {
  if (item.node.revoked) return "Revoked";
  return item.presence?.ready
    ? "Ready"
    : item.presence
      ? "Starting"
      : "Offline";
}
export function hostWorkspaces(
  nodes: FleetNode[],
  project: string,
  hostId: string,
): FleetNode[] {
  return nodes
    .filter(
      (item) =>
        item.node.project_id === project && item.node.host_id === hostId,
    )
    .sort(
      (a, b) =>
        workspaceName(a).localeCompare(workspaceName(b)) ||
        a.node.node_id.localeCompare(b.node.node_id),
    );
}
export function hostConnection(
  host: Host,
  nodes: FleetNode[],
): { ready: number; total: number; label: string } {
  const items = hostWorkspaces(nodes, host.project_id, host.host_id).filter(
    (item) => !item.node.revoked,
  );
  const ready = items.filter((item) => item.presence?.ready).length;
  return {
    ready,
    total: items.length,
    label: ready
      ? "Connected"
      : items.some((item) => item.presence)
        ? "Connecting"
        : "Offline",
  };
}

/** Desktop destination IDs remain presentation keys; web URLs own their route names. */
export const WORKSPACE_VIEW_ROUTES = {
  work: "threads",
  fleet: "capabilities",
  plugins: "plugins",
  workflows: "workflows",
  schedules: "schedules",
  library: "library",
  connections: "connections",
  settings: "policy",
} as const;
