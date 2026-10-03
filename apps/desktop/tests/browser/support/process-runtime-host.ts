import { expect } from "@playwright/test";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readdir,
  realpath,
  rm,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { AcceptanceProcesses } from "./acceptance-processes";

async function removeFixture(path: string): Promise<void> {
  await chmod(path, 0o700);
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink())
      await removeFixture(join(path, entry.name));
  }
  await rm(path, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

/** A fresh home/workspace and real SDK/worker; never connects to a user's runtime. */
export async function processRuntimeHost(
  mode: "process-acceptance" | "workflow-acceptance" = "process-acceptance",
) {
  const root = await realpath(
    await mkdtemp(
      join(process.platform === "win32" ? homedir() : tmpdir(), "cp-"),
    ),
  );
  await chmod(root, 0o700);
  const workspace = join(root, "work");
  await mkdir(workspace, { mode: 0o700 });
  await mkdir(join(root, "instance"), { mode: 0o700 });
  const sidecar = join(
    root,
    process.platform === "win32" ? "colossus-sidecar.exe" : "colossus-sidecar",
  );
  await cp(process.env.COLOSSUS_APPROVAL_TEST_SIDECAR!, sidecar);
  await chmod(sidecar, 0o500);
  let pending: Record<string, unknown> | null = null;
  const observations: unknown[] = [];
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += chunk.toString();
    const parsed = JSON.parse(body);
    observations.push(parsed);
    const call = pending;
    pending = null;
    const delta = call
      ? {
          tool_calls: [
            {
              index: 0,
              id: "process-case",
              type: "function",
              function: { name: "shell_run", arguments: JSON.stringify(call) },
            },
          ],
        }
      : { content: "Acceptance turn completed." };
    response.writeHead(200, { "content-type": "text/event-stream" });
    response.end(
      `data: ${JSON.stringify({ id: "process-turn", choices: [{ index: 0, delta, finish_reason: call ? "tool_calls" : "stop" }] })}\n\ndata: [DONE]\n\n`,
    );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("missing fixture listener");
  const processes = new AcceptanceProcesses();
  const bridge = processes.start(
    process.env.COLOSSUS_APPROVAL_TEST_BRIDGE!,
    [sidecar, root, `http://127.0.0.1:${address.port}/v1`, mode],
    {
      cwd: workspace,
      env: { ...process.env, HOME: root, COLOSSUS_HOME: join(root, "home") },
      stdio: "pipe",
    },
  );
  let diagnostic = "";
  bridge.stderr!.on("data", (chunk: Buffer) => {
    diagnostic = (diagnostic + chunk.toString()).slice(-65536);
  });
  const lines: { ready?: boolean; result?: unknown; error?: string }[] = [];
  const reader = createInterface({ input: bridge.stdout! });
  reader.on("line", (line) => lines.push(JSON.parse(line)));
  const count = () => {
    if (bridge.exitCode !== null || bridge.signalCode !== null)
      throw new Error(`Process bridge exited: ${diagnostic}`);
    return lines.length;
  };
  let chain: Promise<unknown> = Promise.resolve();
  const invoke = (command: string, args: unknown = {}) => {
    const next = chain.then(async () => {
      bridge.stdin!.write(`${JSON.stringify({ command, args })}\n`);
      await expect
        .poll(count, { timeout: 55000, message: command })
        .toBeGreaterThan(0);
      const result = lines.shift()!;
      if (result.error) throw new Error(`${command}: ${result.error}`);
      return result.result;
    });
    chain = next.catch(() => undefined);
    return next;
  };
  await expect.poll(count, { timeout: 45000 }).toBeGreaterThan(0);
  expect(lines.shift()).toEqual({ ready: true });
  return {
    invoke,
    observations,
    async run(arguments_: Record<string, unknown> | null, sessionId?: string) {
      pending = arguments_
        ? {
            cwd: ".",
            justification: "Verify the isolated process acceptance fixture.",
            ...arguments_,
          }
        : null;
      return invoke("run", { sessionId }) as Promise<{
        run: { session_id: string };
        approvals: number;
        activity: { activity: { state: string; preview: string | null }[] };
      }>;
    },
    async close() {
      bridge.stdin?.end(`${JSON.stringify({ command: "close" })}\n`);
      await new Promise<void>((resolve) => {
        if (bridge.exitCode !== null) return resolve();
        const timer = setTimeout(resolve, 15000);
        bridge.once("close", () => {
          clearTimeout(timer);
          resolve();
        });
      });
      reader.close();
      await processes.close();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      // This exact mkdtemp root is owned by this test; all children have closed.
      await removeFixture(root);
    },
  };
}
