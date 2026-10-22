import type { FleetNode } from "./api";

export interface HostedTarget extends FleetNode {
  node: FleetNode["node"] & { owner_user_id: string };
  execution: {
    kind: "hosted";
    state: "warm" | "on_demand";
    environment: null;
    managed_files: boolean;
    filesystem: boolean;
    process: boolean;
  };
}
export interface HostedActivity {
  queued: number;
  running: number;
  waiting: number;
  limited: boolean;
}
export interface HostedInventory {
  enabled: boolean;
  target: HostedTarget | null;
  activity?: HostedActivity;
}
export function hostedStatus(activity: HostedActivity) {
  if (activity.waiting) return "Waiting";
  if (activity.running) return "Running";
  if (activity.queued) return "Queued";
  return activity.limited ? "Busy" : "Idle";
}
