import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Update } from "@colossus/control-plane-client";
import type { MobileClient, MobileStreamEvent } from "./client";
import { useTaskFeed } from "./useTaskFeed";

const hooks = vi.hoisted(() => ({
  effects: [] as (() => void | (() => void))[],
  published: [] as unknown[],
}));
vi.mock("react", () => ({
  useEffect: (effect: () => void | (() => void)) => hooks.effects.push(effect),
  useRef: <T>(current: T) => ({ current }),
  useState: <T>(initial: T) => {
    let value = initial;
    return [
      value,
      (next: T | ((previous: T) => T)) => {
        value =
          typeof next === "function"
            ? (next as (previous: T) => T)(value)
            : next;
        hooks.published.push(value);
      },
    ];
  },
}));
const update = (sequence: number): Update => ({
  run_id: "run-1",
  sequence,
  created_at: "2026-10-16T00:00:00Z",
  update: { output_delta: `chunk ${sequence}` },
});
const last = () =>
  hooks.published.at(-1) as {
    updates: Update[];
    error: string;
    connected: boolean;
  };
const flush = () => vi.advanceTimersByTimeAsync(0);
let cleanup: (() => void) | void;
let surface: EventTarget & { hidden: boolean };
beforeEach(() => {
  hooks.effects.length = 0;
  hooks.published.length = 0;
  vi.useFakeTimers();
  surface = Object.assign(new EventTarget(), { hidden: false });
  const focus = new EventTarget();
  vi.stubGlobal("document", surface);
  vi.stubGlobal("addEventListener", focus.addEventListener.bind(focus));
  vi.stubGlobal("removeEventListener", focus.removeEventListener.bind(focus));
});
afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function clientWith(
  request = vi.fn(async () => ({ updates: [], next_after: null })),
) {
  let listener: (event: MobileStreamEvent) => void = () => {};
  const stop = vi.fn(async () => {});
  const watch = vi.fn(async (_scope: unknown, receive: typeof listener) => {
    listener = receive;
    return stop;
  });
  return {
    client: { request, watch } as unknown as MobileClient,
    request,
    watch,
    stop,
    emit: (event: MobileStreamEvent) => listener(event),
  };
}
function start(client: MobileClient, task = "task-1", options = {}) {
  const hook = useTaskFeed(client, "project-1", task, options);
  cleanup = hooks.effects.at(-1)!();
  return hook;
}
describe("native activity lifecycle", () => {
  it("abandons retained pages overtaken by task streaming and normalizes task identity", async () => {
    let resolve!: (page: {
      updates: Update[];
      next_after: number | null;
    }) => void;
    const pending = new Promise<{
      updates: Update[];
      next_after: number | null;
    }>((done) => {
      resolve = done;
    });
    const request = vi
      .fn()
      .mockReturnValueOnce(pending)
      .mockResolvedValue({ updates: [], next_after: null });
    const fixture = clientWith(request);
    start(fixture.client);
    await flush();
    fixture.emit({ kind: "run_update", update: update(5), cursor: "5" });
    resolve({ updates: [update(1)], next_after: null });
    await flush();
    expect(last().updates.map((event) => event.sequence)).toEqual([5]);
    expect(last().updates[0]?.task_id).toBe("task-1");
    expect(request.mock.calls[1]?.[0]).toMatchObject({ after: 5 });
  });
  it("releases hidden watches, ignores late callbacks and resumes from precise cursor", async () => {
    const fixture = clientWith();
    start(fixture.client, "", { thread: "thread-1" });
    await flush();
    fixture.emit({
      kind: "run_update",
      update: { ...update(1), task_id: "task-1" },
      cursor: "9007199254740993",
    });
    surface.hidden = true;
    surface.dispatchEvent(new Event("visibilitychange"));
    fixture.emit({
      kind: "run_update",
      update: update(2),
      cursor: "9007199254740994",
    });
    expect(last().updates).toHaveLength(1);
    expect(fixture.stop).toHaveBeenCalledTimes(1);
    surface.hidden = false;
    surface.dispatchEvent(new Event("visibilitychange"));
    await flush();
    expect(fixture.watch.mock.calls[1]?.[0]).toMatchObject({
      after: "9007199254740993",
    });
  });
  it("cleans up a terminal error delivered before native watch admission resolves", async () => {
    const fixture = clientWith();
    const watch = vi.fn(
      async (_scope: unknown, receive: (event: MobileStreamEvent) => void) => {
        receive({
          kind: "error",
          code: "session_expired",
          message: "Sign in again",
        });
        return fixture.stop;
      },
    );
    fixture.client.watch = watch;
    const hook = start(fixture.client, "", { thread: "thread-1" });
    await flush();
    expect(fixture.stop).toHaveBeenCalledTimes(1);
    hook.refresh();
    await flush();
    expect(watch).toHaveBeenCalledTimes(1);
  });
  it("reconnects after a terminal transport error and refreshes lifecycle changes", async () => {
    const fixture = clientWith();
    const changed = vi.fn();
    const hook = start(fixture.client, "", {
      thread: "thread-1",
      onThreadChanged: changed,
    });
    await flush();
    fixture.emit({
      kind: "run_update",
      update: { ...update(1), update: { failure: {} } },
      cursor: "1",
    });
    expect(changed).toHaveBeenCalledTimes(1);
    fixture.emit({
      kind: "error",
      code: "unavailable",
      message: "Disconnected",
    });
    hook.refresh();
    await flush();
    expect(fixture.watch).toHaveBeenCalledTimes(2);
    fixture.emit({ kind: "error", code: "forbidden", message: "Denied" });
    hook.refresh();
    await flush();
    expect(fixture.watch).toHaveBeenCalledTimes(2);
  });
});
