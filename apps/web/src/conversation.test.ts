import { describe, expect, it } from "vitest";
import { conversationProjection } from "./conversation";
import type { Task, ThreadDetailResponse, Update } from "./api";

const task = (id: string, created: string, text: string): Task => ({
  task_id: id,
  project_id: "p",
  node_id: "n",
  subject: "human",
  created_at: created,
  request: { input: [{ text }], mode: "execute", role: "primary" },
  run_id: `run-${id}`,
  snapshot: null,
  revision: 1,
  last_sequence: 1,
});
const output = (id: string, created: string, text: string): Update => ({
  task_id: id,
  run_id: `run-${id}`,
  sequence: 1,
  created_at: created,
  update: { result: { output: text } },
});
describe("released conversation chronology", () => {
  it("binds each turn's ordered activity to its exact task/run and fences duplicate or conflicting frames", () => {
    const early = task("first", "2026-10-05T10:00:00Z", "First request"),
      late = task("second", "2026-10-05T10:00:00Z", "Second request");
    const activity = (
      owner: Task,
      sequence: number,
      state: string,
    ): Update => ({
      task_id: owner.task_id,
      run_id: owner.run_id!,
      sequence,
      created_at: "2026-10-05T10:01:00Z",
      update: {
        tool_activity: {
          call_id: "same-call-id",
          tool_name: "echo",
          state,
          summary: `${owner.task_id} ${state}`,
        },
      },
    });
    const first = activity(early, 1, "requested"),
      second = activity(late, 1, "requested");
    const detail = {
      thread: { created_at: early.created_at, updated_at: late.created_at },
      tasks: [late, early],
      messages: [],
    } as unknown as ThreadDetailResponse;
    const view = conversationProjection(detail, [
      activity(late, 2, "completed"),
      second,
      { ...second, task_id: early.task_id },
      activity(early, 2, "completed"),
      first,
      { ...first, update: { result: { output: "Conflicting duplicate" } } },
      { ...first, task_id: "unmapped-task", run_id: "unmapped-delegated-run" },
    ]);
    expect(view.turns.map((turn) => turn.task?.task_id)).toEqual([
      "first",
      "second",
    ]);
    expect(
      view.turns.map((turn) =>
        turn.updates.map((item) => [item.run_id, item.sequence]),
      ),
    ).toEqual([
      [
        [early.run_id, 1],
        [early.run_id, 2],
      ],
      [
        [late.run_id, 1],
        [late.run_id, 2],
      ],
    ]);
    expect(view.messages.map((message) => message.text)).toEqual([
      "First request",
      "Second request",
    ]);
  });
  it("keeps partially loaded history visible without associating unmapped activity by timestamp", () => {
    const loaded = task("loaded", "2026-10-05T10:00:00Z", "Loaded request");
    const detail = {
      thread: { created_at: loaded.created_at, updated_at: loaded.created_at },
      tasks: [loaded],
      messages: [
        {
          message_id: "older-message",
          task_id: "not-loaded",
          role: "assistant",
          text: "Saved partial history",
          created_at: loaded.created_at,
        },
      ],
    } as unknown as ThreadDetailResponse;
    const view = conversationProjection(detail, [
      {
        ...output("not-loaded", loaded.created_at!, "Unmapped output"),
        run_id: loaded.run_id!,
      },
    ]);
    const older = view.turns.find((turn) => turn.key === "task-not-loaded")!;
    expect(older.task).toBeUndefined();
    expect(older.updates).toEqual([]);
    expect(older.messages[0]?.text).toBe("Saved partial history");
    expect(view.byTask.get(loaded.task_id)).toEqual([]);
  });
  it("interleaves completed turns from older SDK result feeds despite hash task IDs and newest-first detail pages", () => {
    const early = task("zz-first", "2026-10-05T10:00:00Z", "First request"),
      late = task("aa-second", "2026-10-05T10:02:00Z", "Second request");
    const detail = {
      thread: { created_at: early.created_at, updated_at: late.created_at },
      tasks: [late, early],
      messages: [],
    } as unknown as ThreadDetailResponse;
    const view = conversationProjection(detail, [
      output(late.task_id, "2026-10-05T10:03:00Z", "Second answer"),
      output(early.task_id, "2026-10-05T10:01:00Z", "First answer"),
    ]);
    expect(view.messages.map((message) => message.text)).toEqual([
      "First request",
      "First answer",
      "Second request",
      "Second answer",
    ]);
  });
  it("deduplicates authoritative assistant messages and never invents human input from a read-only imported run title", () => {
    const imported = {
      ...task("import", "2026-10-05T10:00:00Z", "Source run title"),
      source_read_only: true,
      subject: "runtime",
    };
    const detail = {
      thread: {
        created_at: imported.created_at,
        updated_at: imported.created_at,
      },
      tasks: [imported],
      messages: [
        {
          message_id: "authoritative",
          role: "assistant",
          text: "Released answer",
          created_at: "2026-10-05T10:01:00Z",
          task_id: imported.task_id,
        },
      ],
    } as unknown as ThreadDetailResponse;
    const view = conversationProjection(detail, [
      output(imported.task_id, "2026-10-05T10:01:00Z", "Released answer"),
    ]);
    expect(view.messages).toHaveLength(1);
    expect(view.messages[0]?.message_id).toBe("authoritative");
  });
});
