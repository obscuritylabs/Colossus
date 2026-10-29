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
  vi.restoreAllMocks();
});

describe("loaded conversation actions", () => {
  const transcript = (): ThreadDetailResponse => ({
    ...history,
    thread: {
      ...thread,
      sync_status: "incomplete",
      can_continue: false,
      source: "runtime",
    },
    next_message_cursor: "earlier",
    messages: [
      {
        message_id: "first",
        role: "user",
        text: "Deployment alpha",
        created_at: "2026-10-05T10:00:00Z",
        task_id: "task-a",
      },
      ...history.messages.map((message) => ({
        ...message,
        text: "Saved deployment beta",
      })),
      {
        message_id: "system",
        role: "system",
        text: "PRIVATE SYSTEM INSTRUCTION",
        created_at: "2026-10-05T10:02:00Z",
      },
      {
        message_id: "tool",
        role: "tool",
        text: "PRIVATE TOOL OUTPUT",
        created_at: "2026-10-05T10:03:00Z",
      },
    ],
    message_authors: {
      first: { user_id: "alice", display_name: "Alice Example" },
    },
    tasks: [
      {
        task_id: "task-a",
        project_id: "project-a",
        node_id: "node-a",
        subject: "alice",
        request: {
          input: [
            { text: "PRIVATE RAW REQUEST" },
            { artifact: "PRIVATE-ATTACHMENT-HANDLE" },
          ],
          mode: "execute",
          role: "primary",
        },
        run_id: "run-a",
        revision: 1,
        last_sequence: 0,
        snapshot: null,
        source_read_only: true,
      },
    ],
  });
  async function search(query: string) {
    const input = container.querySelector<HTMLInputElement>(
      ".conversation-find input",
    )!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(input, query);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    return input;
  }
  const selected = () =>
    container.querySelector<HTMLElement>(".conversation-find-active");

  it("finds only loaded user/assistant text offline, wraps message navigation, and restores focus on Escape", async () => {
    const detail = transcript();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(detail)),
    );
    await render(detail.thread, [{ ...node, presence: null }], ["read"]);
    await act(() => button("Find in loaded conversation").click());
    const input = container.querySelector<HTMLInputElement>(
      ".conversation-find input",
    )!;
    expect(document.activeElement).toBe(input);
    expect(container.textContent).toContain(
      "Load earlier history to search more",
    );
    await search("DEPLOYMENT");
    expect(
      container.querySelector(".conversation-find-count")?.textContent,
    ).toBe("1 of 2 matching messages");
    expect(selected()?.dataset.asideMessageId).toBe("first");
    await act(() => button("Next matching message").click());
    expect(selected()?.dataset.asideMessageId).toBe("message-b");
    expect(document.activeElement).toBe(selected());
    await act(() => button("Next matching message").click());
    expect(selected()?.dataset.asideMessageId).toBe("first");
    await act(() => button("Previous matching message").click());
    expect(selected()?.dataset.asideMessageId).toBe("message-b");
    await search("PRIVATE");
    expect(
      container.querySelector(".conversation-find-count")?.textContent,
    ).toBe("No matching messages");
    expect(button("Next matching message").disabled).toBe(true);
    expect(selected()).toBeNull();
    await act(() => button("Topology").click());
    expect(container.querySelector(".conversation-find")).toBeNull();
    await act(() => button("Find in loaded conversation").click());
    const reopened = container.querySelector<HTMLInputElement>(
      ".conversation-find input",
    )!;
    expect(document.activeElement).toBe(reopened);
    expect(reopened.value).toBe("PRIVATE");
    await act(() =>
      reopened.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      ),
    );
    expect(container.querySelector(".conversation-find")).toBeNull();
    expect(document.activeElement).toBe(button("Find in loaded conversation"));
    const browserFind = new KeyboardEvent("keydown", {
      key: "f",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(browserFind);
    expect(browserFind.defaultPrevented).toBe(false);
  });

  it("retains the selected message during released updates and adds explicitly loaded earlier history to search", async () => {
    let detail = transcript();
    vi.stubGlobal(
      "fetch",
      vi.fn(async (path: string) =>
        response(
          path.includes("message_after=")
            ? {
                ...detail,
                next_message_cursor: null,
                messages: [
                  {
                    message_id: "older",
                    role: "user",
                    text: "Earlier deployment context",
                    created_at: "2026-10-04T10:00:00Z",
                  },
                ],
              }
            : detail,
        ),
      ),
    );
    await render(detail.thread, [node], ["read"]);
    await act(() => button("Find in loaded conversation").click());
    const input = await search("deployment");
    await act(() =>
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      ),
    );
    expect(selected()?.dataset.asideMessageId).toBe("message-b");
    detail = {
      ...detail,
      messages: detail.messages.map((message) =>
        message.message_id === "message-b"
          ? { ...message, text: message.text + " is still streaming" }
          : message,
      ),
    };
    await act(async () =>
      TestEventSource.instances[0]!.dispatchEvent(new Event("thread_changed")),
    );
    expect(selected()?.dataset.asideMessageId).toBe("message-b");
    expect(input.value).toBe("deployment");
    await act(async () => button("Load earlier history").click());
    expect(
      container.querySelector(".conversation-find-count")?.textContent,
    ).toBe("3 of 3 matching messages");
    expect(selected()?.dataset.asideMessageId).toBe("message-b");
    await act(() => button("Next matching message").click());
    expect(selected()?.dataset.asideMessageId).toBe("older");
  });

  it("clears search and ignores late copy feedback when conversation identity changes", async () => {
    let detail = transcript();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(detail)),
    );
    let copied!: () => void;
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockImplementation(
        () =>
          new Promise<void>((resolve) => {
            copied = resolve;
          }),
      );
    await render(detail.thread, [node], ["read"]);
    await act(() => button("Find in loaded conversation").click());
    await search("deployment");
    await act(async () =>
      (await conversationMenuItem("Copy loaded conversation")).click(),
    );
    expect(writeText).toHaveBeenCalledOnce();
    detail = {
      ...detail,
      thread: {
        ...detail.thread,
        thread_id: "thread-next",
        session_id: "session-next",
        title: "Another conversation",
      },
    };
    await render(detail.thread, [node], ["read"]);
    expect(container.querySelector(".conversation-find")).toBeNull();
    await act(() => copied());
    expect(container.textContent).not.toContain("Loaded conversation copied");
    await act(() => button("Find in loaded conversation").click());
    expect(
      container.querySelector<HTMLInputElement>(".conversation-find input")
        ?.value,
    ).toBe("");
  });

  it("copies an explicit loaded text snapshot without task, system, tool, or attachment payloads, and reports clipboard failure", async () => {
    const detail = transcript();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(detail)),
    );
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue();
    await render(detail.thread, [{ ...node, presence: null }], ["read"]);
    await act(async () =>
      (await conversationMenuItem("Copy loaded conversation")).click(),
    );
    const markdown = writeText.mock.calls[0]![0];
    expect(markdown).toContain("Alice Example");
    expect(markdown).toContain("Deployment alpha");
    expect(markdown).toContain("Saved deployment beta");
    expect(markdown).toContain(
      "Earlier history is available but is not included",
    );
    expect(markdown).toContain("not the complete agent conversation");
    expect(markdown).not.toContain("PRIVATE");
    expect(markdown).not.toContain("task-a");
    expect(container.textContent).toContain(
      "Loaded conversation copied as Markdown",
    );
    writeText.mockRejectedValueOnce(new Error("clipboard denied"));
    await act(async () =>
      (await conversationMenuItem("Copy loaded conversation")).click(),
    );
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Try downloading it instead",
    );
  });

  it("downloads UTF-8 Markdown on click and releases its object URL", async () => {
    const detail = transcript();
    detail.thread = { ...detail.thread, title: "Inspect / 東京" };
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(detail)),
    );
    const urls = vi.fn((_blob: Blob) => "blob:conversation-snapshot"),
      revoke = vi.fn();
    vi.stubGlobal(
      "URL",
      class extends URL {
        static createObjectURL = urls;
        static revokeObjectURL = revoke;
      },
    );
    const clicks = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => {});
    await render(detail.thread, [node], ["read"]);
    expect(urls).not.toHaveBeenCalled();
    await act(async () =>
      (await conversationMenuItem("Download loaded conversation")).click(),
    );
    expect(urls).toHaveBeenCalledOnce();
    const blob = urls.mock.calls[0]![0] as Blob;
    expect(blob.type).toBe("text/markdown;charset=utf-8");
    expect(await blob.text()).toContain("Saved deployment beta");
    const link = clicks.mock.instances[0] as HTMLAnchorElement;
    expect(link.download).toBe("Inspect 東京-loaded.md");
    expect(link.href).toBe("blob:conversation-snapshot");
    expect(link.isConnected).toBe(false);
    await vi.waitFor(() =>
      expect(revoke).toHaveBeenCalledWith("blob:conversation-snapshot"),
    );
  });
});
const response = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status });
async function render(
  initial = thread,
  nodes = [node],
  permissions: Permission[] = ["read", "execute", "control", "approve"],
  personalChat?: { owner: string; node: string },
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
        personalChat={personalChat}
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
async function conversationMenuItem(label: string) {
  const trigger = container.querySelector<HTMLButtonElement>(
    '[aria-label="Conversation actions"]',
  );
  if (!trigger) throw new Error("Missing conversation action menu");
  await act(async () => {
    trigger.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
  });
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find((item) => item.textContent?.trim() === label);
  if (!item) throw new Error(`Missing menu action ${label}`);
  return item;
}

describe("human cloud conversation authority", () => {
  const planHistory = (): ThreadDetailResponse => ({
    ...history,
    thread: { ...thread, owner_user_id: "alice" },
    tasks: [
      {
        task_id: "task-a",
        project_id: "project-a",
        node_id: "node-a",
        subject: "alice",
        request: {
          input: [{ text: "Draft a plan" }],
          mode: "plan",
          role: "primary",
        },
        run_id: "run-a",
        revision: 1,
        last_sequence: 0,
        snapshot: {
          run: {
            run_id: "run-a",
            session_id: "session-a",
            title: "A reviewed plan",
            status: "completed",
            mode: "plan",
            role: "primary",
            created_at: "2026-10-05T10:00:00Z",
            updated_at: "2026-10-05T10:01:00Z",
            last_sequence: 0,
            terminal: {
              result: {
                output: "A controlled plan",
                plan_id: "plan-a",
                plan_revision: 2,
                plan_status: "draft",
              },
            },
          },
          pending_interactions: [],
        },
      },
    ],
  });
  it("reviews and submits an exact personal Plan with immutable execute retries", async () => {
    const detail = planHistory();
    const attempts: Array<{ request: Record<string, unknown> }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (path: string, options: RequestInit) => {
        if (path.endsWith("/resources"))
          return response({
            kind: "result",
            connection_id: "connection-a",
            value: { capabilities: ["plans.continue", "goal.create"] },
          });
        if (path.endsWith("/messages") && options.method === "POST") {
          attempts.push(JSON.parse(options.body as string));
          return attempts.length === 1
            ? response({ error: "storage" }, 503)
            : response({ thread: detail.thread });
        }
        return response(detail);
      }),
    );
    await render(
      detail.thread,
      [
        {
          ...node,
          presence: {
            ...node.presence!,
            capabilities: ["plans.continue", "goal.create"],
          },
        },
      ],
      ["read", "execute", "control"],
      { owner: "alice", node: "node-a" },
    );
    await act(() => button("Read plan").click());
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain(
      "A controlled plan",
    );
    await act(() =>
      [
        ...document.querySelectorAll<HTMLButtonElement>(
          '[role="dialog"] button',
        ),
      ]
        .find((item) => item.textContent === "Execute once")!
        .click(),
    );
    expect(container.querySelector("textarea")?.value).toBe(
      "Approve and execute Plan revision 2 once.",
    );
    expect(container.querySelector(".composer-fixed-role")?.textContent).toBe(
      "Role: primary",
    );
    expect(container.querySelector('[aria-label="Agent role"]')).toBeNull();
    await act(() => button("Approve and execute plan").click());
    await act(() => button("Approve and execute plan").click());
    expect(attempts).toHaveLength(2);
    expect(attempts[0]?.request).toEqual(attempts[1]?.request);
    expect(attempts[0]?.request).toMatchObject({
      session_id: null,
      mode: "execute",
      role: "primary",
      plan_action: {
        execute: {
          source_run_id: "run-a",
          expected_revision: 2,
          strategy: "direct",
        },
      },
    });
    expect(container.querySelector(".composer-fixed-role")).toBeNull();
    expect(container.querySelector('[aria-label="Agent role"]')).not.toBeNull();
  });
  it("revises from the Plans view using the exact selected reference", async () => {
    const detail = planHistory();
    const submissions: Array<{ request: Record<string, unknown> }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (path: string, options: RequestInit) => {
        if (path.endsWith("/resources"))
          return response({ kind: "unsupported" });
        if (path.endsWith("/messages") && options.method === "POST") {
          submissions.push(JSON.parse(options.body as string));
          return response({ thread: detail.thread });
        }
        return response(detail);
      }),
    );
    await render(
      detail.thread,
      [
        {
          ...node,
          presence: { ...node.presence!, capabilities: ["plans.continue"] },
        },
      ],
      ["read", "execute", "control"],
      { owner: "alice", node: "node-a" },
    );
    await act(() => button("Plans").click());
    await act(() => button("Revise").click());
    await write("Use a smaller first step");
    await act(() => button("Revise plan").click());
    expect(submissions[0]?.request).toMatchObject({
      mode: "plan",
      input: [{ text: "Use a smaller first step" }],
      plan_action: { revise: { source_run_id: "run-a", expected_revision: 2 } },
    });
  });
  it.each([
    {
      label: "another user",
      owner: "bob",
      personalNode: "node-a",
      permissions: ["read", "execute", "control"],
    },
    {
      label: "shared node",
      owner: "alice",
      personalNode: "different-node",
      permissions: ["read", "execute", "control", "approve"],
    },
    {
      label: "read only",
      owner: "alice",
      personalNode: "node-a",
      permissions: ["read"],
    },
  ])(
    "keeps Plan execution unavailable for $label",
    async ({ owner, personalNode, permissions }) => {
      const detail = planHistory();
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => response(detail)),
      );
      await render(
        detail.thread,
        [
          {
            ...node,
            presence: { ...node.presence!, capabilities: ["plans.continue"] },
          },
        ],
        permissions as Permission[],
        { owner, node: personalNode },
      );
      expect(container.textContent).toContain("Plan revision 2");
      expect(
        [...container.querySelectorAll("button")].some(
          (item) => item.textContent === "Execute once",
        ),
      ).toBe(false);
      expect(
        [...container.querySelectorAll("button")].some(
          (item) => item.textContent === "Revise in chat",
        ),
      ).toBe(false);
    },
  );
  it("retains inbox inspection when switching shared session views", async () => {
    const task: Task = {
      task_id: "task-a",
      project_id: "project-a",
      node_id: "node-a",
      subject: "human",
      request: {
        input: [{ text: "Inspect delegated work" }],
        mode: "execute",
        role: "primary",
      },
      run_id: "run-a",
      revision: 1,
      last_sequence: 0,
      snapshot: null,
    };
    const fetch = vi.fn(async (_url: string) =>
      response({ ...history, tasks: [task] }),
    );
    vi.stubGlobal("fetch", fetch);
    await render(
      thread,
      [
        {
          ...node,
          presence: {
            ...node.presence!,
            capabilities: ["agent_messages.read.v1"],
          },
        },
      ],
      ["read"],
    );
    expect(
      container.querySelector(".conversation-run details summary")?.textContent,
    ).not.toBe("Agent inboxes");
    await act(() => button("Inboxes").click());
    const inboxes = container.querySelector('[aria-label="Session inboxes"]');
    expect(inboxes?.querySelector("summary")?.textContent).toBe(
      "Agent inboxes",
    );
    expect(
      container.querySelector('[aria-label="Conversation history"]'),
    ).toBeNull();
    expect(fetch.mock.calls.some(([url]) => url.endsWith("/inboxes"))).toBe(
      false,
    );
    await act(() => button("Conversation").click());
    expect(
      container.querySelector('[aria-label="Conversation history"]'),
    ).not.toBeNull();
  });
  it("keeps unsupported inbox inspection out of conversation and session navigation", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(planHistory())),
    );
    await render();
    expect(
      container.querySelector('[aria-label="Session views"]')?.textContent,
    ).not.toContain("Inboxes");
    expect(
      container.querySelector('[aria-label="Conversation history"]')
        ?.textContent,
    ).not.toContain("Agent inboxes");
  });
  it("renders recorded account names and leaves imported authors neutral", async () => {
    const messages = ["own", "other", "imported"].map((id, index) => ({
      message_id: id,
      role: "user",
      text: `${id} message`,
      created_at: `2026-10-05T10:0${index}:00Z`,
    }));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        response({
          ...history,
          messages,
          message_authors: {
            own: { user_id: "alice", display_name: "Alice Example" },
            other: { user_id: "bob", display_name: "Bob Example" },
          },
        }),
      ),
    );
    await render(thread, [node], ["read"]);
    expect(
      [...container.querySelectorAll(".shared-feed-heading strong")].map(
        (entry) => entry.textContent,
      ),
    ).toEqual(["Alice Example", "Bob Example", "User"]);
    expect(container.textContent).not.toContain("Human");
  });
  it("refreshes the displayed title from canonical thread metadata when the runtime changes it", async () => {
    let canonical = planHistory();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(canonical)),
    );
    await render(canonical.thread, [node], ["read"]);
    const heading = () =>
      container.querySelector(".shared-work-header h2")?.textContent;
    expect(heading()).toBe(thread.title);
    expect(heading()).not.toBe(canonical.tasks[0]?.snapshot?.run.title);

    canonical = {
      ...canonical,
      thread: { ...canonical.thread, title: "A Funny Joke", revision: 8 },
    };
    await act(async () => {
      TestEventSource.instances[0]!.dispatchEvent(new Event("thread_changed"));
    });
    expect(heading()).toBe("A Funny Joke");
  });
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
          tool_name: "mcp.call",
          state,
          summary:
            state === "requested"
              ? "Call MCP tool · requested"
              : `Call MCP tool · ${task.task_id === "first" ? "GitLab" : "Splunk"} · search · ${state} · ${task.task_id} ${state}`,
          ...(state === "requested"
            ? {
                input: JSON.stringify({
                  server: task.task_id === "first" ? "GitLab" : "Splunk",
                  tool: "search",
                }),
              }
            : {}),
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
        turn.querySelector(".shared-activity-tool > summary")?.textContent,
      ).toContain(
        `Call MCP tool · ${index === 0 ? "GitLab" : "Splunk"} · search · completed`,
      );
      expect(
        [...turn.children].map(
          (child) =>
            child.getAttribute("data-role") ??
            child.querySelector("[data-role]")?.getAttribute("data-role") ??
            child.className,
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
  it.each([
    ["runtime", false, false],
    ["alice", true, false],
    ["alice", false, true],
  ])(
    "only offers side-chat actions for owned canonical sources (subject=%s, readOnly=%s)",
    async (subject, readOnly, offered) => {
      const detail = planHistory();
      detail.tasks[0] = {
        ...detail.tasks[0]!,
        subject: String(subject),
        source_read_only: Boolean(readOnly),
        thread_id: thread.thread_id,
      };
      vi.stubGlobal(
        "fetch",
        vi.fn(async () => response(detail)),
      );
      await render(detail.thread, [node], ["read", "execute", "control"], {
        owner: "alice",
        node: node.node.node_id,
      });
      expect(
        [...container.querySelectorAll("button")].some((button) =>
          button.textContent?.includes("Ask in side chat"),
        ),
      ).toBe(offered);
    },
  );
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
          if (_path.endsWith("/resources"))
            return response({ kind: "unsupported" });
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
    expect(
      (await conversationMenuItem("Archive")).getAttribute("aria-disabled"),
    ).not.toBe("true");
  });
  it("submits multi-turn input against the exact revision and preserves its idempotency identity after a retry", async () => {
    const attempts: Array<{
      expected_revision: number;
      request: { idempotency_key: string; input: { text: string }[] };
    }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_path: string, options: RequestInit) => {
        if (_path.endsWith("/resources"))
          return response({ kind: "unsupported" });
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
            message_authors: {
              "message-a": { user_id: "bob", display_name: "Bob Example" },
            },
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
    expect(
      container.querySelector(".shared-feed-heading strong")?.textContent,
    ).toBe("Bob Example");
    const archive = await conversationMenuItem("Archive");
    await act(async () => archive.click());
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
