import { describe, expect, it } from "vitest";
import { boundRunUpdates } from "@colossus/control-plane-client";
import type { Task, Update } from "./api";
import { releasedRunUsage } from "./message-usage";

const event = (
  sequence: number,
  usage: unknown,
  run_id = "run",
  task_id = "task",
): Update => ({
  run_id,
  task_id,
  sequence,
  created_at: "2026-10-21T12:00:00Z",
  update: { usage },
});
describe("released run usage", () => {
  it("sums only bound, unique provider reports and rejects malformed counts", () => {
    const task = { task_id: "task", run_id: "run" } as Task;
    const usage = {
      input_tokens: 10,
      output_tokens: 3,
      total_tokens: 13,
      cached_input_tokens: 2,
    };
    const updates = boundRunUpdates(
      [task],
      [
        event(1, usage),
        event(1, usage),
        event(2, usage, "foreign"),
        event(3, usage, "run", "foreign"),
        event(4, { ...usage, input_tokens: -1 }),
        event(5, { ...usage, input_tokens: Number.MAX_SAFE_INTEGER + 1 }),
        event(6, { input_tokens: 2, output_tokens: 1, total_tokens: 3 }),
      ],
    ).get("task")!;
    expect(releasedRunUsage(updates)).toEqual({
      inputTokens: "12",
      outputTokens: "4",
      totalTokens: "16",
    });
    expect(releasedRunUsage([])).toBeNull();
    expect(releasedRunUsage([event(1, null)])).toBeNull();
  });
});
