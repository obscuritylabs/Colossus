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
  type WatchScope,
  type MobileStreamEvent,
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
    policy: {
      schema_version: 1,
      provenance: "development-preview",
      fingerprint: "sample-policy",
      configuration_revision: 1,
      access_profile: "sample",
      sandbox_backend: "sample",
      sandbox_profile: "workspace",
      boundary_acknowledged: true,
      approval_mode: "ask",
      allowed_roles: ["primary"],
      allowed_tools: ["filesystem.search", "web.search", "mcp.call"],
      capabilities: [
        "goal.create",
        "research.create",
        "agent_runs.model_selection",
      ],
      models: [{ profile: "balanced", label: "Balanced model" }],
      findings: [],
      telemetry: {
        provenance: "sample",
        denied_requests: 0,
        approval_requests: 1,
        outcome_unknown_runs: 0,
      },
    },
  },
  presence: {
    ready: true,
    capabilities: [
      "runtime.model_catalog.v1",
      "agent_runs.model_selection",
      "goal.create",
      "research.create",
      "agent_messages.read.v1",
    ],
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
const sampleActivity = activity.get("task-0")!;
sampleActivity.push(
  {
    run_id: "run-0",
    sequence: 4,
    created_at: now,
    update: {
      tool_activity: {
        call_id: "preview-reconnect",
        tool_name: "filesystem.search",
        state: "started",
        summary: "Inspecting reconnect handling",
        input: "Search the workspace for connection lease handling.",
      },
    },
  },
  {
    run_id: "run-0",
    sequence: 5,
    created_at: now,
    update: {
      tool_activity: {
        call_id: "preview-reconnect",
        tool_name: "filesystem.search",
        state: "completed",
        summary: "Located the reconnect path",
        preview:
          "The sample runtime found a stale lease during the reconnect handshake.",
      },
    },
  },
  {
    run_id: "run-0",
    sequence: 6,
    created_at: now,
    update: {
      reasoning_summary:
        "Checking cancellation before proposing a change to the reconnect path.",
    },
  },
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
const capabilities = [
  "runtime.model_catalog.v1",
  "agent_runs.model_selection",
  "goal.create",
  "research.create",
  "agent_messages.read.v1",
];
const modelProfiles = [
  {
    profile: "balanced",
    model: "Balanced model",
    provider: "open_ai",
    provider_profile: "preview",
    reasoning_effort: "medium",
    default_roles: ["primary"],
  },
  {
    profile: "deep",
    model: "Deep reasoning model",
    provider: "open_ai",
    provider_profile: "preview",
    reasoning_effort: "high",
    default_roles: [],
  },
];
const commandReplies = new Map<string, unknown>();
let eventCursor = 0;
const streamCursors = new Map<string, number>();
function streamCursor(task: Task, event: Update) {
  const key = JSON.stringify([task.task_id, event.run_id, event.sequence]);
  if (!streamCursors.has(key)) streamCursors.set(key, ++eventCursor);
  return streamCursors.get(key)!;
}
// Assign stable sample stream cursors before watchers or submissions are created.
for (const task of tasks)
  for (const event of activity.get(task.task_id) ?? [])
    streamCursor(task, event);
export const previewClient: MobileClient = {
  preview: true,
  async watch(scope: WatchScope, listener: (event: MobileStreamEvent) => void) {
    let active = true,
      pending = false,
      after = Number(scope.after ?? 0);
    const deliver = async () => {
      if (!active || pending) return;
      pending = true;
      try {
        const pendingEvents: { task: Task; update: Update; cursor: number }[] =
          [];
        for (const task of tasks.filter((task) =>
          scope.task
            ? task.task_id === scope.task
            : task.thread_id === scope.thread,
        )) {
          const updates = scope.task
            ? (
                await previewClient.request<{ updates: Update[] }>({
                  kind: "task_updates",
                  project: scope.project,
                  task: task.task_id,
                  after,
                })
              ).updates
            : (activity.get(task.task_id) ?? []);
          for (const update of updates)
            pendingEvents.push({
              task,
              update,
              cursor: scope.task ? update.sequence : streamCursor(task, update),
            });
        }
        pendingEvents.sort((a, b) => a.cursor - b.cursor);
        for (const { task, update, cursor } of pendingEvents) {
          if (cursor <= after || !active) continue;
          after = cursor;
          listener({
            kind: "run_update",
            update: structuredClone({ ...update, task_id: task.task_id }),
            cursor: String(cursor),
          });
        }
      } catch {
        if (active)
          listener({
            kind: "error",
            code: "unavailable",
            message: "Sample activity could not be loaded.",
          });
      } finally {
        pending = false;
      }
    };
    listener({ kind: "connection", connected: true });
    void deliver();
    const timer = setInterval(() => void deliver(), 100);
    return async () => {
      active = false;
      clearInterval(timer);
    };
  },
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
      case "runtime_context":
      case "model_catalog": {
        const node = nodes.find((item) => item.node.node_id === operation.node);
        if (node?.presence?.connection_id !== operation.connection_id)
          throw new ClientFailure(
            "conflict",
            "The runtime connection changed.",
          );
        result = {
          kind: "result",
          connection_id: operation.connection_id,
          value: {
            capabilities,
            ...(operation.kind === "model_catalog"
              ? { model_profiles: modelProfiles }
              : {}),
          },
        };
        break;
      }
      case "respond": {
        const task = tasks.find((item) => item.task_id === operation.task);
        const interaction = task?.snapshot?.pending_interactions.find(
          (item) => item.interaction_id === operation.request.interaction_id,
        );
        if (
          !task ||
          !interaction ||
          interaction.etag !== operation.request.etag
        )
          throw new ClientFailure(
            "conflict",
            "The request changed. Refresh before responding.",
          );
        interaction.status = "answered";
        const command = {
          command_id: operation.mutation_id,
          reply: { kind: "responded" },
        };
        commandReplies.set(command.command_id, command.reply);
        result = { command };
        break;
      }
      case "command":
        result = {
          command: {
            command_id: operation.command,
            reply: commandReplies.get(operation.command) ?? null,
          },
        };
        break;
      case "inspect_inboxes": {
        const task = tasks.find((item) => item.task_id === operation.task);
        const participant = {
          id: "preview-participant",
          root_run_id: task?.run_id ?? "",
          session_id: "preview-session",
          run_id: task?.run_id ?? null,
          parent_id: null,
          subagent_id: null,
          generation: 1,
          open: true,
          closed_reason: null,
          pending_messages: 0,
          pending_bytes: 0,
          created_at: now,
        };
        const reply = {
          kind: "inboxes",
          participants: [participant],
          page: operation.participant_id
            ? { messages: [], next_sequence: 0, has_more: false }
            : null,
        };
        commandReplies.set(operation.request_id, reply);
        result = { command: { command_id: operation.request_id, reply } };
        break;
      }
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
