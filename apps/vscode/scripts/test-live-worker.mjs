import { spawn } from "node:child_process";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== "--electron"))
  throw new Error(
    "Usage: test-live-worker.mjs [--electron /absolute/path/to/electron]",
  );
const electron = args.length ? resolve(args[1]) : undefined;
if (electron) {
  if (process.platform === "win32")
    throw new Error(
      "The Electron worker acceptance launcher currently requires Unix.",
    );
  await access(electron);
}
const repository = fileURLToPath(new URL("../../../", import.meta.url));
const binary = new URL(
  `../../../target/debug/examples/sdk_ephemeral_local${process.platform === "win32" ? ".exe" : ""}`,
  import.meta.url,
);
await access(binary);
const runner = new URL(
  "../../../sdk/typescript/.live-dist/examples/live-run.js",
  import.meta.url,
);
const previous = await readFile(runner).catch((error) => {
  if (error.code === "ENOENT") return undefined;
  throw error;
});
const home = await mkdtemp(`${tmpdir()}/colossus-vscode-live-`);
const config = `${home}/config.json`;
const requests = [];
async function provider(name) {
  const server = createServer(async (request, response) => {
    let bytes = "";
    for await (const chunk of request) {
      bytes += chunk;
      if (bytes.length > 1_048_576) {
        response.writeHead(413).end();
        return;
      }
    }
    const body = JSON.parse(bytes);
    const users = body.messages.filter((message) => message.role === "user");
    requests.push({
      provider: name,
      model: body.model,
      effort: body.reasoning_effort,
      history: users.length,
    });
    const text = String(users.at(-1).content);
    response.setHeader("Content-Type", "text/event-stream");
    response.end(
      `data: ${JSON.stringify({ id: `vscode-${requests.length}`, choices: [{ index: 0, delta: { content: text }, finish_reason: "stop" }] })}\n\ndata: [DONE]\n\n`,
    );
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return { server, origin: `http://127.0.0.1:${server.address().port}` };
}
const fast = await provider("fast");
const deep = await provider("deep");
try {
  const environment = { ...process.env, COLOSSUS_HOME: home };
  if (electron) {
    const bin = `${home}/bin`;
    await mkdir(bin, { mode: 0o700 });
    // The Rust fixture launches node and filters its environment. Set Electron's
    // Node mode in a private launcher; the bearer still arrives only on its pipe.
    const quotedElectron = `'${electron.replaceAll("'", "'\\''")}'`;
    await writeFile(
      `${bin}/node`,
      `#!/bin/sh\nELECTRON_RUN_AS_NODE=1 exec ${quotedElectron} "$@"\n`,
      { mode: 0o700 },
    );
    environment.PATH = `${bin}${delimiter}${process.env.PATH ?? ""}`;
  }
  await writeFile(
    config,
    JSON.stringify({
      schemaVersion: 3,
      storage: {
        adapter: "ephemeral",
        path: `${home}/state`,
        keys: { kind: "none" },
      },
      providers: {
        profiles: {
          fast: {
            kind: "open_ai_compatible",
            baseUrl: `${fast.origin}/v1`,
            timeoutMs: 5000,
          },
          deep: {
            kind: "open_ai_compatible",
            baseUrl: `${deep.origin}/v1`,
            timeoutMs: 5000,
          },
        },
      },
      models: {
        profiles: {
          fast: {
            providerProfile: "fast",
            model: "live-fast",
            reasoningEffort: "low",
            contextWindowTokens: 32000,
            maxOutputTokens: 4000,
            capabilities: { toolCalls: true, streaming: true },
          },
          deep: {
            providerProfile: "deep",
            model: "live-deep",
            reasoningEffort: "high",
            contextWindowTokens: 32000,
            maxOutputTokens: 4000,
            capabilities: { toolCalls: true, streaming: true },
          },
        },
        roles: { primary: "fast" },
      },
      access: {
        profile: "pinned",
        tools: { include: [] },
        actions: {
          allow: [
            "provider.openai.chat",
            "presentation.history.append",
            "context.show",
            "plugin.list",
          ],
        },
      },
      sandbox: { networkDestinations: [fast.origin, deep.origin] },
    }),
    { mode: 0o600 },
  );
  await mkdir(new URL(".", runner), { recursive: true });
  await writeFile(
    runner,
    `import ${JSON.stringify(new URL("../.test-dist/test/live-worker.js", import.meta.url).href)};\n`,
  );
  const code = await new Promise((resolve, reject) => {
    const child = spawn(
      fileURLToPath(binary),
      [config, "typescript", "VS_CODE_LIVE_ECHO"],
      {
        cwd: repository,
        env: environment,
        stdio: "inherit",
      },
    );
    child.on("error", reject);
    child.on("exit", resolve);
  });
  if (code !== 0) throw new Error("Live runtime acceptance failed.");
  assert.deepEqual(requests, [
    { provider: "deep", model: "live-deep", effort: "low", history: 1 },
    { provider: "fast", model: "live-fast", effort: "high", history: 2 },
    { provider: "fast", model: "live-fast", effort: "low", history: 3 },
    { provider: "deep", model: "live-deep", effort: "high", history: 4 },
    { provider: "fast", model: "live-fast", effort: "low", history: 5 },
  ]);
  console.log(
    "VS_CODE_MODEL_SELECTION_OK: two provider endpoints, five turns, explicit effort, saved defaults and retained history",
  );
} finally {
  fast.server.close();
  deep.server.close();
  if (previous) await writeFile(runner, previous);
  else await rm(runner, { force: true });
  await rm(home, { recursive: true, force: true });
}
