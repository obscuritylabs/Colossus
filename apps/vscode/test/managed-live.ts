/** Real bundled sidecar acceptance, without a daemon, enrollment, or bearer on disk. */
import assert from "node:assert/strict";
import { mkdtemp, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { resolve, join } from "node:path";
import { createServer } from "node:http";
import { once } from "node:events";
import { ManagedRuntime, type ManagedSetup } from "../src/managed.js";
import { connectManaged } from "../src/connection.js";
import { WorkController } from "../src/controller.js";
import { removeManagedFixture } from "./managed-fixture.js";

const root = await mkdtemp(join(homedir(), ".colossus-vscode-managed-test-"));
const workspace = join(root, "workspace");
const providerRequests: {
  model: string;
  reasoning_effort?: string;
  output_config?: { effort: string };
  thinking?: { type: string };
  path: string | undefined;
  apiKey: string | string[] | undefined;
  authorization: string | undefined;
}[] = [];
const provider = createServer(async (request, response) => {
  let body = "";
  for await (const bytes of request) {
    body += bytes;
    if (body.length > 1048576) {
      response.writeHead(413);
      response.end();
      return;
    }
  }
  const input = JSON.parse(body) as {
    model: string;
    reasoning_effort?: string;
    output_config?: { effort: string };
    thinking?: { type: string };
  };
  providerRequests.push({
    ...input,
    path: request.url,
    apiKey: request.headers["x-api-key"],
    authorization: request.headers.authorization,
  });
  response.setHeader("content-type", "text/event-stream");
  if (request.url === "/v1/messages") {
    response.end(
      [
        {
          type: "message_start",
          message: {
            id: "managed-anthropic",
            type: "message",
            role: "assistant",
            usage: { input_tokens: 4, output_tokens: 0 },
          },
        },
        {
          type: "content_block_start",
          index: 0,
          content_block: { type: "text", text: "" },
        },
        {
          type: "content_block_delta",
          index: 0,
          delta: {
            type: "text_delta",
            text: `Managed Messages replied with ${input.model}`,
          },
        },
        { type: "content_block_stop", index: 0 },
        {
          type: "message_delta",
          delta: { stop_reason: "end_turn" },
          usage: { output_tokens: 4 },
        },
        { type: "message_stop" },
      ]
        .map(
          (event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`,
        )
        .join(""),
    );
    return;
  }
  response.end(
    `data: ${JSON.stringify({ id: "managed-live", choices: [{ index: 0, delta: { content: `Managed provider replied with ${input.model}` }, finish_reason: "stop" }] })}\n\ndata: [DONE]\n\n`,
  );
});
provider.listen(0, "127.0.0.1");
await once(provider, "listening");
const address = provider.address();
assert.ok(address && typeof address !== "string");
await mkdir(workspace);
const setup: ManagedSetup = {
  version: 1,
  workspaceIdentity: "native-fixture",
  access: "minimal",
  primary: "echo",
  providers: [
    {
      profile: "echo",
      kind: "echo",
      base_url: null,
      credential_id: null,
      timeout_ms: 5000,
    },
  ],
  models: [
    {
      profile: "echo",
      provider_profile: "echo",
      model: "echo",
      context_window_tokens: 32768,
      max_output_tokens: 4096,
      capabilities: {
        tool_calls: "auto",
        streaming: "auto",
        image_inputs: "auto",
        server_compaction: "auto",
        declared: {},
      },
    },
  ],
};
const runtime = new ManagedRuntime();
const other = new ManagedRuntime();
let stopped = 0;
const options = {
  bundle: resolve("native"),
  workspace,
  home: join(root, "home"),
  setup,
  credentials: [],
  stopped: () => {
    stopped++;
  },
  restarted: () => {
    throw new Error("Unexpected restart");
  },
};
let remembered = "";
const controller = new WorkController("Managed fixture", {
  publish() {},
  async remember(id) {
    remembered = id;
  },
  async interaction() {
    throw new Error("Unexpected interaction");
  },
});
try {
  console.log("Managed Local E2E: initial native startup");
  const first = await runtime.start(options);
  assert.ok(runtime.alive);
  assert.ok(runtime.workspaceIdentity);
  setup.nativeWorkspaceIdentity = runtime.workspaceIdentity;
  console.log("Managed Local E2E: reject a changed native workspace identity");
  await assert.rejects(
    other.start({
      ...options,
      setup: {
        ...setup,
        nativeWorkspaceIdentity: {
          version: runtime.workspaceIdentity!.version,
          sha256: "0".repeat(64),
        },
      },
    }),
    /workspace changed/u,
  );
  const pid = runtime.pid;
  console.log("Managed Local E2E: authenticated echo turn");
  await controller.attach(await connectManaged(first, pid), "primary");
  assert.equal(controller.view.modelProfiles.length, 1);
  assert.equal(controller.view.modelProfiles[0]?.profile, "echo");
  await controller.send("managed sidebar lifecycle acceptance", "execute");
  const deadline = Date.now() + 30000;
  while (controller.view.busy) {
    assert.ok(Date.now() < deadline, "Managed turn timeout");
    assert.equal(controller.view.error, "");
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.equal(controller.view.error, "");
  assert.ok(
    controller.view.messages.some(
      (message) =>
        message.role === "assistant" &&
        message.text.includes("managed sidebar lifecycle acceptance"),
    ),
  );
  assert.ok(remembered);
  // A second editor owner must fail without taking over the first writer lease.
  console.log("Managed Local E2E: reject a concurrent owner");
  await assert.rejects(other.start(options));
  assert.ok(runtime.alive);
  controller.detach();
  await runtime.stop();
  assert.equal(runtime.alive, false);
  assert.throws(() => process.kill(pid, 0));
  console.log("Managed Local E2E: graceful restart and durable restore");
  const next = await runtime.start(options);
  assert.equal(next.instance_id, first.instance_id);
  assert.ok(
    next.bearer !== first.bearer,
    "A restart must rotate transport authorization.",
  );
  await controller.attach(
    await connectManaged(next, runtime.pid),
    "primary",
    remembered,
  );
  assert.ok(
    controller.view.messages.some((message) =>
      message.text.includes("managed sidebar lifecycle acceptance"),
    ),
  );
  // A host crash closes the native process job/guardian; the extension detects it.
  console.log("Managed Local E2E: supervisor crash and recovery");
  process.kill(runtime.pid);
  const crashDeadline = Date.now() + 10000;
  while (runtime.alive && Date.now() < crashDeadline)
    await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(runtime.alive, false);
  controller.detach();
  const afterCrash = await runtime.start(options);
  assert.equal(afterCrash.instance_id, first.instance_id);
  await controller.attach(
    await connectManaged(afterCrash, runtime.pid),
    "primary",
    remembered,
  );
  assert.ok(
    controller.view.messages.some((message) =>
      message.text.includes("managed sidebar lifecycle acceptance"),
    ),
  );
  assert.ok(stopped >= 3);
  controller.detach();
  await runtime.stop();
  const providerSetup: ManagedSetup = {
    ...setup,
    primary: "fast",
    providers: [
      {
        profile: "live",
        kind: "open_ai_compatible",
        base_url: `http://127.0.0.1:${address.port}/v1`,
        credential_id: "live-credential",
        timeout_ms: 10000,
      },
    ],
    models: ["fast", "deep"].map((profile) => ({
      ...setup.models[0]!,
      profile,
      provider_profile: "live",
      model: `live-${profile}`,
      reasoning_effort: profile === "fast" ? "low" : "high",
    })),
  };
  console.log(
    "Managed Local E2E: provider credentials and model/reasoning selection",
  );
  const providerConnection = await runtime.start({
    ...options,
    setup: providerSetup,
    credentials: [
      { id: "live-credential", secret: "managed-test-private-provider-key" },
    ],
  });
  await controller.attach(
    await connectManaged(providerConnection, runtime.pid),
    "primary",
  );
  assert.equal(controller.view.modelProfiles.length, 2);
  for (const [modelProfile, reasoningEffort] of [
    ["deep", "xhigh"],
    ["fast", "low"],
  ] as const) {
    controller.setModelSelection({ modelProfile, reasoningEffort });
    await controller.send(
      "Verify managed provider selection",
      "execute",
      undefined,
      controller.view.modelSelection,
    );
    const deadline = Date.now() + 20000;
    while (controller.view.busy && Date.now() < deadline)
      await new Promise((resolve) => setTimeout(resolve, 25));
    assert.equal(controller.view.error, "");
    assert.equal(controller.view.busy, false);
  }
  assert.deepEqual(
    providerRequests.map(({ model, reasoning_effort }) => ({
      model,
      reasoning_effort,
    })),
    [
      { model: "live-deep", reasoning_effort: "xhigh" },
      { model: "live-fast", reasoning_effort: "low" },
    ],
  );
  assert.ok(
    providerRequests.every(
      (request) =>
        request.authorization === "Bearer managed-test-private-provider-key",
    ),
  );
  controller.detach();
  await runtime.stop();
  console.log(
    "Managed Local E2E: native Anthropic provider and reasoning selection",
  );
  const anthropicConnection = await runtime.start({
    ...options,
    setup: {
      ...providerSetup,
      primary: "anthropic",
      providers: [
        { ...providerSetup.providers[0]!, kind: "anthropic_messages" },
      ],
      models: [
        {
          ...providerSetup.models[0]!,
          profile: "anthropic",
          model: "live-anthropic",
          reasoning_effort: "high",
        },
      ],
    },
    credentials: [
      { id: "live-credential", secret: "managed-test-anthropic-key" },
    ],
  });
  await controller.attach(
    await connectManaged(anthropicConnection, runtime.pid),
    "primary",
  );
  controller.setModelSelection({
    modelProfile: "anthropic",
    reasoningEffort: "high",
  });
  await controller.send(
    "Verify native Messages selection",
    "execute",
    undefined,
    controller.view.modelSelection,
  );
  const messagesDeadline = Date.now() + 20000;
  while (controller.view.busy && Date.now() < messagesDeadline)
    await new Promise((resolve) => setTimeout(resolve, 25));
  assert.equal(controller.view.error, "");
  assert.equal(controller.view.busy, false);
  const nativeRequest = providerRequests.at(-1)!;
  assert.equal(nativeRequest.path, "/v1/messages");
  assert.equal(nativeRequest.model, "live-anthropic");
  assert.equal(nativeRequest.apiKey, "managed-test-anthropic-key");
  assert.equal(nativeRequest.authorization, undefined);
  assert.deepEqual(nativeRequest.thinking, { type: "adaptive" });
  assert.deepEqual(nativeRequest.output_config, { effort: "high" });
  assert.ok(
    controller.view.messages.some(
      (message) =>
        message.role === "assistant" &&
        message.text.includes("Managed Messages replied with live-anthropic"),
    ),
  );
  const projection = JSON.stringify(controller.view);
  assert.ok(!projection.includes("managed-test-private-provider-key"));
  assert.ok(!projection.includes("managed-test-anthropic-key"));
  assert.ok(!projection.includes(providerConnection.bearer));
  assert.ok(!projection.includes(providerConnection.approval_broker_bearer));
  assert.ok(!projection.includes(anthropicConnection.bearer));
  assert.ok(!projection.includes(anthropicConnection.approval_broker_bearer));
} finally {
  controller.detach();
  await Promise.all([runtime.stop(), other.stop()]);
  provider.close();
  // Only this test's newly-created owner-private home is removed.
  await removeManagedFixture(root);
}
console.log(
  "Managed Local E2E passed: native start, authenticated turns, provider credential handoff, model/reasoning selection, writer conflict, graceful stop, durable restore, host crash, restart, secret-free projections, and fixture cleanup.",
);
