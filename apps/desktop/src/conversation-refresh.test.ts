import { expect, it, vi } from "vitest";
import { refreshConversation } from "./conversation-refresh";
import type { Run, RunDetails } from "./types";
const run = (id: string, sessionId = "session"): Run => ({
  runId: id,
  sessionId,
  title: "Original thread",
  role: "primary",
  mode: "execute",
  status: "completed",
  createdAt: "2026-10-09T00:00:00Z",
  updatedAt: "2026-10-09T00:00:01Z",
  startedAt: null,
  finishedAt: null,
  lastSequence: 3,
  pendingInteractionCount: 0,
  terminal: null,
  etag: "new",
  archived: false,
  controllable: false,
  continuable: true,
});
it("discovers web replies in the open conversation and hydrates their full prompt without reopening", async () => {
  const web = run("web-reply"),
    foreign = run("other", "other-session");
  const list = vi.fn(async (session?: string) => ({
    runs: session ? [web, foreign] : [web],
    nextPageToken: "",
  }));
  const details: RunDetails = {
    run: web,
    pendingInteractions: [],
    initialPrompt:
      "This full prompt came from the web, not the unchanged thread title.",
  };
  const hydrate = vi.fn(async () => details),
    onRun = vi.fn(),
    onRecent = vi.fn();
  await refreshConversation({
    sessionId: "session",
    isCurrent: () => true,
    list,
    hydrate,
    changed: () => true,
    onRun,
    onRecent,
  });
  expect(list.mock.calls.map(([session]) => session)).toEqual([
    undefined,
    "session",
  ]);
  expect(hydrate).toHaveBeenCalledExactlyOnceWith("web-reply");
  expect(onRun).toHaveBeenCalledExactlyOnceWith(details);
  expect(details.run.controllable).toBe(false);
});
it("drops late metadata and hydration after a target or session change", async () => {
  let current = true,
    finish: (value: RunDetails) => void = () => {};
  const details = new Promise<RunDetails>((resolve) => {
    finish = resolve;
  });
  const onRun = vi.fn();
  const pending = refreshConversation({
    sessionId: "session",
    isCurrent: () => current,
    list: async () => ({ runs: [run("web")], nextPageToken: "" }),
    hydrate: () => details,
    changed: () => true,
    onRecent: vi.fn(),
    onRun,
  });
  await Promise.resolve();
  await Promise.resolve();
  current = false;
  finish({ run: run("web"), pendingInteractions: [] });
  await pending;
  expect(onRun).not.toHaveBeenCalled();
});
it("does not repeatedly hydrate unchanged runs", async () => {
  const hydrate = vi.fn();
  await refreshConversation({
    sessionId: "session",
    isCurrent: () => true,
    list: async () => ({ runs: [run("known")], nextPageToken: "" }),
    hydrate,
    changed: () => false,
    onRecent: vi.fn(),
    onRun: vi.fn(),
  });
  expect(hydrate).not.toHaveBeenCalled();
});
