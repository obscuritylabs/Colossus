import { expect, it } from "vitest";
import type { Task, Update } from "@colossus/control-plane-client";
import { taskReleasedOutput } from "./released-output";
const task = (status: string, terminal: unknown): Task => ({
  task_id: "task",
  project_id: "project",
  node_id: "node",
  subject: "user",
  request: { input: [], role: "primary", mode: "execute" },
  run_id: "run",
  last_sequence: 2,
  revision: 1,
  snapshot: {
    run: {
      run_id: "run",
      title: "Task",
      status,
      created_at: "",
      updated_at: "",
      role: "primary",
      mode: "execute",
      last_sequence: 2,
      terminal,
    },
    pending_interactions: [],
  },
});
const updates: Update[] = [
  {
    run_id: "run",
    sequence: 1,
    created_at: "",
    update: { output_delta: "Partial retained output" },
  },
];
it("prefers canonical terminal output when the result event is outside the bounded window", () => {
  expect(
    taskReleasedOutput(
      task("completed", { result: { output: "Final released result" } }),
      updates,
    ),
  ).toBe("Final released result");
  expect(
    taskReleasedOutput(
      task("completed", { output: "Legacy released result" }),
      updates,
    ),
  ).toBe("Legacy released result");
  expect(
    taskReleasedOutput(task("completed", { result: { output: "" } }), updates),
  ).toBe("");
});
it("uses only typed released output and keeps live output until terminal confirmation", () => {
  expect(
    taskReleasedOutput(
      task("running", { result: { output: "Unconfirmed result" } }),
      updates,
    ),
  ).toBe("Partial retained output");
  for (const terminal of [null, [], "invalid", { result: { output: 12 } }])
    expect(taskReleasedOutput(task("completed", terminal), updates)).toBe(
      "Partial retained output",
    );
});
