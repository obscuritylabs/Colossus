// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  ThreadBranchDraft,
  branchTitle,
  type AsideDraftSource,
} from "./ThreadBranchDraft";
import type { Task, Thread } from "./api";
import type { RunRequest } from "./RunComposer";

const submitted: RunRequest = {
  input: [{ text: "Explain this" }],
  plugin_skill_ids: [],
  session_id: null,
  end_user_id: null,
  role: "primary",
  mode: "execute",
  research_depth: null,
  research_sources: [],
  plan_action: null,
  branch: null,
  max_turns: 24,
  idempotency_key: "human-request",
};
vi.mock("./RunComposer", () => ({
  RunComposer: ({
    onSubmit,
  }: {
    onSubmit: (value: RunRequest) => Promise<boolean>;
  }) => <button onClick={() => void onSubmit(submitted)}>Send branch</button>,
}));
const parent: Thread = {
  thread_id: "parent",
  project_id: "project",
  node_id: "node",
  session_id: "session",
  owner_user_id: "alice",
  title: "A chat",
  revision: 2,
  archived: false,
  can_continue: true,
  source: "cloud",
  sync_status: "current",
  created_at: "now",
  updated_at: "now",
};
const source: Task = {
  task_id: "source",
  project_id: "project",
  node_id: "node",
  thread_id: "parent",
  subject: "alice",
  run_id: "run",
  source_read_only: false,
  request: { role: "primary", mode: "execute", input: [] },
  last_sequence: 1,
  revision: 1,
  snapshot: {
    run: {
      run_id: "run",
      session_id: "session",
      title: "Source run",
      terminal: null,
      mode: "execute",
      role: "primary",
      status: "completed",
      created_at: "now",
      updated_at: "now",
      last_sequence: 1,
    },
    pending_interactions: [],
  },
};
const draft: AsideDraftSource = {
  parent,
  source,
  key: "draft",
  quote: "Selected\nanswer",
};
let container: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
const response = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });
async function render() {
  await act(() =>
    root.render(
      <ThreadBranchDraft
        draft={draft}
        kind="aside"
        project="project"
        nodes={[]}
        disabled={false}
        onCancel={() => {}}
        onChanged={() => {}}
        onCreated={() => {}}
      />,
    ),
  );
}
async function send() {
  await act(async () => {
    [...container.querySelectorAll("button")]
      .find((button) => button.textContent === "Send branch")!
      .click();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it("bounds branch titles by UTF-8 bytes without control characters or split characters", () => {
  const value = branchTitle("aside", "\n" + "🙂".repeat(200) + "\t");
  expect(new TextEncoder().encode(value).length).toBeLessThanOrEqual(256);
  expect(value).not.toMatch(/[\n\t\uFFFD]/u);
});

it("refreshes the pinned source before Send and reconciles an unknown outcome with the same immutable POST", async () => {
  const posts: unknown[] = [];
  let reads = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (init?.method === "POST") {
        posts.push(JSON.parse(String(init.body)));
        return response({ error: "storage" }, 500);
      }
      reads++;
      return response(
        url.endsWith("/tasks/source")
          ? { task: source }
          : {
              thread: { ...parent, revision: 8 },
              tasks: [source],
              messages: [],
            },
      );
    }),
  );
  await render();
  expect(posts).toHaveLength(0);
  await send();
  expect(reads).toBe(2);
  expect(posts).toHaveLength(1);
  expect(posts[0]).toMatchObject({
    source_task_id: "source",
    kind: "aside",
    expected_revision: 8,
    request: {
      branch: null,
      session_id: null,
      idempotency_key: "human-request",
      input: [
        { text: "Selected text (quoted context):\nSelected\nanswer" },
        { text: "Explain this" },
      ],
    },
  });
  await send();
  expect(reads).toBe(2);
  expect(posts[1]).toEqual(posts[0]);
});

it("does not POST when preflight finds a replacement runtime or source session", async () => {
  const fetch = vi.fn(async (url: string, init?: RequestInit) => {
    if (init?.method === "POST") throw new Error("Unexpected mutation");
    return response(
      url.endsWith("/tasks/source")
        ? { task: { ...source, run_id: "replacement" } }
        : { thread: parent, tasks: [], messages: [] },
    );
  });
  vi.stubGlobal("fetch", fetch);
  await render();
  await send();
  expect(container.textContent).toContain("The source chat changed");
  expect(fetch.mock.calls.some(([, init]) => init?.method === "POST")).toBe(
    false,
  );
});

it("only revalidates a confirmed conflict on the next explicit Send", async () => {
  const posts: { expected_revision: number }[] = [];
  let revision = 3;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (init?.method === "POST") {
        posts.push(JSON.parse(String(init.body)));
        revision = 4;
        return response({ error: "conflict" }, 409);
      }
      return response(
        url.endsWith("/tasks/source")
          ? { task: source }
          : { thread: { ...parent, revision }, tasks: [source], messages: [] },
      );
    }),
  );
  await render();
  await send();
  expect(posts).toHaveLength(1);
  expect(posts[0]?.expected_revision).toBe(3);
  await send();
  expect(posts).toHaveLength(2);
  expect(posts[1]?.expected_revision).toBe(4);
});
