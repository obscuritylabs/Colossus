/** Development-only, explicitly labeled fixture. Native production uses real server responses. */
import type {
  FleetNode,
  Host,
  Task,
  Thread,
  ThreadDetailResponse,
} from "@colossus/control-plane-client";
import type {
  AuthConfiguration,
  Identity,
  MobileClient,
  Operation,
} from "./client";

export const previewAuth: AuthConfiguration = {
  local_enabled: true,
  oidc: { label: "Organization SSO", login_url: "/auth/login" },
};
const identity: Identity = {
  user: { id: "preview-user", display_name: "Alex Lee", is_admin: false },
  projects: [{ id: "engineering", name: "Engineering", archived: false }],
  memberships: [
    {
      subject: "preview-user",
      project_id: "engineering",
      permissions: ["read", "execute", "control", "approve"],
    },
  ],
};
const names = ["Windows dev", "Linux build", "Mac studio"];
const workspaces = ["Platform", "Colossus", "Release"];
const platforms = ["windows", "linux", "macos"];
const hosts: Host[] = names.map((label, i) => ({
  host_id: `host-${i}`,
  project_id: "engineering",
  label,
  platform: platforms[i]!,
  deployment_kind: "desktop",
  revision: 1,
  last_seen_at: Date.now() / 1000,
}));
const nodes: FleetNode[] = names.map((label, i) => ({
  node: {
    node_id: `node-${i}`,
    project_id: "engineering",
    instance_id: `instance-${i}`,
    label,
    certificate_sha256: "preview",
    roles: ["primary"],
    revoked: false,
    revision: 1,
    host_id: `host-${i}`,
    workspace_label: workspaces[i]!,
    workspace_id: `workspace-${i}`,
  },
  presence: { ready: true, capabilities: [], connection_id: `preview-${i}` },
}));
let threads: Thread[] = [
  "Investigate connection timeout",
  "Review cancellation flow",
  "Prepare release notes",
].map((title, i) => ({
  thread_id: `thread-${i}`,
  project_id: "engineering",
  node_id: `node-${i}`,
  title,
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  revision: 1,
  archived: false,
  session_id: `session-${i}`,
  sync_status: "current",
  source: "cloud",
  can_continue: true,
  active_task_id: i === 0 ? "task-0" : null,
}));
const tasks: Task[] = [
  {
    task_id: "task-1",
    project_id: "engineering",
    node_id: "node-1",
    subject: "preview-user",
    request: {
      input: [{ text: "Review cancellation flow" }],
      role: "primary",
      mode: "execute",
    },
    run_id: "run-1",
    last_sequence: 1,
    revision: 1,
    thread_id: "thread-1",
    snapshot: {
      run: {
        run_id: "run-1",
        title: "Review cancellation flow",
        status: "waiting_for_input",
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        role: "primary",
        mode: "execute",
        last_sequence: 1,
        terminal: null,
      },
      pending_interactions: [
        {
          interaction_id: "interaction-1",
          run_id: "run-1",
          kind: "approval",
          status: "pending",
          respondable_by_caller: true,
          etag: "preview",
          content: {
            approval: {
              action: "filesystem.write",
              resource: "cancellation.rs",
              reason:
                "Review the proposed cancellation fix before writing the file.",
              risk: "low",
              request_hash: "preview",
              command_context: null,
            },
          },
        },
      ],
    },
  },
];
const messages = new Map<string, ThreadDetailResponse["messages"]>(
  threads.map((t) => [
    t.thread_id,
    [
      {
        message_id: `${t.thread_id}-user`,
        role: "user",
        text: "Review the cancellation flow and suggest a fix.",
        created_at: t.created_at,
      },
      {
        message_id: `${t.thread_id}-assistant`,
        role: "assistant",
        text: "I found a race between cancellation and queued work. The fix is ready for review.",
        created_at: t.created_at,
      },
    ],
  ]),
);
export const previewClient: MobileClient = {
  preview: true,
  async connect() {
    return structuredClone(previewAuth);
  },
  async login() {
    return structuredClone(identity);
  },
  async request<T>(op: Operation): Promise<T> {
    let result: unknown;
    switch (op.kind) {
      case "me":
        result = identity;
        break;
      case "fleet":
        result = { nodes, next_cursor: null };
        break;
      case "hosts":
        result = { hosts, next_cursor: null };
        break;
      case "threads":
        result = { threads, next_cursor: null };
        break;
      case "tasks":
        result = { tasks };
        break;
      case "thread":
        result = {
          thread: threads.find((t) => t.thread_id === op.thread),
          tasks,
          messages: messages.get(op.thread) ?? [],
        };
        break;
      case "create_thread": {
        const t: Thread = {
          ...threads[0]!,
          thread_id: crypto.randomUUID(),
          node_id: op.node,
          title: op.request.input[0]?.text.slice(0, 70) || "New task",
          session_id: crypto.randomUUID(),
          active_task_id: null,
        };
        threads = [t, ...threads];
        messages.set(t.thread_id, [
          {
            message_id: crypto.randomUUID(),
            role: "user",
            text: op.request.input[0]?.text ?? "",
            created_at: new Date().toISOString(),
          },
        ]);
        result = { thread: t, task: tasks[0] };
        break;
      }
      case "send_message": {
        const t = threads.find((t) => t.thread_id === op.thread)!;
        t.revision++;
        messages.get(op.thread)?.push({
          message_id: crypto.randomUUID(),
          role: "user",
          text: op.request.input[0]?.text ?? "",
          created_at: new Date().toISOString(),
        });
        result = { thread: t, task: tasks[0] };
        break;
      }
      case "cancel":
        result = {
          command: { command_id: "preview-cancellation", reply: null },
        };
        break;
    }
    return structuredClone(result) as T;
  },
  async logout() {},
  async openBrowser() {
    throw new Error(
      "Browser actions are unavailable in the sample-data preview.",
    );
  },
};
