import assert from "node:assert/strict";
import { test } from "node:test";
import {
  Run,
  RunMode,
  RunStatus,
  PlanStatus,
  RunResult,
  SessionActivity,
  SessionActivityStatus,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";
import { ServerInfo } from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/system";
import { WorkController } from "../src/controller.js";
import type { WorkerClient } from "../src/connection.js";
import { activityView, plansFor, runView } from "../src/state.js";

const revision = 9007199254741009n;
const planRun = (id = "plan-run") =>
  Run.fromPartial({
    runId: id,
    sessionId: "session-1",
    title: "Improve workspace navigation",
    mode: RunMode.RUN_MODE_PLAN,
    status: RunStatus.RUN_STATUS_COMPLETED,
    createdAt: new Date("2026-10-01T12:00:00Z"),
    lastSequence: revision,
    etag: "native-only-etag",
    terminal: {
      $case: "result",
      value: RunResult.fromPartial({
        planId: "plan-1",
        planRevision: revision,
        planStatus: PlanStatus.PLAN_STATUS_DRAFT,
        output: "## Saved plan\n\n1. Add navigation\n2. Verify worker state",
      }),
    },
  });
function controller() {
  return new WorkController("fixture", {
    publish() {},
    remember: async () => {},
    interaction: async () => undefined,
  });
}
function worker(runs: Run[], activityEnabled = true) {
  const reads: string[] = [];
  let activityReads = 0;
  let reader = async (id: string) => ({
    run: runs.find((r) => r.runId === id),
    pendingInteractions: [],
  });
  const client = {
    info: ServerInfo.fromPartial({
      serverVersion: "0.11.7",
      capabilities: [{ name: "sessions.activity", enabled: activityEnabled }],
    }),
    close() {},
    listRuns: async () => ({ runs, page: { nextPageToken: "" } }),
    get: async (id: string) => {
      reads.push(id);
      return reader(id);
    },
    activity: async () => {
      activityReads++;
      return {
        activities: [
          SessionActivity.fromPartial({
            activityId: "a1",
            title: "Plan saved",
            summary: "Canonical plan revision saved",
            status: SessionActivityStatus.SESSION_ACTIVITY_STATUS_COMPLETED,
            input: { value: "never forward input", format: "json" },
            attributes: { internal: "never forward attributes" },
            result: { value: "Released result", format: "text" },
          }),
        ],
        caughtUp: true,
        page: { nextPageToken: "older" },
      };
    },
  };
  return {
    client: client as unknown as WorkerClient,
    reads,
    activityReads: () => activityReads,
    setReader(value: typeof reader) {
      reader = value;
    },
  };
}

test("plans require canonical terminal metadata and preserve exact revisions and latest lifecycle", () => {
  const plan = planRun();
  const execution = Run.fromPartial({
    ...plan,
    runId: "execute-run",
    mode: RunMode.RUN_MODE_EXECUTE,
    terminal: {
      $case: "result",
      value: {
        ...plan.terminal!.value,
        planStatus: PlanStatus.PLAN_STATUS_EXECUTED,
      },
    },
  });
  const prose = Run.fromPartial({
    runId: "prose",
    terminal: { $case: "result", value: { output: "Plan: do three things" } },
  });
  assert.equal(plansFor([prose]).length, 0);
  const plans = plansFor([execution, plan]);
  assert.equal(plans.length, 1);
  assert.equal(plans[0]!.status, "executed");
  assert.equal(plans[0]!.revision, revision.toString());
  assert.equal(plans[0]!.sourceRunId, "execute-run");
  assert.equal(runView(plan).sequence, revision.toString());
  assert.ok(!JSON.stringify(runView(plan)).includes("native-only-etag"));
});

test("inspection reads a known run and released activity, and rejects renderer-invented identities", async () => {
  const c = controller();
  const w = worker([planRun()]);
  await c.attach(w.client, "primary");
  const before = w.reads.length;
  await assert.rejects(c.inspect("not-listed", "run"), /listed run or plan/);
  assert.equal(w.reads.length, before);
  await c.inspect("plan-1", "plan");
  assert.equal(c.view.inspection?.plan?.revision, revision.toString());
  assert.match(c.view.inspection!.output, /Saved plan/);
  assert.equal(c.view.inspection?.activities[0]!.result, "Released result");
  assert.equal(c.view.inspection?.activityHasMore, true);
  const encoded = JSON.stringify(c.view);
  for (const secret of [
    "native-only-etag",
    "never forward input",
    "never forward attributes",
  ])
    assert.ok(!encoded.includes(secret));
  assert.equal(w.activityReads(), 1);
  assert.deepEqual(
    activityView(SessionActivity.fromPartial({ activityId: "empty" })).result,
    "",
  );
  c.detach();
});

test("activity capability absence does not call an unsupported RPC", async () => {
  const c = controller();
  const w = worker([planRun()], false);
  await c.attach(w.client, "primary");
  await c.inspect("plan-run", "run");
  assert.equal(w.activityReads(), 0);
  assert.match(c.view.inspection!.activityState, /unavailable/);
  c.detach();
});

test("disconnect clears resource bindings and discards in-flight inspection from the previous worker", async () => {
  const c = controller();
  const w = worker([planRun()]);
  await c.attach(w.client, "primary");
  let resolve!: (value: { run: Run; pendingInteractions: [] }) => void;
  w.setReader(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const reading = c.inspect("plan-run", "run");
  c.detach();
  const fresh = worker([]);
  await c.attach(fresh.client, "primary");
  resolve({ run: planRun(), pendingInteractions: [] });
  await reading;
  assert.equal(c.view.inspection, undefined);
  assert.equal(c.view.inspectionLoading, false);
  assert.equal(c.view.plans.length, 0);
  await assert.rejects(c.inspect("plan-run", "run"), /listed run or plan/);
  assert.equal(fresh.reads.length, 0);
  c.detach();
});

test("inspection rejects a response for a different session", async () => {
  const c = controller();
  const w = worker([planRun()]);
  await c.attach(w.client, "primary");
  w.setReader(async () => ({
    run: Run.fromPartial({ ...planRun(), sessionId: "other-session" }),
    pendingInteractions: [],
  }));
  await assert.rejects(c.inspect("plan-run", "run"), /no longer available/);
  assert.equal(c.view.inspection, undefined);
  assert.equal(w.activityReads(), 0);
  c.detach();
});

test("a plan inspection retains the canonical draft output after execution updates its lifecycle", async () => {
  const plan = planRun();
  assert.equal(plan.terminal?.$case, "result");
  if (plan.terminal?.$case !== "result") return;
  const execution = Run.fromPartial({
    ...plan,
    runId: "execution",
    mode: RunMode.RUN_MODE_EXECUTE,
    createdAt: new Date("2026-10-02T12:00:00Z"),
    terminal: {
      $case: "result",
      value: {
        ...plan.terminal.value,
        planStatus: PlanStatus.PLAN_STATUS_EXECUTED,
        output: "Implementation completed.",
      },
    },
  });
  const c = controller();
  const w = worker([execution, plan]);
  await c.attach(w.client, "primary");
  await c.inspect("plan-1", "plan");
  assert.equal(c.view.inspection?.plan?.status, "executed");
  assert.match(c.view.inspection!.output, /Saved plan/);
  assert.ok(w.reads.includes("plan-run"));
  c.detach();
});

test("older history uses a native cursor and merges sessions without duplicating runs", async () => {
  const c = controller();
  const w = worker([planRun()]);
  const calls: [string, string][] = [];
  w.client.listRuns = async (sessionId = "", cursor = "") => {
    calls.push([sessionId, cursor]);
    return {
      runs: [planRun(cursor ? "older-run" : "plan-run")],
      page: { nextPageToken: cursor ? "" : "native-only-cursor" },
    };
  };
  await c.attach(w.client, "primary");
  assert.equal(c.view.historyHasMore, true);
  assert.ok(!JSON.stringify(c.view).includes("native-only-cursor"));
  await c.refreshSessions(true);
  assert.deepEqual(calls, [
    ["", ""],
    ["", "native-only-cursor"],
  ]);
  assert.equal(c.view.sessions.length, 1);
  assert.equal(c.view.runs.length, 2);
  assert.equal(c.view.historyHasMore, false);
  c.detach();
});
