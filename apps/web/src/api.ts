export type Permission =
  "read" | "execute" | "control" | "approve" | "administer";
export interface Membership {
  subject: string;
  project_id: string;
  permissions: Permission[];
}
export interface Node {
  node_id: string;
  project_id: string;
  instance_id: string;
  label: string;
  certificate_sha256: string;
  roles: string[];
  revoked: boolean;
  revision: number;
  host_id?: string | null;
  workspace_id?: string | null;
  workspace_label?: string | null;
  policy?: RuntimePolicyPosture | null;
  policy_observed_at?: number | null;
}
export interface RuntimePolicyPosture {
  schema_version: number;
  provenance: string;
  fingerprint: string;
  configuration_revision: number | null;
  access_profile: string;
  sandbox_backend: string;
  sandbox_profile: string;
  boundary_acknowledged: boolean;
  approval_mode: string;
  allowed_roles: string[];
  allowed_tools: string[];
  capabilities: string[];
  models: { profile: string; label: string }[];
  findings: { code: string; severity: string }[];
  telemetry: {
    provenance: string;
    denied_requests: number | null;
    approval_requests: number | null;
    outcome_unknown_runs: number | null;
  };
}
export interface Host {
  host_id: string;
  project_id: string;
  label: string;
  platform: string;
  deployment_kind: string;
  revision: number;
  last_seen_at: number | null;
}
export interface Thread {
  thread_id: string;
  project_id: string;
  node_id: string;
  host_id?: string | null;
  workspace_id?: string | null;
  title: string;
  created_at: string;
  updated_at: string;
  revision: number;
  archived: boolean;
  session_id?: string | null;
  sync_status: "current" | "incomplete";
  source: "cloud" | "runtime";
  can_continue: boolean;
  active_task_id?: string | null;
  queued_task_ids?: string[];
}
export interface ThreadMessage {
  message_id: string;
  role: string;
  text: string;
  created_at: string;
  task_id?: string | null;
}
export interface ThreadDetailResponse {
  thread: Thread;
  tasks: Task[];
  messages: ThreadMessage[];
  next_task_cursor?: string | null;
  next_message_cursor?: string | null;
}
export interface FleetNode {
  node: Node;
  presence: {
    ready: boolean;
    capabilities: string[];
    connection_id: string;
  } | null;
}
export interface Interaction {
  interaction_id: string;
  run_id: string;
  kind: "approval" | "user_prompt";
  status: string;
  respondable_by_caller: boolean;
  etag: string;
  content: {
    approval?: {
      action: string;
      resource: string;
      reason: string;
      risk: string | null;
      request_hash: string;
      command_context: {
        justification: string;
        executable: string;
        arguments: string[];
        working_directory: string;
        redacted: boolean;
      } | null;
    };
    user_prompt?: {
      question: string;
      choices: { choice_id: string; label: string }[];
      allow_free_form: boolean;
    };
  };
}
export interface Run {
  run_id: string;
  session_id?: string;
  title: string;
  status: string;
  created_at: string;
  updated_at: string;
  role: string;
  mode: string;
  last_sequence: number;
  terminal: unknown;
}
export interface Task {
  task_id: string;
  project_id: string;
  node_id: string;
  subject: string;
  created_at?: string;
  updated_at?: string;
  request: { input: { text: string }[]; mode: string; role: string };
  run_id: string | null;
  snapshot: { run: Run; pending_interactions: Interaction[] } | null;
  last_sequence: number;
  revision: number;
  dispatch_error?: { message: string; code: string } | null;
  output_limited?: boolean;
  thread_id?: string | null;
  source_read_only?: boolean;
  history_bounded?: boolean;
  history_complete?: boolean;
}
export interface Update {
  run_id: string;
  sequence: number;
  created_at: string;
  update: Record<string, unknown>;
  task_id?: string;
}
export class ApiFailure extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
  }
}
export interface CommandReceipt {
  command_id: string;
  reply: { kind: string; error?: { code: string; message: string } } | null;
}

// A successful HTTP mutation acknowledges the queue. Only the runtime receipt
// confirms the operation; an offline runtime leaves it durably queued.
export async function awaitReceipt(
  taskPath: string,
  command: CommandReceipt,
  signal?: AbortSignal,
): Promise<boolean> {
  const deadline = Date.now() + 10000;
  for (;;) {
    if (command.reply) {
      if (command.reply.kind === "failed")
        throw new Error(
          command.reply.error?.message ?? "The runtime rejected this action.",
        );
      return true;
    }
    if (Date.now() >= deadline) return false;
    await new Promise<void>((resolve) => setTimeout(resolve, 300));
    command = (
      await request<{ command: CommandReceipt }>(
        `${taskPath}/commands/${encodeURIComponent(command.command_id)}`,
        undefined,
        signal,
      )
    ).command;
  }
}
const messages: Record<string, string> = {
  permission_denied:
    "Your session expired or this project permission is unavailable.",
  conflict:
    "The state changed. Refresh and review the current task before retrying.",
  storage:
    "The control plane is temporarily unavailable. Your accepted work remains on its runtime.",
  resource_exhausted:
    "A resource limit was reached. Wait for active work to finish before retrying.",
  invalid_argument: "Check the request fields and try again.",
  not_found: "This resource is no longer available in the selected project.",
};
export async function request<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  method?: "PATCH" | "DELETE",
): Promise<T> {
  const response = await fetch(path, {
    method: method ?? (body === undefined ? "GET" : "POST"),
    credentials: "same-origin",
    headers:
      body === undefined
        ? method === "DELETE"
          ? { "X-Colossus-CSRF": "1" }
          : {}
        : { "Content-Type": "application/json", "X-Colossus-CSRF": "1" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    ...(signal ? { signal } : {}),
  });
  if (!response.ok) {
    // The web host reconciles identity after authorization failures. A project
    // denial may only remove one role; it must not blindly log the user out.
    if (
      (response.status === 401 || response.status === 403) &&
      typeof window !== "undefined" &&
      path.startsWith("/api/") &&
      !["/api/me", "/api/auth/config", "/api/settings"].includes(path)
    ) {
      window.dispatchEvent(new Event("colossus:web:reconcile-identity"));
    }
    const error = (await response
      .json()
      .catch(() => ({ error: "storage" }))) as { error?: string };
    throw new ApiFailure(
      response.status,
      messages[error.error ?? "storage"] ??
        "The request could not be completed.",
    );
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}
export function projectPath(project: string) {
  return `/api/projects/${encodeURIComponent(project)}`;
}
export function taskTitle(task: Task) {
  return (
    task.snapshot?.run.title ||
    task.request.input.map((part) => part.text).join(" ") ||
    "Untitled task"
  );
}
export function taskStatus(task: Task) {
  return task.dispatch_error?.code === "outcome_unknown"
    ? "outcome_unknown"
    : task.dispatch_error
      ? "failed"
      : task.snapshot?.run.status || "queued";
}
export const terminalStatuses = new Set([
  "completed",
  "failed",
  "cancelled",
  "interrupted",
  "outcome_unknown",
]);
export function statusLabel(value: string) {
  return value
    .split("_")
    .map((word) => word[0]?.toUpperCase() + word.slice(1))
    .join(" ");
}
export function visibleOutput(updates: Update[]): string {
  let output = "";
  for (const event of updates) {
    if (typeof event.update.output_delta === "string")
      output += event.update.output_delta;
    const result = event.update.result as { output?: string } | undefined;
    if (result?.output) output = result.output;
  }
  return output;
}
