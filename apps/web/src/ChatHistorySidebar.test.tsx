// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatHistorySidebar } from "./ChatHistorySidebar";
import { NavigationProvider } from "./navigation";
import type { Thread } from "./api";

let root: Root, container: HTMLDivElement;
const paths: URL[] = [];
const now = new Date(2026, 9, 21, 12);
const current: Thread = {
  thread_id: "current",
  project_id: "p",
  owner_user_id: "alice",
  node_id: "personal-node",
  title: "Current conversation",
  created_at: new Date(2026, 9, 21, 8).toISOString(),
  updated_at: new Date(2026, 9, 21, 10).toISOString(),
  revision: 1,
  archived: false,
  session_id: "session-current",
  sync_status: "current",
  source: "cloud",
  can_continue: true,
};
const older: Thread = {
  ...current,
  thread_id: "older",
  title: "Earlier conversation",
  updated_at: new Date(2026, 9, 1, 10).toISOString(),
};
let threads: Thread[];
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(now);
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear();
  paths.length = 0;
  threads = [current];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      const url = new URL(path, "https://control.example");
      paths.push(url);
      const after = url.searchParams.has("after");
      const archived = url.searchParams.get("archived") === "true";
      return new Response(
        JSON.stringify({
          threads: archived
            ? [{ ...current, archived: true, title: "Archived conversation" }]
            : after
              ? [older]
              : threads,
          next_cursor: after || archived ? null : "server-cursor",
        }),
      );
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
async function render(refreshKey = 0) {
  await act(async () =>
    root.render(
      <NavigationProvider>
        <ChatHistorySidebar
          project="p"
          identity="alice"
          selected={current}
          refreshKey={refreshKey}
        />
      </NavigationProvider>,
    ),
  );
}
function button(text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent?.trim() === text,
  )!;
}

it("retains server cursors, search and selection in conversation navigation", async () => {
  await render();
  expect(
    container
      .querySelector('a[aria-current="page"]')
      ?.getAttribute("aria-label"),
  ).toBe("Current conversation");
  expect(paths.at(-1)?.searchParams.get("archived")).toBe("false");
  await act(() => button("Older chats").click());
  expect(paths.at(-1)?.searchParams.get("after")).toBe("server-cursor");
  expect(container.textContent).toContain("Earlier conversation");
  await render(1);
  expect(paths.at(-1)?.searchParams.get("after")).toBe("server-cursor");
  await act(() => button("Newer chats").click());
  expect(paths.at(-1)?.searchParams.has("after")).toBe(false);
  await act(() => button("Older chats").click());
  const input = container.querySelector<HTMLInputElement>(
    'input[type="search"]',
  )!;
  const requestsBeforeSearch = paths.length;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "migration");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => new Promise((resolve) => setTimeout(resolve, 300)));
  expect(paths.at(-1)?.searchParams.get("query")).toBe("migration");
  expect(paths.at(-1)?.searchParams.has("after")).toBe(false);
  expect(
    paths
      .slice(requestsBeforeSearch)
      .every((url) => !url.searchParams.has("after")),
  ).toBe(true);
  expect(paths.every((url) => !url.searchParams.has("user_id"))).toBe(true);
});

it("resets the cursor for the archive view and preserves it on refresh", async () => {
  await render();
  await act(() => button("Older chats").click());
  expect(paths.at(-1)?.searchParams.get("after")).toBe("server-cursor");
  const requestsBeforeArchive = paths.length;
  await act(() => button("Archived").click());
  expect(paths.at(-1)?.searchParams.get("archived")).toBe("true");
  expect(paths.at(-1)?.searchParams.has("after")).toBe(false);
  expect(
    paths
      .slice(requestsBeforeArchive)
      .every((url) => !url.searchParams.has("after")),
  ).toBe(true);
  expect(container.textContent).toContain("Archived conversation");
  expect(button("Archived").getAttribute("aria-checked")).toBe("true");
  expect(
    container.querySelector('[aria-label="Chat history view"]'),
  ).not.toBeNull();
  await render(1);
  expect(paths.at(-1)?.searchParams.get("archived")).toBe("true");
  await act(() => button("Open").click());
  expect(paths.at(-1)?.searchParams.get("archived")).toBe("false");
  expect(paths.at(-1)?.searchParams.has("after")).toBe(false);
});

it("refreshes a runtime-updated title without losing the selected chat route", async () => {
  await render();
  const selected = () =>
    container.querySelector<HTMLAnchorElement>('a[aria-current="page"]')!;
  expect(selected().getAttribute("aria-label")).toBe(current.title);
  threads = [{ ...current, title: "A Funny Joke", revision: 2 }];
  await render(1);
  expect(selected().getAttribute("href")).toBe("/projects/p/chat/current");
  expect(selected().getAttribute("aria-label")).toBe("A Funny Joke");
  expect(selected().textContent).toContain("A Funny Joke");
  expect(selected().textContent).not.toContain(current.title);
});

it("shows released titles and described updated times without guessing run metadata", async () => {
  threads = [
    {
      ...current,
      active_task_id: "outstanding-task",
      queued_task_ids: ["outstanding-task"],
    },
    {
      ...older,
      thread_id: "yesterday",
      title: "Yesterday conversation",
      updated_at: new Date(2026, 9, 20, 10).toISOString(),
      can_continue: false,
    },
    {
      ...older,
      thread_id: "week",
      title: "Recent conversation",
      updated_at: new Date(2026, 9, 18, 10).toISOString(),
      sync_status: "incomplete",
    },
    older,
    {
      ...older,
      thread_id: "invalid",
      title: "No timestamp conversation",
      updated_at: "invalid",
    },
  ];
  await render();
  const selected = container.querySelector<HTMLAnchorElement>(
    'a[aria-current="page"]',
  )!;
  expect(selected.getAttribute("href")).toBe("/projects/p/chat/current");
  expect(selected.getAttribute("aria-label")).toBe("Current conversation");
  const descriptionId = selected.getAttribute("aria-describedby");
  expect(descriptionId).toBeTruthy();
  expect(document.getElementById(descriptionId!)?.textContent).toContain(
    "Updated Today",
  );
  expect(selected.querySelector("time")?.getAttribute("datetime")).toBe(
    current.updated_at,
  );
  const navigation = container.querySelector(
    '[aria-label="Your chat history"]',
  )!;
  for (const group of [
    "Today",
    "Yesterday",
    "Previous 7 days",
    "Earlier",
    "Other chats",
  ]) {
    expect(navigation.textContent).toContain(group);
  }
  expect(navigation.textContent).toContain("Update time unavailable");
  expect(navigation.textContent).toContain("Pending work");
  expect(navigation.textContent).toContain("Read only");
  expect(navigation.textContent).toContain("History incomplete");
  expect(navigation.textContent).not.toMatch(
    /Running|Completed|Failed|Research|Plan/,
  );
});

it("keeps an inaccessible pin removable without retaining its title or opening a chat", async () => {
  const key = 'colossus.web.chat-pins:v1:["alice","p"]';
  localStorage.setItem(key, JSON.stringify({ version: 1, ids: ["missing"] }));
  threads = [];
  const initial = vi.mocked(fetch).getMockImplementation()!;
  vi.mocked(fetch).mockImplementation(async (path, init) =>
    String(path).endsWith("/metadata")
      ? Response.json({ error: "not_found" }, { status: 404 })
      : initial(path, init),
  );
  await render();
  expect(container.textContent).toContain("Pinned chat unavailable.");
  expect(container.textContent).not.toContain("Start a conversation");
  await act(() => button("Remove pin").click());
  expect(JSON.parse(localStorage.getItem(key)!).ids).toEqual([]);
  expect(container.textContent).not.toContain("Pinned chat unavailable.");
  expect(document.activeElement?.getAttribute("aria-label")).toBe("New chat");
  expect(
    vi.mocked(fetch).mock.calls.every(([, init]) => init?.method === "GET"),
  ).toBe(true);
});
