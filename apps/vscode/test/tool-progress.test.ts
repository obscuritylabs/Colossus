import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RunUpdate,
  Run,
  RunStatus,
  ToolActivityState,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";
import { toolProgress } from "../src/tool-progress.js";
import { WorkController } from "../src/controller.js";
import type { WorkerClient } from "../src/connection.js";
import { ServerInfo } from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/system";

function event(
  runId: string,
  sequence: bigint,
  state: ToolActivityState,
  extra = {},
) {
  return RunUpdate.fromPartial({
    runId,
    sequence,
    createdAt: new Date("2026-10-04T10:00:00Z"),
    update: {
      $case: "toolActivity",
      value: {
        callId: "same-call",
        toolName: "shell.run",
        state,
        summary: "Released status",
        ...extra,
      },
    },
  });
}

test("tool progress scopes calls to runs and ignores replayed or older uint64 sequences", () => {
  const seq = 9007199254740993n;
  const first = toolProgress(
    [],
    event("run-a", seq, ToolActivityState.TOOL_ACTIVITY_STATE_STARTED, {
      input: "Released input",
    }),
  );
  const second = toolProgress(
    first,
    event("run-b", 1n, ToolActivityState.TOOL_ACTIVITY_STATE_REQUESTED),
  );
  assert.equal(second.length, 2);
  assert.equal(
    toolProgress(
      second,
      event("run-a", seq, ToolActivityState.TOOL_ACTIVITY_STATE_FAILED),
    ),
    second,
  );
  assert.equal(
    toolProgress(
      second,
      event("run-a", seq - 1n, ToolActivityState.TOOL_ACTIVITY_STATE_FAILED),
    ),
    second,
  );
  const done = toolProgress(
    second,
    event("run-a", seq + 1n, ToolActivityState.TOOL_ACTIVITY_STATE_COMPLETED, {
      preview: "Released result",
    }),
  );
  assert.equal(done[0]?.input, "Released input");
  assert.equal(done[0]?.preview, "Released result");
  assert.deepEqual(
    done[0]?.history?.map((step) => step.state),
    ["started", "completed"],
  );
  assert.equal(done[1]?.state, "requested");
  const unknown = toolProgress(
    done,
    event(
      "run-a",
      seq + 2n,
      ToolActivityState.TOOL_ACTIVITY_STATE_OUTCOME_UNKNOWN,
      { preview: "Must not present as success" },
    ),
  );
  assert.equal(unknown[0]?.preview, undefined);
  assert.equal(unknown[0]?.state, "outcome unknown");
});

test("tool details and retained progress are bounded; unnamed calls stay distinct", () => {
  let tools = toolProgress(
    [],
    event("run-a", 1n, ToolActivityState.TOOL_ACTIVITY_STATE_STARTED, {
      summary: "s".repeat(10000),
      input: "i".repeat(10000),
    }),
  );
  assert.equal(tools[0]?.summary.length, 2048);
  assert.equal(tools[0]?.input?.length, 8192);
  for (let n = 2; n < 15; n++)
    tools = toolProgress(
      tools,
      event("run-a", BigInt(n), ToolActivityState.TOOL_ACTIVITY_STATE_STARTED),
    );
  assert.equal(tools[0]?.history?.length, 8);
  for (let n = 15; n < 120; n++)
    tools = toolProgress(
      tools,
      event("run-a", BigInt(n), ToolActivityState.TOOL_ACTIVITY_STATE_STARTED, {
        callId: "",
      }),
    );
  assert.equal(tools.length, 100);
  assert.equal(new Set(tools.map((tool) => tool.id)).size, 100);
});

test("restoring older feeds keeps the latest turn's calls within the conversation limit", async () => {
  const runs = ["recent", "older"].map((runId, index) =>
    Run.fromPartial({
      runId,
      sessionId: "session-1",
      title: runId,
      createdAt: new Date(`2026-10-0${4 - index}T10:00:00Z`),
      status: RunStatus.RUN_STATUS_COMPLETED,
      terminal: { $case: "result", value: { output: `${runId} response` } },
    }),
  );
  const watched: string[] = [];
  const client = {
    info: ServerInfo.fromPartial({ serverVersion: "fixture" }),
    close() {},
    listRuns: async () => ({ runs }),
    get: async (id: string) => ({
      run: runs.find((run) => run.runId === id),
      pendingInteractions: [],
    }),
    async *watch(id: string) {
      watched.push(id);
      const count = id === "recent" ? 20 : 90;
      for (let n = 0; n < count; n++) {
        const value = event(
          id,
          BigInt(n + 1),
          ToolActivityState.TOOL_ACTIVITY_STATE_COMPLETED,
          { callId: `call-${n}` },
        );
        yield { runId: id, sequence: value.sequence, value };
      }
      const value = RunUpdate.fromPartial({
        runId: id,
        sequence: BigInt(count + 1),
        update: { $case: "result", value: { output: `${id} response` } },
      });
      yield { runId: id, sequence: value.sequence, value };
    },
    create() {
      throw new Error("History must never execute work");
    },
  } as unknown as WorkerClient;
  const controller = new WorkController("fixture", {
    publish() {},
    remember: async () => {},
    interaction: async () => undefined,
  });
  try {
    await controller.attach(client, "primary", "session-1");
    assert.deepEqual(watched, ["recent", "older"]);
    assert.equal(controller.view.tools.length, 100);
    assert.equal(
      controller.view.tools.filter((tool) => tool.runId === "recent").length,
      20,
    );
    assert.equal(
      controller.view.tools.filter((tool) => tool.runId === "older").length,
      80,
    );
    assert.deepEqual(
      controller.view.messages
        .filter((message) => message.role === "assistant")
        .map((message) => message.text),
      ["older response", "recent response"],
    );
  } finally {
    controller.detach();
  }
});
