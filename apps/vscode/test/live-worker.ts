/** Development-only acceptance: the existing Rust fixture supplies the bearer over an anonymous pipe. */
import assert from "node:assert/strict";
import { fstatSync, readFileSync } from "node:fs";
import { dirname } from "node:path";
import { connectWorker, workspaceIdentity } from "../src/connection.js";
import { WorkController } from "../src/controller.js";

const [descriptor, _certificate, instanceId, certificateSha256, prompt] =
  process.argv.slice(2);
if (!descriptor || !instanceId || !certificateSha256 || !prompt)
  throw new Error("Missing live-fixture enrollment metadata.");
if (!fstatSync(0).isFIFO() && !fstatSync(0).isSocket())
  throw new Error(
    "The fixture credential must arrive through an anonymous pipe.",
  );
const bytes = readFileSync(0);
if (bytes.length < 16 || bytes.length > 761)
  throw new Error("Invalid fixture credential length.");
const workspace = await workspaceIdentity(process.cwd());
const client = await connectWorker(
  {
    schemaVersion: 1,
    workspacePath: workspace.path,
    workspaceIdentity: workspace.identity,
    discoveryDirectory: dirname(descriptor),
    instanceId,
    certificateSha256,
    keyringService: "fixture",
    keyringAccount: "fixture",
    role: "primary",
  },
  async () => bytes.toString("ascii"),
);
bytes.fill(0);
const controller = new WorkController("Live workspace", {
  publish() {},
  async remember() {},
  async interaction() {
    throw new Error("This noninteractive fixture does not grant tools.");
  },
});
try {
  await controller.attach(client, "primary");
  await controller.send(prompt, "execute");
  const deadline = Date.now() + 20_000;
  while (controller.view.busy) {
    if (Date.now() > deadline || controller.view.error)
      throw new Error(controller.view.error || "Live runtime did not finish.");
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.ok(
    controller.view.messages.some(
      (message) =>
        message.role === "assistant" && message.text.includes(prompt),
    ),
  );
  assert.ok(controller.view.sessionId);
  await controller.selectSession(controller.view.sessionId, true);
  assert.ok(
    controller.view.messages.some(
      (message) =>
        message.role === "assistant" && message.text.includes(prompt),
    ),
  );
  const existingSession = controller.view.sessionId;
  await controller.send("VS_CODE_FOLLOW_UP", "execute");
  const followUpDeadline = Date.now() + 20_000;
  while (controller.view.busy) {
    if (Date.now() > followUpDeadline || controller.view.error)
      throw new Error(controller.view.error || "Follow-up run did not finish.");
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.equal(controller.view.sessionId, existingSession);
  assert.ok(
    controller.view.messages.some(
      (message) =>
        message.role === "assistant" &&
        message.text.includes("VS_CODE_FOLLOW_UP"),
    ),
  );
  const observedRun = controller.view.runs.find(
    (run) => run.sessionId === existingSession,
  );
  assert.ok(observedRun);
  await controller.inspect(observedRun.id, "run");
  assert.equal(controller.view.inspection?.run.id, observedRun.id);
  assert.match(controller.view.inspection!.output, /VS_CODE_FOLLOW_UP/);
  assert.equal(controller.view.inspection!.run.status, "completed");
  assert.ok(!JSON.stringify(controller.view.inspection).includes("etag"));
  process.stdout.write(
    "VS_CODE_RUNTIME_OK: run, stream, released history, recovery, follow-up in the same session, and run inspection\n",
  );
} finally {
  controller.detach();
}
