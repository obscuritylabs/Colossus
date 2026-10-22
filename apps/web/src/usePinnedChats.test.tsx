// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Thread } from "./api";
import { usePinnedChats } from "./usePinnedChats";

let root: Root, container: HTMLDivElement;
let ids: readonly string[], identity: string, project: string;
let latest: ReturnType<typeof usePinnedChats>;
let respond: (path: string, init: RequestInit) => Promise<Response>;
const rendered: { identity: string; threads: Thread[] }[] = [];
const thread = (id: string, owner = "alice"): Thread => ({
  thread_id: id,
  owner_user_id: owner,
  project_id: "project",
  node_id: "personal-node",
  title: `${owner}: ${id}`,
  created_at: "2026-10-22T08:00:00Z",
  updated_at: "2026-10-22T08:00:00Z",
  revision: 1,
  archived: false,
  source: "cloud",
  sync_status: "current",
  can_continue: true,
});
function Harness() {
  latest = usePinnedChats(identity, project, ids, 0);
  rendered.push({ identity, threads: latest.threads });
  return <>{latest.threads.map((value) => value.title).join(", ")}</>;
}
async function render() {
  await act(async () => root.render(<Harness />));
}
beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  ids = ["one"];
  identity = "alice";
  project = "project";
  rendered.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  respond = async (path) =>
    Response.json({
      thread: thread(decodeURIComponent(path.split("/").at(-2)!)),
    });
  vi.stubGlobal(
    "fetch",
    vi.fn((path: string, init: RequestInit) => respond(path, init)),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("resolves pins outside the current page through bounded metadata requests", async () => {
  ids = ["one", "two", "three", "four", "five", "six"];
  const completions = new Map<string, (response: Response) => void>();
  respond = (path) =>
    new Promise((resolve) => {
      completions.set(path.split("/").at(-2)!, resolve);
    });
  await render();
  expect(fetch).toHaveBeenCalledTimes(4);
  await act(async () =>
    completions.get("one")!(Response.json({ thread: thread("one") })),
  );
  expect(fetch).toHaveBeenCalledTimes(5);
  await act(async () => {
    for (const id of ["two", "three", "four", "five"])
      completions.get(id)!(Response.json({ thread: thread(id) }));
  });
  expect(fetch).toHaveBeenCalledTimes(6);
  await act(async () =>
    completions.get("six")!(Response.json({ thread: thread("six") })),
  );
  expect(latest.threads.map((value) => value.thread_id)).toEqual(ids);
  expect(latest.unavailable).toEqual([]);
  for (const [path, init] of vi.mocked(fetch).mock.calls) {
    expect(String(path)).toMatch(/\/threads\/[^/]+\/metadata$/);
    expect(init?.method).toBe("GET");
  }
});

it("rejects metadata for another owner, project, ID, or runtime source", async () => {
  ids = ["valid", "owner", "project", "id", "source", "missing"];
  respond = async (path) => {
    const id = path.split("/").at(-2)!;
    const value = thread(id);
    if (id === "missing")
      return Response.json({ error: "not_found" }, { status: 404 });
    if (id === "owner") value.owner_user_id = "bob";
    if (id === "project") value.project_id = "other";
    if (id === "id") value.thread_id = "different";
    if (id === "source") value.source = "runtime";
    return Response.json({ thread: value });
  };
  await render();
  expect(latest.threads.map((value) => value.thread_id)).toEqual(["valid"]);
  expect(latest.unavailable).toEqual(ids.slice(1));
});

it("fences account changes synchronously and drops late responses", async () => {
  let complete!: (response: Response) => void;
  respond = () =>
    new Promise((resolve) => {
      complete = resolve;
    });
  await render();
  const previous = complete;
  identity = "bob";
  respond = async () => Response.json({ thread: thread("one", "bob") });
  await render();
  expect(rendered.find((value) => value.identity === "bob")?.threads).toEqual(
    [],
  );
  expect(container.textContent).toBe("bob: one");
  await act(async () => previous(Response.json({ thread: thread("one") })));
  expect(container.textContent).toBe("bob: one");
});

it("does not restore an unpinned ID when its request finishes late", async () => {
  let complete!: (response: Response) => void;
  respond = () =>
    new Promise((resolve) => {
      complete = resolve;
    });
  await render();
  ids = [];
  await render();
  await act(async () => complete(Response.json({ thread: thread("one") })));
  expect(latest.threads).toEqual([]);
  expect(latest.unavailable).toEqual([]);
  expect(latest.loading).toBe(false);
});

it("refreshes on focus, clears inaccessible metadata, and pauses polling while hidden", async () => {
  await render();
  expect(container.textContent).toBe("alice: one");
  respond = async () => Response.json({ error: "not_found" }, { status: 404 });
  await act(async () => window.dispatchEvent(new Event("focus")));
  expect(latest.threads).toEqual([]);
  expect(latest.unavailable).toEqual(["one"]);
  const calls = vi.mocked(fetch).mock.calls.length;
  const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  await act(async () => vi.advanceTimersByTimeAsync(30_000));
  expect(fetch).toHaveBeenCalledTimes(calls);
  hidden.mockReturnValue(false);
  await act(async () => vi.advanceTimersByTimeAsync(30_000));
  expect(fetch).toHaveBeenCalledTimes(calls + 1);
});

it("bounds a stalled load and makes its IDs removable", async () => {
  respond = (_path, init) =>
    new Promise((_resolve, reject) => {
      init.signal?.addEventListener("abort", () =>
        reject(new DOMException("Aborted", "AbortError")),
      );
    });
  await render();
  expect(latest.loading).toBe(true);
  await act(async () => vi.advanceTimersByTimeAsync(10_000));
  expect(latest.loading).toBe(false);
  expect(latest.unavailable).toEqual(["one"]);
});
