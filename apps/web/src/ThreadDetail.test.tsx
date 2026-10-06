// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ThreadDetail } from "./ThreadDetail";
import type {
  FleetNode,
  Permission,
  Thread,
  ThreadDetailResponse,
  Task,
  Update,
} from "./api";

const thread: Thread = {
  thread_id: "thread-a",
  project_id: "project-a",
  node_id: "node-a",
  session_id: "session-a",
  title: "Investigate the deployment",
  created_at: "2026-10-05T10:00:00Z",
  updated_at: "2026-10-05T10:00:00Z",
  revision: 7,
  archived: false,
  sync_status: "current",
  source: "cloud",
  can_continue: true,
};
const node: FleetNode = {
  node: {
    node_id: "node-a",
    project_id: "project-a",
    instance_id: "instance-a",
    label: "Development workstation",
    roles: ["primary"],
    certificate_sha256: "certificate",
    revoked: false,
    revision: 1,
  },
  presence: { ready: true, capabilities: [], connection_id: "connection-a" },
};
const history: ThreadDetailResponse = {
  thread,
  tasks: [],
  messages: [
    {
      message_id: "message-b",
      role: "assistant",
      text: "Saved assistant reply",
      created_at: "2026-10-05T10:01:00Z",
      task_id: "task-a",
    },
  ],
};
class TestEventSource extends EventTarget {
  static instances: TestEventSource[] = [];
  constructor() {
    super();
    TestEventSource.instances.push(this);
  }
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  close() {}
}
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  TestEventSource.instances = [];
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("EventSource", TestEventSource);
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
const response = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status });
async function render(
  initial = thread,
  nodes = [node],
  permissions: Permission[] = ["read", "execute", "control", "approve"],
) {
  await act(async () => {
    root.render(
      <ThreadDetail
        initial={initial}
        project="project-a"
        nodes={nodes}
        permissions={permissions}
        onBack={() => {}}
        onChanged={() => {}}
      />,
    );
  });
}
function button(label: string) {
  const result = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find(
    (item) =>
      item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label,
  );
  if (!result) throw new Error(`Missing button ${label}`);
  return result;
}
async function write(value: string) {
  const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
describe("human cloud conversation authority", () => {
  it("exposes a real Back permalink while plain clicks preserve the host callback and modified clicks remain native", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => Promise.resolve(response(history))),
    );
    const onBack = vi.fn();
    await act(async () =>
      root.render(
        <ThreadDetail
          initial={thread}
          project="project-a"
          nodes={[node]}
          permissions={["read"]}
          onBack={onBack}
          onChanged={() => {}}
          backHref="/projects/project-a/agents/node-a/overview"
        />,
      ),
    );
    const link = container.querySelector<HTMLAnchorElement>("a.back")!;
    expect(link.getAttribute("href")).toBe(
      "/projects/project-a/agents/node-a/overview",
    );
    const modified = new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
      ctrlKey: true,
    });
    link.dispatchEvent(modified);
    expect(modified.defaultPrevented).toBe(false);
    expect(onBack).not.toHaveBeenCalled();
    const plain = new MouseEvent("click", { bubbles: true, cancelable: true });
    await act(() => link.dispatchEvent(plain));
    expect(plain.defaultPrevented).toBe(true);
    expect(onBack).toHaveBeenCalledTimes(1);
  });
  it("renders each human turn, its exact run's activity, and response together rather than appending activity", async () => {
    const task = (id: string, created: string): Task => ({
      task_id: id,
      project_id: "project-a",
      node_id: "node-a",
      subject: "human",
      created_at: created,
      request: {
        input: [{ text: `${id} request` }],
        mode: "execute",
        role: "primary",
      },
      run_id: `run-${id}`,
      revision: 1,
      last_sequence: 3,
      snapshot: {
        run: {
          run_id: `run-${id}`,
          title: "Saved turn",
          status: "completed",
          created_at: created,
          updated_at: created,
          role: "primary",
          mode: "execute",
          last_sequence: 3,
          terminal: { result: { elapsed_seconds: 2 } },
        },
        pending_interactions: [],
      },
    });
    const first = task("first", "2026-10-05T10:00:00Z"),
      second = task("second", "2026-10-05T10:02:00Z");
    const detail = {
      ...history,
      tasks: [second, first],
      messages: [first, second].flatMap((task) => [
        {
          message_id: `${task.task_id}-user`,
          task_id: task.task_id,
          role: "user",
          text: `${task.task_id} request`,
          created_at: task.created_at!,
        },
        {
          message_id: `${task.task_id}-assistant`,
          task_id: task.task_id,
          role: "assistant",
          text: `${task.task_id} reply`,
          created_at: task.created_at!,
        },
      ]),
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => Promise.resolve(response(detail))),
    );
    await render();
    const activity = (task: Task, sequence: number, state: string): Update => ({
      task_id: task.task_id,
      run_id: task.run_id!,
      sequence,
      created_at: task.created_at!,
      update: {
        tool_activity: {
          call_id: "same-call",
          tool_name: "echo",
          state,
          summary: `${task.task_id} ${state}`,
          preview: `${task.task_id} released preview`,
        },
      },
    });
    await act(() => {
      for (const update of [
        activity(second, 2, "completed"),
        activity(first, 1, "requested"),
        { ...activity(second, 1, "requested"), task_id: first.task_id },
        activity(second, 1, "requested"),
        activity(first, 2, "completed"),
        activity(first, 2, "completed"),
      ])
        TestEventSource.instances[0]!.dispatchEvent(
          new MessageEvent("run_update", { data: JSON.stringify(update) }),
        );
    });
    const turns = [
      ...container.querySelectorAll<HTMLElement>(".shared-conversation-turn"),
    ];
    expect(turns.map((turn) => turn.dataset.taskId)).toEqual([
      "first",
      "second",
    ]);
    for (const [index, turn] of turns.entries()) {
      const owner = index === 0 ? "first" : "second",
        other = index === 0 ? "second" : "first";
      expect(turn.dataset.runId).toBe(`run-${owner}`);
      expect(
        [...turn.children].map(
          (child) => child.getAttribute("data-role") ?? child.className,
        ),
      ).toEqual(["user", "conversation-run", "assistant"]);
      expect(turn.querySelectorAll(".shared-activity-tool")).toHaveLength(1);
      expect(
        turn.querySelector(".shared-conversation-activity")?.textContent,
      ).toContain("1 action");
      expect(
        turn.querySelector(".shared-activity-history")?.textContent,
      ).toContain(`${owner} completed`);
      expect(
        turn.querySelector(".shared-activity-history")?.textContent,
      ).not.toContain(`${other} completed`);
      expect(turn.querySelectorAll(".shared-activity-history li")).toHaveLength(
        2,
      );
    }
    expect(
      container
        .querySelector(".shared-conversation-timeline")
        ?.contains(turns[1]!),
    ).toBe(true);
  });
  it("resolves a selected agent beyond the loaded fleet page without assuming its grant", async () => {
    const fetch = vi.fn(async (path: string) =>
      response(path.endsWith("/nodes/node-a") ? node : history),
    );
    vi.stubGlobal("fetch", fetch);
    await render(thread, []);
    expect(
      fetch.mock.calls.some(([path]) => path.endsWith("/nodes/node-a")),
    ).toBe(true);
    expect(container.querySelector(".composer-target")?.textContent).toBe(
      "Development workstation",
    );
    await write("Continue on the resolved agent");
    expect(button("Send message").disabled).toBe(false);
  });
  it.each([false, true])(
    "reconciles runtime revision progress once while respecting concurrent archive=%s",
    async (archived) => {
      const attempts: {
        expected_revision: number;
        request: { idempotency_key: string };
      }[] = [];
      vi.stubGlobal(
        "fetch",
        vi.fn(async (_path: string, options: RequestInit) => {
          if (_path.endsWith("/nodes/node-a")) return response(node);
          if (options.method === "POST") {
            attempts.push(JSON.parse(options.body as string));
            return attempts.length === 1
              ? response({ error: "conflict" }, 409)
              : response({ thread: { ...thread, revision: 8 } });
          }
          return response({
            ...history,
            thread: attempts.length
              ? { ...thread, revision: 8, archived }
              : thread,
          });
        }),
      );
      await render();
      await write("Continue after runtime progress");
      await act(async () => button("Send message").click());
      expect(attempts).toHaveLength(archived ? 1 : 2);
      if (!archived) {
        expect(attempts.map((attempt) => attempt.expected_revision)).toEqual([
          7, 8,
        ]);
        expect(attempts[0]?.request.idempotency_key).toBe(
          attempts[1]?.request.idempotency_key,
        );
      }
      expect(container.querySelector("textarea")?.value).toBe(
        archived ? "Continue after runtime progress" : "",
      );
    },
  );
  it("keeps imported local-run controls read-only even when project permissions allow responses", async () => {
    const imported = {
      ...thread,
      source: "runtime" as const,
      can_continue: false,
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        response({
          ...history,
          thread: imported,
          tasks: [
            {
              task_id: "imported-task",
              project_id: "project-a",
              node_id: "node-a",
              subject: "runtime",
              request: { input: [], role: "primary", mode: "execute" },
              run_id: "local-run",
              source_read_only: true,
              last_sequence: 1,
              revision: 1,
              snapshot: {
                run: {
                  run_id: "local-run",
                  title: "Shared session",
                  status: "waiting",
                  created_at: "2026-10-05T10:00:00Z",
                  updated_at: "2026-10-05T10:00:00Z",
                  role: "primary",
                  mode: "execute",
                  last_sequence: 1,
                  terminal: null,
                },
                pending_interactions: [
                  {
                    interaction_id: "approval-a",
                    run_id: "local-run",
                    kind: "approval",
                    status: "pending",
                    respondable_by_caller: true,
                    etag: "current",
                    content: {
                      approval: {
                        action: "execute",
                        resource: "command",
                        reason: "Needs approval",
                        risk: "medium",
                        request_hash: "prepared-request",
                        command_context: null,
                      },
                    },
                  },
                ],
              },
            },
          ],
        }),
      ),
    );
    await render(imported);
    expect(button("Approve this action").disabled).toBe(true);
    expect(button("Deny").disabled).toBe(true);
    expect(container.textContent).not.toContain("Stop active run");
  });
  it("reads saved history offline and prevents continuation when local sharing is read-only", async () => {
    const readOnly = {
      ...thread,
      source: "runtime" as const,
      can_continue: false,
      sync_status: "incomplete" as const,
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(response({ ...history, thread: readOnly })),
    );
    await render(readOnly, [{ ...node, presence: null }]);
    expect(container.textContent).toContain("Saved assistant reply");
    expect(container.textContent).toContain("viewing saved history");
    expect(container.textContent).toContain(
      "Released history is incomplete or bounded",
    );
    expect(button("Send message").disabled).toBe(true);
    expect(container.textContent).toContain("shared for viewing");
    expect(button("Archive").disabled).toBe(false);
  });
  it("submits multi-turn input against the exact revision and preserves its idempotency identity after a retry", async () => {
    const attempts: Array<{
      expected_revision: number;
      request: { idempotency_key: string; input: { text: string }[] };
    }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_path: string, options: RequestInit) => {
        if (options.method === "POST") {
          attempts.push(JSON.parse(options.body as string));
          return attempts.length === 1
            ? response({ error: "storage" }, 503)
            : response({ thread: { ...thread, revision: 8 } });
        }
        return response(history);
      }),
    );
    await render();
    await write("Continue with the deployment checks");
    await act(async () => button("Send message").click());
    expect(container.querySelector("textarea")?.value).toBe(
      "Continue with the deployment checks",
    );
    await act(async () => button("Send message").click());
    expect(attempts).toHaveLength(2);
    expect(attempts[0]?.expected_revision).toBe(7);
    expect(attempts[0]?.request.idempotency_key).toBe(
      attempts[1]?.request.idempotency_key,
    );
    expect(attempts[0]?.request.input[0]?.text).toBe(
      "Continue with the deployment checks",
    );
    expect(container.querySelector("textarea")?.value).toBe("");
  });
  it("loads earlier history without duplicating messages and uses Control for metadata mutations", async () => {
    const calls: Array<{ path: string; options: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (path: string, options: RequestInit) => {
        calls.push({ path, options });
        if (options.method === "PATCH")
          return response({
            thread: { ...thread, archived: true, revision: 8 },
          });
        if (path.includes("message_after="))
          return response({
            ...history,
            messages: [
              {
                message_id: "message-a",
                role: "user",
                text: "Earlier human request",
                created_at: "2026-10-05T10:00:00Z",
                task_id: "task-a",
              },
              ...history.messages,
            ],
          });
        return response({ ...history, next_message_cursor: "message-b" });
      }),
    );
    await render(thread, [node], ["read", "control"]);
    await act(async () => button("Load earlier history").click());
    expect(
      [...container.querySelectorAll(".shared-message-body")].map(
        (item) => item.textContent,
      ),
    ).toEqual(["Earlier human request", "Saved assistant reply"]);
    await act(async () => button("Archive").click());
    const mutation = calls.find((item) => item.options.method === "PATCH")!;
    expect(JSON.parse(mutation.options.body as string)).toEqual({
      revision: 7,
      archived: true,
    });
    expect(mutation.options.headers).toEqual({
      "Content-Type": "application/json",
      "X-Colossus-CSRF": "1",
    });
  });
});
