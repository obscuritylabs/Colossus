import type {
  Membership,
  Permission,
  RuntimePolicyPosture,
  Thread,
} from "./api";
export interface UserIdentity {
  kind: "local" | "oidc";
  label: string;
  username?: string;
  issuer?: string;
  subject?: string;
}
export interface User {
  id: string;
  display_name: string;
  email: string | null;
  active: boolean;
  is_admin: boolean;
  revision: number;
  created_at: string;
  updated_at: string;
  identities?: UserIdentity[];
}
export interface Project {
  id: string;
  name: string;
  description: string;
  parent_project_id: string | null;
  archived: boolean;
  revision: number;
  created_at: string;
  updated_at: string;
}
export interface Member {
  user_id: string;
  project_id: string;
  display_name: string;
  role: ProjectRole;
  permissions: string[];
  revision: number;
  active?: boolean;
}
export type ProjectRole = "viewer" | "operator" | "approver" | "project_admin";
export const projectRoles: {
  value: ProjectRole;
  label: string;
  description: string;
}[] = [
  {
    value: "viewer",
    label: "Viewer",
    description: "Read released history and fleet inventory.",
  },
  {
    value: "operator",
    label: "Operator",
    description: "Read, execute, and control authorized agent work.",
  },
  {
    value: "approver",
    label: "Approver",
    description: "Read and approve actions within the runtime’s grant.",
  },
  {
    value: "project_admin",
    label: "Project administrator",
    description:
      "Operate project work and manage access and its monitoring baseline.",
  },
];
export interface Me {
  user: User;
  memberships: (Membership & { user_id: string; role: ProjectRole })[];
  projects: Project[];
}
/** Mirror management visibility without manufacturing execution or approval rights. */
export function projectPermissions(
  me: Me | null,
  projectId: string,
): Permission[] {
  const project = me?.projects.find((item) => item.id === projectId);
  if (!me || !project) return [];
  const permissions = new Set(
    me.memberships.find((member) => member.project_id === projectId)
      ?.permissions ?? [],
  );
  if (me.user.is_admin) {
    permissions.add("read");
    permissions.add("administer");
  }
  if (project.archived) {
    permissions.delete("execute");
    permissions.delete("control");
    permissions.delete("approve");
  }
  return [...permissions];
}
export interface Classification {
  enabled: boolean;
  text: string;
  tone: "neutral" | "info" | "warning" | "danger";
  position: "top" | "top_and_bottom";
}
export interface PublicSettings {
  classification: Classification;
}
export interface AdminSettings extends PublicSettings {
  revision: number;
  auth: { local_enabled: boolean; oidc_label: string | null };
}
export interface AuthConfig {
  local_enabled: boolean;
  oidc: { label: string; login_url: string } | null;
}
export interface Activity {
  date: string;
  runs: number;
  completed: number;
  failed: number;
  input_tokens?: number | null;
  output_tokens?: number | null;
}
export interface Analytics {
  generated_at: string;
  window_days: number;
  counts: {
    threads: number;
    runs: number;
    completed: number;
    failed: number;
    cancelled: number;
    active: number;
    queued: number;
    agents: number;
    online_agents: number;
    hosts: number;
    interrupted: number;
    outcome_unknown: number;
  };
  activity: Activity[];
  usage: {
    input_tokens: number | null;
    output_tokens: number | null;
    estimated_cost: number | null;
    currency?: string;
    coverage?: string;
  };
  telemetry?: { complete: boolean; description?: string };
}
export interface Dashboard {
  generated_at: string;
  counts: {
    projects: number;
    hosts: number;
    agents: number;
    online_agents: number;
    threads: number;
    active_runs: number;
    queued_tasks: number;
    failed_runs: number;
  };
  recent_threads: { thread: Thread; project_name: string }[];
  activity: Activity[];
  telemetry?: { complete: boolean; description?: string };
}
export interface PolicyBaseline {
  revision: number;
  required_sandbox_profile: string | null;
  allowed_approval_modes: string[];
  allowed_tools: string[] | null;
}
export interface AgentPolicy {
  node_id: string;
  observed_at: number | null;
  last_seen_at: number | null;
  connected: boolean;
  posture: RuntimePolicyPosture | null;
  provenance: string;
  stale: boolean;
  status: "reported" | "unknown" | "unsupported";
  description?: string;
  expectation: PolicyBaseline;
  evaluation: {
    status: "unknown" | "aligned" | "drift";
    findings: { code: string; description: string }[];
  };
}

/** Bounded dashboard period. Transport and credentials remain in the web host. */
export function dashboard(days = 7): string {
  if (!Number.isInteger(days) || days < 1 || days > 90)
    throw new RangeError("Activity days must be between 1 and 90.");
  return `/api/dashboard?days=${days}`;
}
