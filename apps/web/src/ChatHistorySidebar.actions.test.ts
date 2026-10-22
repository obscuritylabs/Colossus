import { expect, it } from "vitest";
import {
  chatTitleError,
  latestChatForkSource,
} from "./ChatHistorySidebar.actions";
import type { Task, Thread, ThreadDetailResponse } from "./api";
const thread: Thread = {
  thread_id: "chat",
  project_id: "p",
  node_id: "n",
  owner_user_id: "alice",
  title: "Chat",
  created_at: "2026-10-21T08:00:00Z",
  updated_at: "2026-10-21T09:00:00Z",
  revision: 1,
  archived: false,
  session_id: "s",
  sync_status: "current",
  source: "cloud",
  can_continue: true,
};
const task: Task = {
  task_id: "task",
  project_id: "p",
  node_id: "n",
  thread_id: "chat",
  subject: "alice",
  request: { input: [{ text: "Run" }], mode: "act", role: "primary" },
  run_id: "run",
  revision: 1,
  last_sequence: 1,
  snapshot: {
    run: {
      run_id: "run",
      session_id: "s",
      title: "Run",
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
const detail = (
  overrides: Partial<ThreadDetailResponse> = {},
): ThreadDetailResponse => ({
  thread,
  tasks: [task],
  messages: [],
  ...overrides,
});
it("uses the newest owned terminal source and fresh thread revision", () => {
  expect(
    latestChatForkSource(
      thread,
      detail({
        thread: { ...thread, revision: 9 },
        tasks: [{ ...task, source_read_only: true }, task],
      }),
      "alice",
    ),
  ).toBe(task);
});
it("rejects mismatched bindings, ownership, archived or incomplete history and pending work", () => {
  for (const values of [
    { thread_id: "other" },
    { project_id: "other" },
    { node_id: "other" },
    { owner_user_id: "bob" },
    { session_id: "other" },
    { archived: true },
    { can_continue: false },
    { sync_status: "incomplete" as const },
    { active_task_id: "newer" },
    { queued_task_ids: ["newer"] },
  ])
    expect(
      latestChatForkSource(
        thread,
        detail({ thread: { ...thread, ...values } }),
        "alice",
      ),
    ).toBeNull();
  expect(latestChatForkSource(thread, detail(), "bob")).toBeNull();
});
it("rejects wrong task bindings and never skips unfinished work for an older terminal run", () => {
  for (const values of [
    { project_id: "other" },
    { node_id: "other" },
    { thread_id: "other" },
    { subject: "bob" },
    { run_id: "other" },
    { snapshot: null },
    { source_read_only: true },
    {
      snapshot: {
        ...task.snapshot!,
        run: { ...task.snapshot!.run, archived: true },
      },
    },
    {
      snapshot: {
        ...task.snapshot!,
        run: { ...task.snapshot!.run, session_id: "other" },
      },
    },
    {
      snapshot: {
        ...task.snapshot!,
        run: { ...task.snapshot!.run, status: "running" },
      },
    },
  ])
    expect(
      latestChatForkSource(
        thread,
        detail({ tasks: [{ ...task, ...values }] }),
        "alice",
      ),
    ).toBeNull();
  expect(
    latestChatForkSource(
      thread,
      detail({
        tasks: [
          { ...task, task_id: "new", run_id: null, snapshot: null },
          task,
        ],
      }),
      "alice",
    ),
  ).toBeNull();
  expect(
    latestChatForkSource(
      thread,
      detail({
        tasks: [
          {
            ...task,
            snapshot: {
              ...task.snapshot!,
              run: { ...task.snapshot!.run, status: "running" },
            },
          },
          task,
        ],
      }),
      "alice",
    ),
  ).toBeNull();
});
it("counts UTF-8 bytes and rejects empty or control-character titles", () => {
  expect(chatTitleError("🪐".repeat(64))).toBe("");
  expect(chatTitleError("🪐".repeat(65))).toContain("256");
  expect(chatTitleError(" \t ")).toContain("Enter");
  expect(chatTitleError("A\u0000B")).toContain("control");
  expect(chatTitleError("  Project chat  ")).toBe("");
});
