import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import * as grpc from "@grpc/grpc-js";
import { certificateSha256 } from "@obscuritylabs/colossus-sdk";
import {
  ColossusErrorDetail,
  OutcomeCertainty,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/common";
import { Status } from "@obscuritylabs/colossus-sdk/gen/google/rpc/status";
import {
  AgentRunServiceService,
  Interaction,
  InteractionStatus,
  Run,
  RunStatus,
  RunUpdate,
  RunResult,
  RunCancellation,
  ToolActivity,
  ToolActivityState,
  type AgentRunServiceServer,
  type WatchRunRequest,
  type WatchRunResponse,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";

import {
  ServerInfo,
  SystemServiceService,
  DeploymentMode,
  type SystemServiceServer,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/system";
import { connectWorker, workspaceIdentity } from "../src/connection.js";
import { WorkController, type ControllerHost } from "../src/controller.js";
import { ConnectionError, type ConnectionDiagnostic } from "../src/errors.js";

async function until(condition: () => boolean) {
  const deadline = Date.now() + 5000;
  while (!condition()) {
    if (Date.now() > deadline)
      throw new Error("Timed out waiting for observable behavior");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

async function fixture() {
  const home = await mkdtemp(join(tmpdir(), "colossus-vscode-runtime-"));
  const workspace = join(home, "workspace");
  const discovery = join(home, "api");
  await mkdir(workspace);
  await mkdir(discovery, { mode: 0o700 });
  const certificate = await readFile(
    new URL("../../../../sdk/testdata/connector-cert.pem", import.meta.url),
  );
  const key = await readFile(
    new URL("../../../../sdk/testdata/connector-key.pem", import.meta.url),
  );
  const instanceId = "00000000-0000-4000-8000-000000000001";
  const server = new grpc.Server();
  let creates = 0;
  let answers = 0;
  let cancelRequests = 0;
  let watchStarts = 0;
  let rejectHistory = false;
  let lists = 0;
  let listFailure: grpc.ServiceError | undefined;
  let listFailuresRemaining = 0;
  let uncertainCreate = false;
  let uncertainAnswer = false;
  let rejectAuthentication = false;
  let handshakes = 0;
  let run = Run.fromPartial({
    runId: "run-1",
    sessionId: "session-1",
    status: RunStatus.RUN_STATUS_RUNNING,
    title: "Inspect code",
  });
  const pending = Interaction.fromPartial({
    interactionId: "interaction-1",
    runId: "run-1",
    status: InteractionStatus.INTERACTION_STATUS_PENDING,
    respondableByCaller: true,
    etag: "current-etag",
    content: {
      $case: "approval",
      value: {
        reason: "Check code",
        action: "process.execute",
        resource: "workspace",
        requestHash: "one-use-binding",
        commandContext: {
          executable: "/bin/echo",
          arguments: ["two  spaces", ""],
          justification: "Inspect code",
          workingDirectory: "/workspace",
          redacted: true,
        },
      },
    },
  });
  let waiting: Interaction[] = [pending];
  const history: RunUpdate[] = [];
  const watchers = new Set<
    grpc.ServerWritableStream<WatchRunRequest, WatchRunResponse>
  >();
  const auth = (call: { metadata: grpc.Metadata }) =>
    assert.deepEqual(call.metadata.get("authorization"), [
      "Bearer non-secret-fixture-token",
    ]);
  function emit(update: RunUpdate["update"]) {
    const event = RunUpdate.fromPartial({
      runId: run.runId,
      sequence: BigInt(history.length + 1),
      createdAt: new Date(),
      update,
    });
    history.push(event);
    run.lastSequence = event.sequence;
    for (const watcher of watchers) watcher.write({ update: event });
  }
  function finish(cancelled = false) {
    waiting = [];
    if (cancelled) {
      const value = RunCancellation.fromPartial({ message: "Stopped" });
      run.terminal = { $case: "cancellation", value };
      run.status = RunStatus.RUN_STATUS_CANCELLED;
      emit({ $case: "cancellation", value });
    } else {
      const value = RunResult.fromPartial({
        output: "Working done.",
        profile: "primary",
        modelProfile: "primary",
        providerProfile: "offline",
      });
      run.terminal = { $case: "result", value };
      run.status = RunStatus.RUN_STATUS_COMPLETED;
      emit({ $case: "result", value });
    }
    for (const watcher of watchers) watcher.end();
    watchers.clear();
  }
  server.addService(SystemServiceService, {
    getServerInfo(call, callback) {
      auth(call);
      handshakes++;
      if (rejectAuthentication) {
        callback({
          code: grpc.status.UNAUTHENTICATED,
          message: "non-secret-fixture-token private-path",
          details: "non-secret-fixture-token private-path",
        });
        return;
      }
      callback(null, {
        serverInfo: ServerInfo.fromPartial({
          instanceId,
          apiPackages: ["colossus.api.v1alpha1"],
          serverVersion: "0.11.6",
          deploymentMode: DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
        }),
      });
    },
  } satisfies Partial<SystemServiceServer>);
  server.addService(AgentRunServiceService, {
    createRun(call, callback) {
      auth(call);
      creates++;
      assert.equal(call.request.role, "primary");
      assert.ok(call.request.idempotencyKey);
      assert.ok(call.request.input.length > 0);
      emit({ $case: "state", value: { status: RunStatus.RUN_STATUS_RUNNING } });
      emit({ $case: "outputDelta", value: { text: "Working " } });
      emit({ $case: "interaction", value: pending });
      if (uncertainCreate)
        callback({
          code: grpc.status.UNAVAILABLE,
          details: "Unknown fixture outcome",
          name: "Error",
          message: "Unknown",
        });
      else callback(null, { run });
    },
    getRun(call, callback) {
      auth(call);
      assert.equal(call.request.runId, run.runId);
      callback(null, { run, pendingInteractions: waiting });
    },
    listRuns(call, callback) {
      auth(call);
      lists++;
      if (listFailuresRemaining-- > 0) {
        callback(listFailure!);
        return;
      }
      assert.ok(
        call.request.sessionId === undefined ||
          call.request.sessionId.length > 0,
      );
      callback(null, { runs: creates ? [run] : [], page: undefined });
    },
    watchRun(call) {
      auth(call);
      watchStarts++;
      if (rejectHistory && run.terminal) {
        call.destroy(
          Object.assign(new Error("private-history-error"), {
            code: grpc.status.PERMISSION_DENIED,
          }),
        );
        return;
      }
      for (const event of history)
        if (event.sequence > call.request.afterSequence)
          call.write({ update: event });
      if (run.terminal) call.end();
      else {
        watchers.add(call);
        call.on("cancelled", () => watchers.delete(call));
      }
    },
    respondInteraction(call, callback) {
      auth(call);
      answers++;
      assert.equal(call.request.runId, pending.runId);
      assert.equal(call.request.etag, pending.etag);
      assert.equal(call.request.response?.$case, "approvalAnswer");
      if (call.request.response?.$case === "approvalAnswer")
        assert.equal(
          call.request.response.value.requestHash,
          "one-use-binding",
        );
      finish();
      if (uncertainAnswer)
        callback({
          code: grpc.status.UNAVAILABLE,
          details: "Unknown fixture outcome",
          name: "Error",
          message: "Unknown",
        });
      else
        callback(null, {
          interaction: Interaction.fromPartial({
            ...pending,
            status: InteractionStatus.INTERACTION_STATUS_ANSWERED,
          }),
        });
    },
    cancelRun(call, callback) {
      auth(call);
      cancelRequests++;
      callback(null, { run });
    },
  } satisfies Partial<AgentRunServiceServer>);
  const port = await new Promise<number>((resolve, reject) =>
    server.bindAsync(
      "127.0.0.1:0",
      grpc.ServerCredentials.createSsl(null, [
        { cert_chain: certificate, private_key: key },
      ]),
      (error, port) => (error ? reject(error) : resolve(port)),
    ),
  );
  const identity = await workspaceIdentity(workspace);
  const pin = certificateSha256(certificate);
  const profile = {
    schemaVersion: 1 as const,
    workspacePath: identity.path,
    workspaceIdentity: identity.identity,
    discoveryDirectory: discovery,
    instanceId,
    certificateSha256: pin,
    keyringService: "test.service",
    keyringAccount: "test",
    role: "primary",
  };
  await writeFile(
    join(discovery, "endpoint.json"),
    JSON.stringify({
      schema_version: 1,
      api_version: "colossus.api.v1alpha1",
      instance_id: instanceId,
      endpoint: `https://127.0.0.1:${port}`,
      pid: process.pid,
      certificate_sha256: pin,
    }),
    { mode: 0o600 },
  );
  await writeFile(join(discovery, "certificate.pem"), certificate, {
    mode: 0o600,
  });
  let remembered = "";
  const host: ControllerHost = {
    publish(view) {
      assert.doesNotMatch(
        JSON.stringify(view),
        /non-secret-fixture-token|one-use-binding|current-etag|certificateSha256|discoveryDirectory/u,
      );
    },
    async remember(id) {
      remembered = id;
    },
    async interaction(value) {
      assert.deepEqual(
        value.content?.$case === "approval"
          ? value.content.value.commandContext?.arguments
          : [],
        ["two  spaces", ""],
      );
      return {
        $case: "approvalAnswer",
        value: { approved: true, requestHash: "one-use-binding" },
      };
    },
  };
  const controllers: WorkController[] = [];
  async function controller(sessionId = "") {
    const result = new WorkController("Workspace", host);
    controllers.push(result);
    await result.attach(
      await connectWorker(profile, async () => "non-secret-fixture-token"),
      "primary",
      sessionId,
    );
    return result;
  }
  return {
    controller,
    connect(report?: (event: ConnectionDiagnostic) => void) {
      return connectWorker(
        profile,
        async () => "non-secret-fixture-token",
        report,
      );
    },
    rejectAuthentication() {
      rejectAuthentication = true;
    },
    get handshakes() {
      return handshakes;
    },
    failLists(error: grpc.ServiceError, count: number) {
      listFailure = error;
      listFailuresRemaining = count;
    },
    get lists() {
      return lists;
    },
    finish,
    emit,
    rejectToolHistory() {
      rejectHistory = true;
    },
    run,
    watchers,
    setUncertainCreate() {
      uncertainCreate = true;
    },
    setUncertainAnswer() {
      uncertainAnswer = true;
    },
    get creates() {
      return creates;
    },
    get answers() {
      return answers;
    },
    get cancelRequests() {
      return cancelRequests;
    },
    get watchStarts() {
      return watchStarts;
    },
    get remembered() {
      return remembered;
    },
    async close() {
      controllers.forEach((c) => c.detach());
      server.forceShutdown();
      await rm(home, { recursive: true, force: true });
    },
  };
}

test("a real authenticated worker rejection identifies the handshake without leaking transport details", async () => {
  const f = await fixture();
  const events: ConnectionDiagnostic[] = [];
  try {
    f.rejectAuthentication();
    await assert.rejects(
      f.connect((event) => events.push(event)),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "handshake");
        assert.equal(error.reason, "unauthenticated");
        assert.doesNotMatch(
          error.message,
          /non-secret-fixture-token|private-path/u,
        );
        return true;
      },
    );
    assert.equal(f.handshakes, 1);
    assert.deepEqual(events.at(-1), {
      stage: "handshake",
      state: "failed",
      reason: "unauthenticated",
    });
    assert.doesNotMatch(
      JSON.stringify(events),
      /non-secret-fixture-token|private-path/u,
    );
  } finally {
    await f.close();
  }
});

test("pinned authenticated client streams, restores approvals across disconnect, and recovers released run history", async () => {
  const f = await fixture();
  try {
    const first = await f.controller();
    await first.send("Inspect code", "plan");
    await until(
      () =>
        first.view.interactions.length === 1 &&
        first.view.messages.some((m) => m.text === "Working "),
    );
    await assert.rejects(first.send("Second task", "execute"));
    first.detach();
    const resumed = await f.controller(f.remembered);
    await until(
      () =>
        resumed.view.interactions.length === 1 &&
        resumed.view.messages.some((m) => m.text === "Working "),
    );
    assert.equal(f.creates, 1);
    await resumed.respond("interaction-1");
    await until(
      () =>
        !resumed.view.busy &&
        resumed.view.messages.some((m) => m.text === "Working done."),
    );
    assert.equal(f.answers, 1);
    assert.equal(resumed.view.interactions.length, 0);
    assert.equal(
      resumed.view.messages.filter((m) => m.text === "Working done.").length,
      1,
    );
  } finally {
    await f.close();
  }
});

test("history reads recover from bounded admission bursts and preserve permanent failures", async () => {
  const f = await fixture();
  const client = await f.connect();
  const metadata = new grpc.Metadata();
  const detail = ColossusErrorDetail.fromPartial({
    reason: "capacity_exceeded",
    retryable: true,
    outcomeCertainty: OutcomeCertainty.OUTCOME_CERTAINTY_KNOWN,
  });
  metadata.set(
    "grpc-status-details-bin",
    Buffer.from(
      Status.encode({
        code: grpc.status.RESOURCE_EXHAUSTED,
        message: "Admission temporarily exhausted",
        details: [
          {
            typeUrl:
              "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail",
            value: Buffer.from(ColossusErrorDetail.encode(detail).finish()),
          },
        ],
      }).finish(),
    ),
  );
  const capacity = Object.assign(new Error("Admission temporarily exhausted"), {
    code: grpc.status.RESOURCE_EXHAUSTED,
    details: "Admission temporarily exhausted",
    metadata,
  });
  try {
    f.failLists(capacity, 2);
    assert.deepEqual((await client.listRuns()).runs, []);
    assert.equal(f.lists, 3);
    f.failLists(capacity, 10);
    await assert.rejects(client.listRuns());
    assert.equal(f.lists, 7);
    f.failLists(
      Object.assign(new Error("Permission denied"), {
        code: grpc.status.PERMISSION_DENIED,
        details: "Permission denied",
        metadata: new grpc.Metadata(),
      }),
      10,
    );
    await assert.rejects(client.listRuns());
    assert.equal(f.lists, 8);
    assert.equal(f.creates, 0);
  } finally {
    client.close();
    await f.close();
  }
});

test("cancellation waits for durable terminal evidence and never creates another run", async () => {
  const f = await fixture();
  try {
    const c = await f.controller();
    await c.send("Inspect code", "execute");
    await until(() => c.view.watching && f.watchers.size > 0);
    await c.stop();
    assert.equal(f.cancelRequests, 1);
    assert.equal(c.view.busy, true);
    await assert.rejects(c.newSession());
    f.finish(true);
    await until(() => !c.view.busy);
    assert.equal(c.view.status, "Cancelled");
    assert.equal(f.creates, 1);
  } finally {
    await f.close();
  }
});

test("unknown creation outcome blocks another effect until explicit connection reconciliation", async () => {
  const f = await fixture();
  try {
    f.setUncertainCreate();
    const c = await f.controller();
    await c.send("Inspect code", "execute");
    assert.equal(c.view.busy, true);
    await assert.rejects(c.send("Retry", "execute"));
    assert.equal(f.creates, 1);
    c.detach();
    const recovered = await f.controller();
    await recovered.refreshSessions();
    await recovered.selectSession("session-1");
    await until(() => recovered.view.interactions.length === 1);
    assert.equal(f.creates, 1);
  } finally {
    await f.close();
  }
});

test("an uncertain interaction response is reconciled without sending the action again", async () => {
  const f = await fixture();
  try {
    f.setUncertainAnswer();
    const c = await f.controller();
    await c.send("Inspect code", "plan");
    await until(() => c.view.interactions.length === 1);
    await c.respond("interaction-1");
    await until(() => !c.view.busy);
    await assert.rejects(c.respond("interaction-1"));
    assert.equal(f.answers, 1);
  } finally {
    await f.close();
  }
});

test("live tool lifecycle remains in its turn and is restored from a terminal feed without repeating effects", async () => {
  const f = await fixture();
  const activity = (state: ToolActivityState, summary: string, extra = {}) =>
    f.emit({
      $case: "toolActivity",
      value: ToolActivity.fromPartial({
        callId: "shell-1",
        toolName: "shell.run",
        state,
        summary,
        ...extra,
      }),
    });
  try {
    const first = await f.controller();
    await first.send("Inspect code", "execute");
    await until(() => first.view.messages.some((m) => m.text === "Working "));
    activity(
      ToolActivityState.TOOL_ACTIVITY_STATE_REQUESTED,
      "Preparing command",
    );
    activity(
      ToolActivityState.TOOL_ACTIVITY_STATE_WAITING_APPROVAL,
      "Waiting for approval",
    );
    activity(ToolActivityState.TOOL_ACTIVITY_STATE_STARTED, "Command running", {
      input: '{"command":"echo ready"}',
    });
    await until(() => first.view.tools[0]?.state === "started");
    assert.equal(first.view.tools.length, 1);
    assert.equal(first.view.tools[0]?.runId, "run-1");
    first.detach();
    const resumed = await f.controller(f.remembered);
    await until(() => resumed.view.tools[0]?.state === "started");
    assert.equal(resumed.view.tools[0]?.history?.length, 3);
    activity(
      ToolActivityState.TOOL_ACTIVITY_STATE_COMPLETED,
      "Command exited",
      { preview: "ready\n" },
    );
    f.finish();
    await until(
      () =>
        !resumed.view.busy &&
        resumed.view.messages.some((m) => m.text === "Working done."),
    );
    assert.equal(resumed.view.tools[0]?.state, "completed");
    assert.equal(resumed.view.tools[0]?.history?.length, 4);
    resumed.detach();
    const restored = await f.controller(f.remembered);
    assert.equal(restored.view.tools[0]?.preview, "ready\n");
    assert.equal(restored.view.tools[0]?.input, '{"command":"echo ready"}');
    assert.equal(restored.view.tools[0]?.history?.length, 4);
    assert.equal(restored.view.tools.length, 1);
    assert.equal(
      restored.view.messages.every((m) => m.runId === "run-1"),
      true,
    );
    assert.equal(f.creates, 1);
    assert.equal(f.answers, 0);
    assert.equal(f.cancelRequests, 0);
  } finally {
    await f.close();
  }
});

test("unavailable historical tool feeds leave the canonical response visible with an honest notice", async () => {
  const f = await fixture();
  try {
    const first = await f.controller();
    await first.send("Inspect code", "plan");
    await until(() => first.view.messages.some((m) => m.text === "Working "));
    f.finish();
    await until(
      () =>
        !first.view.busy &&
        first.view.messages.some((m) => m.text === "Working done."),
    );
    first.detach();
    f.rejectToolHistory();
    const restored = await f.controller(f.remembered);
    assert.equal(
      restored.view.messages.some((m) => m.text === "Working done."),
      true,
    );
    assert.equal(
      restored.view.messages.some(
        (m) =>
          m.role === "notice" &&
          m.text.includes("tool progress could not be loaded"),
      ),
      true,
    );
    assert.equal(restored.view.error, "");
    assert.equal(f.creates, 1);
  } finally {
    await f.close();
  }
});
