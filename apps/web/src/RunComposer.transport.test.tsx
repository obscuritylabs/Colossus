// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AuthenticatedApp } from "./ControlPlane";
import { NavigationProvider } from "./navigation";
import { Threads } from "./Threads";
import { ThreadDetail } from "./ThreadDetail";
import type { FleetNode, Task, Thread, ThreadDetailResponse } from "./api";
import type { Me } from "./control-api";

const now = "2026-10-16T10:00:00Z";
const capabilities = [
  "runtime.resources.v1",
  "runtime.model_catalog.v1",
  "agent_runs.model_selection",
  "research.create",
  "goal.create",
];
const node: FleetNode = {
  node: {
    node_id: "node-a",
    project_id: "project-a",
    instance_id: "instance-a",
    label: "Development workstation",
    certificate_sha256: "fixture",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: { ready: true, capabilities, connection_id: "connection-a" },
};
const thread: Thread = {
  thread_id: "thread-a",
  project_id: "project-a",
  node_id: "node-a",
  session_id: "session-a",
  title: "Saved conversation",
  created_at: now,
  updated_at: now,
  revision: 7,
  archived: false,
  source: "cloud",
  sync_status: "current",
  can_continue: true,
};
const task: Task = {
  task_id: "task-a",
  project_id: "project-a",
  node_id: "node-a",
  subject: "human",
  request: {
    input: [{ text: "Work on the request" }],
    mode: "execute",
    role: "primary",
  },
  revision: 1,
  run_id: null,
  last_sequence: 0,
  snapshot: null,
};
const me: Me = {
  user: {
    id: "operator-a",
    display_name: "Operator",
    email: null,
    active: true,
    is_admin: false,
    revision: 1,
    created_at: now,
    updated_at: now,
  },
  memberships: [
    {
      project_id: "project-a",
      user_id: "operator-a",
      subject: "operator-a",
      role: "operator",
      permissions: ["read", "execute"],
    },
  ],
  projects: [
    {
      id: "project-a",
      name: "Test project",
      description: "",
      parent_project_id: null,
      archived: false,
      revision: 1,
      created_at: now,
      updated_at: now,
    },
  ],
};
class EventSourceFixture extends EventTarget {
  onopen = null;
  onerror = null;
  close() {}
}
interface CapturedRequest {
  path: string;
  options: RequestInit;
  body: {
    node_id?: string;
    expected_revision?: number;
    request?: {
      model_profile?: string;
      reasoning_effort?: string;
      input: { text: string }[];
      idempotency_key: string;
    };
  };
}
let root: Root, container: HTMLDivElement;
let submissions: CapturedRequest[], revision: number, conflictOnce: boolean;
const base = "/api/projects/project-a";
const response = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status });
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(null, "", "/");
  window.localStorage?.clear();
  window.sessionStorage?.clear();
  vi.stubGlobal("EventSource", EventSourceFixture);
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  submissions = [];
  revision = thread.revision;
  conflictOnce = false;
  // Keep every application callback and the HTTP serializer real. Only the
  // server's resource/catalog and queue responses are fixtures.
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit = {}) => {
      if (path === `${base}/nodes/node-a/resources`) {
        const body = JSON.parse(String(options.body));
        expect(body.connection_id).toBe("connection-a");
        expect(["context", "model_catalog"]).toContain(
          body.operation.operation,
        );
        return response({
          connection_id: "connection-a",
          kind: "result",
          value:
            body.operation.operation === "context"
              ? { capabilities }
              : {
                  model_profiles: [
                    {
                      profile: "careful",
                      model: "Careful model",
                      provider: "Test provider",
                      provider_profile: "test",
                      reasoning_effort: "high",
                      default_roles: ["primary"],
                    },
                  ],
                },
        });
      }
      if (
        options.method === "POST" &&
        [
          `${base}/tasks`,
          `${base}/threads`,
          `${base}/threads/thread-a/messages`,
        ].includes(path)
      ) {
        submissions.push({
          path,
          options,
          body: JSON.parse(String(options.body)),
        });
        if (conflictOnce && path.endsWith("/messages")) {
          conflictOnce = false;
          revision++;
          return response({ error: "conflict" }, 409);
        }
        return response({ thread: { ...thread, revision }, task });
      }
      if (path === `${base}/nodes/node-a`) return response(node);
      if (path.startsWith(`${base}/nodes?`))
        return response({ nodes: [node], next_cursor: null });
      if (path.startsWith(`${base}/hosts?`))
        return response({ hosts: [], next_cursor: null });
      if (path.startsWith(`${base}/tasks?`))
        return response({ tasks: [], next_cursor: null });
      if (path.startsWith(`${base}/threads?`))
        return response({ threads: [], next_cursor: null });
      if (path === `${base}/threads/thread-a`)
        return response({
          thread: { ...thread, revision },
          tasks: [],
          messages: [],
        } satisfies ThreadDetailResponse);
      if (path === `${base}/tasks/task-a`) return response({ task });
      if (path.startsWith(`${base}/tasks/task-a/updates`))
        return response({ updates: [] });
      throw new Error(`Unexpected HTTP request: ${options.method} ${path}`);
    }),
  );
  // AuthenticatedApp retains its production lazy view import. Resolve that
  // module before bounded UI waits rather than relying on transform speed.
  await import("./ThreadDetail");
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  history.replaceState(null, "", "/");
  vi.unstubAllGlobals();
});
function button(label: string, scope: ParentNode = document) {
  const result = [...scope.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label,
  );
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function settled(assertion: () => void) {
  await vi.waitFor(
    async () => {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 10));
      });
      assertion();
    },
    { timeout: 2000 },
  );
}
async function selectModelAndReasoning() {
  await settled(() =>
    expect(
      container.querySelector('[aria-label^="Choose model and reasoning:"]'),
    ).not.toBeNull(),
  );
  await act(() =>
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label^="Choose model and reasoning:"]',
      )!
      .click(),
  );
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find(
    (item) => item.querySelector("strong")?.textContent === "Careful model",
  );
  expect(option).toBeDefined();
  await act(() => option!.click());
  const slider = document.querySelector<HTMLElement>('[role="slider"]');
  expect(slider).not.toBeNull();
  await act(() =>
    slider!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Home", bubbles: true }),
    ),
  );
  for (let step = 0; step < 3; step++)
    await act(() =>
      slider!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      ),
    );
  expect(slider!.getAttribute("aria-valuetext")).toBe("low");
  await act(() => button("Done").click());
}
async function send() {
  const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(textarea).not.toBeNull();
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(textarea, "Work on the request");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    textarea
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}
type EntryPoint = "task" | "agent" | "inventory" | "continuation";
async function renderEntryPoint(entry: EntryPoint) {
  if (entry === "task" || entry === "agent") {
    history.replaceState(
      null,
      "",
      entry === "task"
        ? "/projects/project-a/tasks"
        : "/projects/project-a/agents/node-a/threads?compose=1",
    );
    await act(async () =>
      root.render(
        <NavigationProvider>
          <AuthenticatedApp
            me={me}
            authConfig={null}
            settings={null}
            onSettings={() => {}}
            loadMe={() => {}}
            onSignOut={() => {}}
          />
        </NavigationProvider>,
      ),
    );
    if (entry === "task") {
      await settled(() => expect(button("New task").disabled).toBe(false));
      await act(() => button("New task").click());
    }
  } else if (entry === "inventory") {
    await act(async () =>
      root.render(
        <Threads
          project="project-a"
          nodes={[node]}
          permissions={["read", "execute"]}
          agentId="node-a"
          onAgentChange={() => {}}
          onOpen={() => {}}
          onError={(error) => {
            throw new Error(error);
          }}
          onFleet={() => {}}
        />,
      ),
    );
    await act(() => button("New thread").click());
  } else {
    await act(async () =>
      root.render(
        <ThreadDetail
          initial={thread}
          project="project-a"
          nodes={[node]}
          permissions={["read", "execute"]}
          onBack={() => {}}
          onChanged={() => {}}
        />,
      ),
    );
  }
}
it.each<{
  label: string;
  entry: EntryPoint;
  suffix: string;
}>([
  { label: "standalone task", entry: "task", suffix: "/tasks" },
  { label: "agent conversation", entry: "agent", suffix: "/threads" },
  { label: "thread inventory", entry: "inventory", suffix: "/threads" },
  {
    label: "saved conversation continuation",
    entry: "continuation",
    suffix: "/threads/thread-a/messages",
  },
])(
  "forwards chosen model and reasoning through the $label HTTP callback",
  async ({ entry, suffix }) => {
    await renderEntryPoint(entry);
    await selectModelAndReasoning();
    await send();
    expect(submissions).toHaveLength(1);
    expect(submissions[0]!.path).toBe(`${base}${suffix}`);
    expect(submissions[0]!.options).toMatchObject({
      method: "POST",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        "X-Colossus-CSRF": "1",
      },
    });
    expect(submissions[0]!.body.request).toMatchObject({
      input: [{ text: "Work on the request" }],
      model_profile: "careful",
      reasoning_effort: "low",
      idempotency_key: expect.any(String),
    });
    if (entry === "continuation")
      expect(submissions[0]!.body.expected_revision).toBe(7);
    else expect(submissions[0]!.body.node_id).toBe("node-a");
  },
);
it("preserves chosen model, reasoning, and request identity when continuation reconciles a revision conflict", async () => {
  conflictOnce = true;
  await renderEntryPoint("continuation");
  await selectModelAndReasoning();
  await send();
  expect(submissions).toHaveLength(2);
  expect(
    submissions.map((submission) => submission.body.expected_revision),
  ).toEqual([7, 8]);
  expect(submissions[0]!.path).toBe(`${base}/threads/thread-a/messages`);
  expect(submissions[1]!.path).toBe(submissions[0]!.path);
  expect(submissions[1]!.body.request).toEqual(submissions[0]!.body.request);
  expect(submissions[1]!.body.request).toMatchObject({
    model_profile: "careful",
    reasoning_effort: "low",
    idempotency_key: expect.any(String),
  });
});
