// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MAX_PINNED_CHATS, useChatPins } from "./chat-pins";

let root: Root;
let container: HTMLDivElement;
let current: ReturnType<typeof useChatPins>;
let identity: string;
let project: string;
let sequence = 0;
let rendered: { identity: string; project: string; ids: readonly string[] }[];
function key(user = identity, scope = project) {
  return "colossus.web.chat-pins:v1:" + JSON.stringify([user, scope]);
}
function save(ids: unknown[], user = identity, scope = project) {
  window.localStorage.setItem(
    key(user, scope),
    JSON.stringify({ version: 1, ids }),
  );
}
function Probe(props: {
  identity: string;
  project: string;
  onRender?: () => void;
}) {
  current = useChatPins(props.identity, props.project);
  rendered.push({
    identity: props.identity,
    project: props.project,
    ids: current.ids,
  });
  props.onRender?.();
  return null;
}
async function render(user = identity, scope = project, onRender?: () => void) {
  await act(() =>
    root.render(
      createElement(Probe, {
        identity: user,
        project: scope,
        ...(onRender ? { onRender } : {}),
      }),
    ),
  );
}
async function pin(id: string, pinned = true) {
  await act(() => current.setPinned(id, pinned));
}
async function storage(eventKey: string | null = key(), area?: Storage) {
  await act(() =>
    window.dispatchEvent(
      new StorageEvent("storage", {
        key: eventKey,
        storageArea: area ?? window.localStorage,
      }),
    ),
  );
}
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  window.localStorage.clear();
  identity = `pins-user-${++sequence}`;
  project = "pins-project";
  rendered = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

it("persists only scoped bounded IDs, with the newest pin first and explicit unpin", async () => {
  await render();
  await pin("first");
  await pin("second");
  expect(current.ids).toEqual(["second", "first"]);
  expect(JSON.parse(window.localStorage.getItem(key())!)).toEqual({
    version: 1,
    ids: ["second", "first"],
  });
  await pin("second");
  expect(current.ids).toEqual(["second", "first"]);
  await pin("second", false);
  expect(current.ids).toEqual(["first"]);
  await act(() => root.unmount());
  root = createRoot(container);
  await render();
  expect(current.ids).toEqual(["first"]);
  expect(current.notice).toBe("");
});

it("fences account and project switches immediately, including retained old row handlers", async () => {
  save(["alice-pin"]);
  await render();
  const alice = identity;
  const previous = current.setPinned;
  const bob = `${identity}-other`;
  await render(bob, project, () => previous("stale-write", true));
  expect(current.ids).toEqual([]);
  expect(
    rendered
      .filter((value) => value.identity === bob)
      .every((value) => !value.ids.length),
  ).toBe(true);
  expect(JSON.parse(window.localStorage.getItem(key(alice))!).ids).toEqual([
    "alice-pin",
  ]);
  expect(window.localStorage.getItem(key(bob))).toBeNull();
  await pin("bob-pin");
  expect(JSON.parse(window.localStorage.getItem(key(bob))!).ids).toEqual([
    "bob-pin",
  ]);
  await render(alice, "other-project");
  expect(current.ids).toEqual([]);
  await pin("other-project-pin");
  await render(alice);
  expect(current.ids).toEqual(["alice-pin"]);
});

it("parses malformed, oversized and unsupported persisted data without retaining content", async () => {
  const invalid = [
    "not json",
    "null",
    "[]",
    JSON.stringify({ version: 2, ids: ["ignored"] }),
    JSON.stringify({
      version: 1,
      ids: ["ignored"],
      title: "Never retain message data",
    }),
    "x".repeat(65_537),
  ];
  for (const [index, value] of invalid.entries()) {
    const scope = `malformed-${index}`;
    window.localStorage.setItem(key(identity, scope), value);
    await render(identity, scope);
    expect(current.ids).toEqual([]);
  }
  save([
    123,
    null,
    "\nunsafe",
    "../../other",
    "first",
    "first",
    "second",
    "x".repeat(257),
  ]);
  await render();
  expect(current.ids).toEqual(["first", "second"]);
  await pin("\nunsafe");
  await pin("../../other");
  expect(current.ids).toEqual(["first", "second"]);
});

it("accepts route-compatible periods, unicode and bounded 256-character IDs and scopes", async () => {
  const user = "用户.one";
  const scope = "项目.one";
  const longest = "界".repeat(256);
  save(["row.one", "线程", longest], user, scope);
  await render(user, scope);
  expect(current.ids).toEqual(["row.one", "线程", longest]);
  await pin("new.线程");
  expect(current.ids).toEqual(["new.线程", "row.one", "线程", longest]);
  for (const invalid of [
    ".",
    "..",
    "space id",
    "a/b",
    "a\\b",
    "a\u007f",
    "界".repeat(257),
  ])
    await pin(invalid);
  expect(current.ids).toEqual(["new.线程", "row.one", "线程", longest]);

  const manyLongIds = Array.from(
    { length: MAX_PINNED_CHATS },
    (_, index) => `${index}.` + "界".repeat(250),
  );
  const longScope = "项目".repeat(128);
  save(manyLongIds, user, longScope);
  await render(user, longScope);
  expect(current.ids).toEqual(manyLongIds);
  expect(current.ids).toHaveLength(MAX_PINNED_CHATS);
});

it("caps restored data and rejects a new pin at capacity without silently evicting", async () => {
  save(Array.from({ length: 40 }, (_, index) => `saved-${index}`));
  await render();
  expect(current.ids).toHaveLength(MAX_PINNED_CHATS);
  const before = [...current.ids];
  const writes = vi.spyOn(window.localStorage, "setItem");
  await pin("overflow");
  expect(current.ids).toEqual(before);
  expect(current.notice).toContain("32");
  expect(current.notice).toContain("Unpin");
  expect(writes).not.toHaveBeenCalled();
  await pin("saved-0", false);
  await pin("allowed");
  expect(current.ids).toHaveLength(MAX_PINNED_CHATS);
  expect(current.ids[0]).toBe("allowed");
  expect(current.ids).not.toContain("saved-0");
  expect(current.notice).toBe("");
});

it("reads latest storage before writing so a delayed other-tab addition is preserved", async () => {
  save(["original"]);
  await render();
  save(["other-tab", "original"]);
  // No storage event has reached this tab yet.
  await pin("this-tab");
  expect(current.ids).toEqual(["this-tab", "other-tab", "original"]);
  expect(JSON.parse(window.localStorage.getItem(key())!).ids).toEqual(
    current.ids,
  );
  save(["new-other-tab", ...current.ids]);
  await pin("original", false);
  expect(current.ids).toEqual(["new-other-tab", "this-tab", "other-tab"]);
});

it("honors matching local storage changes and clear events without replaying stale values", async () => {
  save(["first"]);
  await render();
  save(["newest", "first"]);
  await act(() =>
    window.dispatchEvent(
      new StorageEvent("storage", {
        key: key(),
        newValue: JSON.stringify({ version: 1, ids: ["stale"] }),
        storageArea: window.localStorage,
      }),
    ),
  );
  expect(current.ids).toEqual(["newest", "first"]);
  save(["unrelated-event"]);
  await storage(key("other-user"));
  expect(current.ids).toEqual(["newest", "first"]);
  await storage(key(), window.sessionStorage);
  expect(current.ids).toEqual(["newest", "first"]);
  await storage();
  expect(current.ids).toEqual(["unrelated-event"]);
  window.localStorage.removeItem(key());
  await storage(null);
  expect(current.ids).toEqual([]);
});

it("keeps pins working for the session after quota denial without overwriting durable data", async () => {
  save(["existing"]);
  await render();
  const writes = vi
    .spyOn(window.localStorage, "setItem")
    .mockImplementation(() => {
      throw new DOMException("Storage is full", "QuotaExceededError");
    });
  await pin("new-pin");
  expect(current.ids).toEqual(["new-pin", "existing"]);
  expect(current.notice).toContain("session only");
  await pin("existing", false);
  await pin("another-pin");
  expect(current.ids).toEqual(["another-pin", "new-pin"]);
  expect(writes).toHaveBeenCalledTimes(1);
  expect(JSON.parse(window.localStorage.getItem(key())!).ids).toEqual([
    "existing",
  ]);
  writes.mockRestore();
  await storage();
  expect(current.ids).toEqual(["another-pin", "new-pin"]);
  await render(identity, "another-project");
  await render();
  expect(current.ids).toEqual(["another-pin", "new-pin"]);
});

it("preserves existing session choices when browser storage reads become unavailable", async () => {
  save(["existing"]);
  await render();
  const reads = vi
    .spyOn(window.localStorage, "getItem")
    .mockImplementation(() => {
      throw new DOMException("Blocked", "SecurityError");
    });
  await pin("session-pin");
  expect(current.ids).toEqual(["session-pin", "existing"]);
  expect(current.notice).toContain("Browser storage is unavailable");
  await pin("existing", false);
  expect(current.ids).toEqual(["session-pin"]);
  reads.mockRestore();
});

it("works when access to the localStorage property itself is denied", async () => {
  const property = vi
    .spyOn(window, "localStorage", "get")
    .mockImplementation(() => {
      throw new DOMException("Blocked", "SecurityError");
    });
  await render();
  await pin("session-pin");
  expect(current.ids).toEqual(["session-pin"]);
  expect(current.notice).toContain("session only");
  await pin("session-pin", false);
  expect(current.ids).toEqual([]);
  property.mockRestore();
});

it("does not read or write pin storage for an absent or invalid authenticated scope", async () => {
  const reads = vi.spyOn(window.localStorage, "getItem");
  const writes = vi.spyOn(window.localStorage, "setItem");
  await render("", project);
  await pin("ignored");
  await render(identity, "../invalid");
  await pin("ignored");
  expect(current.ids).toEqual([]);
  expect(current.notice).toBe("");
  expect(reads).not.toHaveBeenCalled();
  expect(writes).not.toHaveBeenCalled();
});
