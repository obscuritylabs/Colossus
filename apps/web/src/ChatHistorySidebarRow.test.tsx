// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatHistorySidebarRow } from "./ChatHistorySidebarRow";
import { NavigationProvider } from "./navigation";
import type { Task, Thread, ThreadDetailResponse } from "./api";

const thread: Thread = {
  thread_id: "row.one",
  project_id: "project",
  node_id: "fixed-node",
  owner_user_id: "alice",
  title: "A different chat",
  created_at: "2026-10-21T08:00:00Z",
  updated_at: "2026-10-21T09:00:00Z",
  revision: 3,
  archived: false,
  session_id: "session",
  sync_status: "current",
  source: "cloud",
  can_continue: true,
};
const task: Task = {
  task_id: "source-task",
  project_id: thread.project_id,
  node_id: thread.node_id,
  thread_id: thread.thread_id,
  subject: "alice",
  request: { input: [{ text: "Source turn" }], mode: "act", role: "primary" },
  run_id: "source-run",
  revision: 1,
  last_sequence: 1,
  snapshot: {
    run: {
      run_id: "source-run",
      session_id: "session",
      title: "Source turn",
      status: "completed",
      created_at: thread.created_at,
      updated_at: thread.updated_at,
      role: "primary",
      mode: "act",
      last_sequence: 1,
      terminal: null,
    },
    pending_interactions: [],
  },
};
let root: Root,
  container: HTMLDivElement,
  row: Thread,
  detail: ThreadDetailResponse;
let canControl: boolean, forkEnabled: boolean;
let respond: (path: string, init?: RequestInit) => Promise<Response>;
const changed = vi.fn(),
  refresh = vi.fn(),
  removedFocus = vi.fn(),
  fork = vi.fn();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  row = { ...thread };
  detail = { thread: { ...thread, revision: 8 }, tasks: [task], messages: [] };
  canControl = true;
  forkEnabled = false;
  for (const callback of [changed, refresh, removedFocus, fork])
    callback.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  respond = async (_path, init) => {
    if (init?.method === "PATCH") {
      const { revision: _, ...values } = JSON.parse(String(init.body));
      return Response.json({ thread: { ...row, ...values, revision: 9 } });
    }
    return Response.json(detail);
  };
  vi.stubGlobal(
    "fetch",
    vi.fn((path: string, init?: RequestInit) => respond(path, init)),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(identity = "alice") {
  await act(async () =>
    root.render(
      <NavigationProvider>
        <ChatHistorySidebarRow
          key={`${identity}:${row.node_id}`}
          thread={row}
          project="project"
          identity={identity}
          selected={false}
          now={new Date("2026-10-21T12:00:00Z")}
          metadataId="row-meta"
          canControl={canControl}
          onChanged={changed}
          onRefresh={refresh}
          onRemovedFocus={removedFocus}
          onFork={forkEnabled ? fork : undefined}
        />
      </NavigationProvider>,
    ),
  );
}
function trigger() {
  return container.querySelector<HTMLButtonElement>(
    '[aria-label^="Chat actions for"]',
  )!;
}
async function open() {
  await act(async () =>
    trigger().dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
}
function item(label: string) {
  const value = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find((item) => item.textContent?.trim() === label);
  if (!value) throw new Error(`Missing ${label}`);
  return value;
}
async function select(label: string) {
  await act(async () => item(label).click());
}
async function write(value: string) {
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Chat title"]',
  )!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function submit() {
  await act(async () =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}
it("keeps actions beside the link and binds rename to the opened row revision", async () => {
  await render();
  expect(trigger().closest("a")).toBeNull();
  expect(trigger().hasAttribute("title")).toBe(false);
  expect(container.querySelector("a")?.getAttribute("href")).toBe(
    "/projects/project/chat/row.one",
  );
  await open();
  row = {
    ...row,
    title: "Newer chat title",
    updated_at: "2026-10-21T11:00:00Z",
    revision: 7,
  };
  await render();
  const context = document.querySelector(".chat-history-menu-context");
  expect(context?.querySelector("strong")?.textContent).toBe(thread.title);
  expect(context?.querySelector("time")?.getAttribute("datetime")).toBe(
    thread.updated_at.replace("Z", ".000Z"),
  );
  expect(
    document.querySelector('[role="group"][aria-label="Chat actions"]')
      ?.textContent,
  ).toContain("Rename");
  expect(
    document.querySelector('[role="group"][aria-label="Chat history"]')
      ?.textContent,
  ).toBe("Archive");
  await select("Rename");
  await write("  Renamed chat  ");
  await submit();
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(fetch).toHaveBeenCalledWith(
    "/api/projects/project/threads/row.one",
    expect.objectContaining({
      method: "PATCH",
      body: JSON.stringify({ revision: 3, title: "Renamed chat" }),
    }),
  );
  expect(changed).toHaveBeenCalledWith(
    expect.objectContaining({
      thread_id: thread.thread_id,
      title: "Renamed chat",
      revision: 9,
    }),
  );
});
it("validates UTF-8 title bounds and cancels rename without a mutation", async () => {
  await render();
  await open();
  await select("Rename");
  await write("🪐".repeat(65));
  await submit();
  expect(fetch).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "256 UTF-8 bytes",
  );
  const input = container.querySelector<HTMLInputElement>("input")!;
  await act(async () => {
    input.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
    await new Promise((resolve) => requestAnimationFrame(resolve));
  });
  expect(container.querySelector("form")).toBeNull();
  expect(document.activeElement).toBe(trigger());
  expect(changed).not.toHaveBeenCalled();
});
it("archives and restores the exact row without opening its conversation", async () => {
  await render();
  await open();
  await select("Archive");
  expect(fetch).toHaveBeenLastCalledWith(
    "/api/projects/project/threads/row.one",
    expect.objectContaining({
      method: "PATCH",
      body: JSON.stringify({ revision: 3, archived: true }),
    }),
  );
  row = { ...row, archived: true, revision: 9 };
  await render();
  await open();
  await select("Restore");
  expect(fetch).toHaveBeenLastCalledWith(
    "/api/projects/project/threads/row.one",
    expect.objectContaining({
      method: "PATCH",
      body: JSON.stringify({ revision: 9, archived: false }),
    }),
  );
  expect(changed).toHaveBeenCalledTimes(2);
});
it("refreshes a revision conflict without retrying or reporting a stale change", async () => {
  respond = async () => Response.json({ error: "conflict" }, { status: 409 });
  await render();
  await open();
  await select("Archive");
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(changed).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "state changed",
  );
});
it("loads Fork on demand and opens a draft with the fresh exact source", async () => {
  forkEnabled = true;
  detail.tasks = [
    { ...task, task_id: "inherited", source_read_only: true },
    task,
  ];
  await render();
  expect(fetch).not.toHaveBeenCalled();
  await open();
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(fetch).toHaveBeenLastCalledWith(
    "/api/projects/project/threads/row.one",
    expect.objectContaining({ method: "GET" }),
  );
  expect(fork).not.toHaveBeenCalled();
  expect(item("Fork chat").hasAttribute("data-disabled")).toBe(false);
  await select("Fork chat");
  expect(fork).toHaveBeenCalledWith(detail.thread, task);
  expect(fetch).toHaveBeenCalledTimes(1);
});
it("hides controls without authority or for another owner", async () => {
  canControl = false;
  await render();
  expect(trigger()).toBeNull();
  canControl = true;
  await render("bob");
  expect(trigger()).toBeNull();
  expect(fetch).not.toHaveBeenCalled();
});

it("allows an authorized Fork callback without granting metadata controls", async () => {
  canControl = false;
  forkEnabled = true;
  await render();
  await open();
  expect(document.querySelector('[role="menu"]')?.textContent).toContain(
    "Fork chat",
  );
  expect(document.querySelector('[role="menu"]')?.textContent).not.toMatch(
    /Rename|Archive|Restore/,
  );
  await select("Fork chat");
  expect(fork).toHaveBeenCalledWith(detail.thread, task);
});

it("suppresses duplicate mutations and rejects a response for another runtime binding", async () => {
  let complete!: (response: Response) => void;
  respond = () =>
    new Promise((resolve) => {
      complete = resolve;
    });
  await render();
  await open();
  const archive = item("Archive");
  await act(async () => {
    archive.click();
    archive.click();
  });
  expect(fetch).toHaveBeenCalledTimes(1);
  await act(async () =>
    complete(Response.json({ thread: { ...thread, node_id: "other-node" } })),
  );
  expect(changed).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "no longer matches",
  );
});
it("drops late fork detail when the account changes", async () => {
  forkEnabled = true;
  let complete!: (response: Response) => void;
  respond = () =>
    new Promise((resolve) => {
      complete = resolve;
    });
  await render();
  await open();
  expect(item("Fork chat").hasAttribute("data-disabled")).toBe(true);
  await render("bob");
  await act(async () => complete(Response.json(detail)));
  expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(fork).not.toHaveBeenCalled();
});
