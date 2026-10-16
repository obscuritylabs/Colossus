/** Development-only, explicitly labeled sample data. No provider or production controller is called. */
import type {
  FleetNode,
  Host,
  Task,
  Thread,
  ThreadDetailResponse,
  Update,
} from "@colossus/control-plane-client";
import {
  ClientFailure,
  type AuthConfiguration,
  type Identity,
  type MobileClient,
  type Operation,
  type RunRequest,
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
const names = ["Windows dev", "Linux build", "Mac studio"],
  workspaces = ["Platform", "Colossus", "Release"],
  platforms = ["windows", "linux", "macos"];
const now = new Date().toISOString();
const hosts: Host[] = names.map((label, index) => ({
  host_id: `host-${index}`,
  project_id: "engineering",
  label,
  platform: platforms[index]!,
  deployment_kind: "desktop",
  revision: 1,
  last_seen_at: Date.now() / 1000,
}));
const nodes: FleetNode[] = names.map((label, index) => ({
  node: {
    node_id: `node-${index}`,
    project_id: "engineering",
    instance_id: `instance-${index}`,
    label,
    certificate_sha256: "preview",
    roles: ["primary"],
    revoked: false,
    revision: 1,
    host_id: `host-${index}`,
    workspace_label: workspaces[index]!,
    workspace_id: `workspace-${index}`,
  },
  presence: {
    ready: true,
    capabilities: [],
    connection_id: `preview-${index}`,
  },
}));
const titles = [
  "Investigate connection timeout",
  "Review cancellation flow",
  "Prepare release notes",
];
const prompts = [
  "Investigate the intermittent connection timeout after a host reconnects.",
  "Review the cancellation flow and suggest a fix before changing files.",
  "Prepare the mobile release notes with testing and migration notes.",
];
const replies = [
  "I traced the timeout to a reconnect handshake that can retain a stale connection lease.\n\n- The retry keeps its original task identity.\n- The host is still connected.\n- I’m checking the cancellation path before proposing a change.",
  "I found a race between cancellation and queued work.\n\nThe proposed change checks cancellation before moving a task from `queued` to `running`. I’m waiting for your approval before writing the patch.",
  "## Release notes ready\n\n- Added mobile conversations, task history, and host status.\n- Local sign-in keeps sessions in native memory.\n- Existing runtime grants still control every task.\n\n**Migration:** no runtime data migration is required. Native distribution and device acceptance remain release prerequisites.",
];
const states = ["running", "waiting_for_input", "completed"];
const threads: Thread[] = titles.map((title, index) => ({
  thread_id: `thread-${index}`,
  project_id: "engineering",
  node_id: `node-${index}`,
  title,
  created_at: now,
  updated_at: now,
  revision: 1,
  archived: false,
  session_id: `session-${index}`,
  sync_status: "current",
  source: "cloud",
  can_continue: true,
  active_task_id: index < 2 ? `task-${index}` : null,
}));
const tasks: Task[] = titles.map((title, index) => ({
  task_id: `task-${index}`,
  project_id: "engineering",
  node_id: `node-${index}`,
  subject: "preview-user",
  request: {
    input: [{ text: prompts[index]! }],
    role: "primary",
    mode: "execute",
  },
  run_id: `run-${index}`,
  last_sequence: index === 2 ? 4 : 3,
  revision: 1,
  thread_id: `thread-${index}`,
  created_at: now,
  updated_at: now,
  snapshot: {
    run: {
      run_id: `run-${index}`,
      title,
      status: states[index]!,
      created_at: now,
      updated_at: now,
      role: "primary",
      mode: "execute",
      last_sequence: index === 2 ? 4 : 3,
      terminal: index === 2 ? { output: replies[index] } : null,
    },
    pending_interactions:
      index === 1
        ? [
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
          ]
        : [],
  },
}));
const messages = new Map<string, ThreadDetailResponse["messages"]>(
  threads.map((thread, index) => [
    thread.thread_id,
    [
      {
        message_id: `${thread.thread_id}-user`,
        task_id: `task-${index}`,
        role: "user",
        text: prompts[index]!,
        created_at: now,
      },
      {
        message_id: `${thread.thread_id}-assistant`,
        task_id: `task-${index}`,
        role: "assistant",
        text: replies[index]!,
        created_at: now,
      },
    ],
  ]),
);
const activity = new Map<string, Update[]>(
  tasks.map((task, index) => [
    task.task_id,
    [
      {
        run_id: task.run_id!,
        sequence: 1,
        created_at: now,
        update: { created: {} },
      },
      {
        run_id: task.run_id!,
        sequence: 2,
        created_at: now,
        update: { started: {} },
      },
      {
        run_id: task.run_id!,
        sequence: 3,
        created_at: now,
        update: { output_delta: replies[index]! },
      },
      ...(index === 2
        ? [
            {
              run_id: task.run_id!,
              sequence: 4,
              created_at: now,
              update: { result: { output: replies[index]! } },
            },
          ]
        : []),
    ],
  ]),
);
function complete(
  thread: Thread,
  request: RunRequest,
): { thread: Thread; task: Task } {
  const id = crypto.randomUUID(),
    stamp = new Date().toISOString();
  const output =
    "This sample task is complete. In the installed app, this view shows output released by the selected runtime.";
  const task: Task = {
    task_id: id,
    thread_id: thread.thread_id,
    project_id: thread.project_id,
    node_id: thread.node_id,
    subject: "preview-user",
    request,
    run_id: `run-${id}`,
    last_sequence: 4,
    revision: 1,
    created_at: stamp,
    updated_at: stamp,
    snapshot: {
      run: {
        run_id: `run-${id}`,
        title: request.input[0]?.text.slice(0, 100) || "Preview task",
        status: "completed",
        created_at: stamp,
        updated_at: stamp,
        role: request.role,
        mode: request.mode,
        last_sequence: 4,
        terminal: { output },
      },
      pending_interactions: [],
    },
  };
  tasks.unshift(task);
  thread.updated_at = stamp;
  const feed = messages.get(thread.thread_id) ?? [];
  messages.set(thread.thread_id, feed);
  feed.push(
    {
      message_id: crypto.randomUUID(),
      task_id: id,
      role: "user",
      text: request.input[0]?.text ?? "",
      created_at: stamp,
    },
    {
      message_id: crypto.randomUUID(),
      task_id: id,
      role: "assistant",
      text: output,
      created_at: stamp,
    },
  );
  activity.set(id, [
    {
      run_id: task.run_id!,
      sequence: 1,
      created_at: stamp,
      update: { created: {} },
    },
    {
      run_id: task.run_id!,
      sequence: 2,
      created_at: stamp,
      update: { started: {} },
    },
    {
      run_id: task.run_id!,
      sequence: 3,
      created_at: stamp,
      update: { output_delta: output },
    },
    {
      run_id: task.run_id!,
      sequence: 4,
      created_at: stamp,
      update: { result: { output } },
    },
  ]);
  return { thread, task };
}
export const previewClient: MobileClient = {
  preview: true,
  async connect() {
    return structuredClone(previewAuth);
  },
  async login(username) {
    if (username === "invalid")
      throw new ClientFailure(
        "login_failed",
        "Sign-in failed. Check your credentials or try again later.",
      );
    return structuredClone(identity);
  },
  async request<T>(operation: Operation): Promise<T> {
    let result: unknown;
    switch (operation.kind) {
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
        result = { tasks, next_cursor: null };
        break;
      case "thread":
        result = {
          thread: threads.find(
            (thread) => thread.thread_id === operation.thread,
          ),
          tasks: tasks.filter((task) => task.thread_id === operation.thread),
          messages: messages.get(operation.thread) ?? [],
        };
        break;
      case "task":
        result = {
          task: tasks.find((task) => task.task_id === operation.task),
        };
        break;
      case "task_updates": {
        const updates = (activity.get(operation.task) ?? []).filter(
          (event) => event.sequence > operation.after,
        );
        result = {
          updates: updates.slice(0, 32),
          next_after: updates.length > 32 ? updates[31]!.sequence : null,
        };
        break;
      }
      case "create_thread": {
        const thread: Thread = {
          ...threads[0]!,
          thread_id: crypto.randomUUID(),
          node_id: operation.node,
          title: operation.request.input[0]?.text.slice(0, 70) || "New task",
          session_id: crypto.randomUUID(),
          active_task_id: null,
        };
        threads.unshift(thread);
        result = complete(thread, operation.request);
        break;
      }
      case "send_message": {
        const thread = threads.find(
          (thread) => thread.thread_id === operation.thread,
        )!;
        thread.revision++;
        result = complete(thread, operation.request);
        break;
      }
      case "cancel": {
        const task = tasks.find((task) => task.task_id === operation.task);
        if (task?.snapshot) {
          task.snapshot.run.status = "cancelled";
          task.snapshot.pending_interactions.forEach((item) => {
            item.status = "cancelled";
          });
        }
        result = {
          command: { command_id: "preview-cancellation", reply: null },
        };
        break;
      }
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
