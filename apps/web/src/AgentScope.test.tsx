// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentSidebar, AgentWorkspace } from "./AgentScope";
import type { FleetNode, Task, Thread } from "./api";
import type { RunRequest } from "./RunComposer";
const at = "2026-10-06T10:00:00Z";
const node = (id: string): FleetNode => ({
  node: {
    node_id: id,
    project_id: "p",
    instance_id: id,
    label: `Agent ${id}`,
    workspace_label: `Workspace ${id}`,
    certificate_sha256: "test",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: { ready: true, connection_id: id, capabilities: [] },
});
const nodes = [node("a"), node("b")];
const thread = (id: string, nodeId: string): Thread => ({
  thread_id: id,
  project_id: "p",
  node_id: nodeId,
  title: `Thread ${id}`,
  created_at: at,
  updated_at: at,
  revision: 1,
  archived: false,
  source: "cloud",
  sync_status: "current",
  can_continue: nodeId === "a",
});
const first = thread("first", "a"),
  other = thread("other", "b"),
  older = thread("older", "a");
const task: Task = {
  task_id: "task",
  project_id: "p",
  node_id: "a",
  thread_id: "first",
  subject: "human",
  request: { input: [{ text: "Plan" }], role: "primary", mode: "plan" },
  run_id: "run",
  last_sequence: 1,
  revision: 1,
  snapshot: {
    run: {
      run_id: "run",
      title: "Plan",
      role: "primary",
      mode: "plan",
      status: "completed",
      created_at: at,
      updated_at: at,
      last_sequence: 1,
      terminal: null,
    },
    pending_interactions: [],
  },
};
let root: Root, container: HTMLDivElement, paths: string[];
let deferOlder = false;
const pending: { resolve: ((response: Response) => void) | null } = {
  resolve: null,
};
const response = (value: unknown) => new Response(JSON.stringify(value));
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  paths = [];
  deferOlder = false;
  pending.resolve = null;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      paths.push(path);
      const url = new URL(path, "http://fixture.test");
      if (url.searchParams.has("after")) {
        if (deferOlder)
          return new Promise<Response>((resolve) => {
            pending.resolve = resolve;
          });
        return response({ threads: [older], next_cursor: null });
      }
      return response({
        threads: url.searchParams.has("node_id") ? [first] : [first, other],
        next_cursor: url.searchParams.has("node_id") ? "older-cursor" : null,
      });
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
const button = (name: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.getAttribute("aria-label") === name ||
      item.textContent?.trim() === name,
  )!;
async function renderSidebar() {
  await act(() =>
    root.render(
      <AgentSidebar
        project="p"
        nodes={nodes}
        hosts={[]}
        agentId="a"
        selected="first"
        tasks={[task]}
        onAgent={() => {}}
        onOpen={() => {}}
        onNew={() => {}}
        onBack={() => {}}
      />,
    ),
  );
}
it("uses authorized workspace scopes and appends paged history with exact permalinks", async () => {
  await renderSidebar();
  expect(paths[0]).toContain("node_id=a");
  expect(container.textContent).toContain("Plan ·");
  expect(
    container.querySelector('a[href="/projects/p/threads/first"]'),
  ).not.toBeNull();
  await act(() => button("Load older threads").click());
  expect(paths.at(-1)).toContain("after=older-cursor");
  expect(container.textContent).toContain("Thread older");
  expect(container.textContent).toContain("2 loaded threads");
  await act(() => button("All Workspaces").click());
  expect(
    new URL(paths.at(-1)!, "http://fixture.test").searchParams.has("node_id"),
  ).toBe(false);
  expect(container.textContent).toContain("In this project");
  expect(
    container.querySelector('a[href="/projects/p/threads/other"]'),
  ).not.toBeNull();
  expect(container.textContent).toContain("View only");
  expect(container.textContent).not.toContain("Execute ·");
});
it("keeps search keyboard focus and rejects an older page after a scope change", async () => {
  await renderSidebar();
  await act(() =>
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "k",
        ctrlKey: true,
        cancelable: true,
      }),
    ),
  );
  expect(document.activeElement).toBe(
    container.querySelector('input[aria-label="Search threads"]'),
  );
  deferOlder = true;
  await act(() => button("Load older threads").click());
  expect(pending.resolve).not.toBeNull();
  await act(() => button("All Workspaces").click());
  await act(async () =>
    pending.resolve!(response({ threads: [older], next_cursor: null })),
  );
  expect(container.textContent).not.toContain("Thread older");
  expect(container.textContent).toContain("Thread other");
});
it("renders a dedicated new-work hero and seeds a focused draft without changing retry idempotence", async () => {
  const submit = vi.fn<(request: RunRequest) => Promise<boolean>>(
    async () => false,
  );
  await act(() =>
    root.render(
      <AgentWorkspace
        project="p"
        agent={nodes[0]}
        nodes={nodes}
        permissions={["read", "execute"]}
        creating
        busy={false}
        onCreate={submit}
        onNew={() => {}}
        view="threads"
      />,
    ),
  );
  expect(container.querySelector(".shared-work-welcome")).not.toBeNull();
  expect(container.querySelector(".section-tabs")).toBeNull();
  expect(container.textContent).not.toContain("Agent conversations");
  await act(() => button("Orient yourself in this repo").click());
  const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(input.value).toBe("Orient yourself in this repo");
  expect(document.activeElement).toBe(input);
  await act(() => button("Start conversation").click());
  await act(() => button("Start conversation").click());
  expect(submit).toHaveBeenCalledTimes(2);
  expect(submit.mock.calls[0]![0].idempotency_key).toBe(
    submit.mock.calls[1]![0].idempotency_key,
  );
  expect(submit.mock.calls[0]![0].role).toBe("primary");
  expect(submit.mock.calls[0]![0].mode).toBe("execute");
});
