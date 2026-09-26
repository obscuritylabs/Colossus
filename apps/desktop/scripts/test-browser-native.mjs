import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acceptanceTargets } from "./acceptance-targets.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
const targets = acceptanceTargets(
  repository,
  desktop,
  process.env.CARGO_TARGET_DIR,
);
const build = spawnSync(
  "cargo",
  [
    "build",
    "--locked",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--target-dir",
    targets.native,
    "--example",
    "browser-acceptance",
    "--features",
    "browser-test-bridge",
  ],
  { cwd: repository, stdio: "inherit", windowsHide: true },
);
if (build.status !== 0) process.exit(build.status ?? 1);
// Native code creates this new directory with an owner-only DACL. Windows' shared
// temp ancestors do not satisfy Colossus home namespace-authority requirements.
const root = join(homedir(), `.colossus-browser-acceptance-${randomUUID()}`);
const server = createServer((request, response) => {
  if (request.url === "/download") {
    response.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Disposition": 'attachment; filename="browser-fixture.txt"',
    });
    response.end("Synthetic browser fixture");
    return;
  }
  response.writeHead(200, {
    "Content-Type": "text/html",
    "Cache-Control": "no-store",
  });
  response.end(
    `<!doctype html><html><head><title>${request.url === "/second" ? "Second page" : "First page"}</title></head><body><h1>Native browser fixture</h1><a href="/second">Next page</a></body></html>`,
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const address = `http://127.0.0.1:${server.address().port}`;
try {
  const child = spawn(
    join(
      targets.native,
      "debug/examples",
      `browser-acceptance${process.platform === "win32" ? ".exe" : ""}`,
    ),
    [address],
    {
      cwd: repository,
      stdio: "inherit",
      windowsHide: true,
      env: { ...process.env, COLOSSUS_HOME: root },
    },
  );
  const timeout = setTimeout(() => {
    console.error("Native browser acceptance timed out");
    child.kill();
  }, 120_000);
  process.exitCode = await new Promise((done) => {
    child.once("error", (error) => {
      console.error(error);
      done(1);
    });
    child.once("exit", (code, signal) => {
      if (code !== 0)
        console.error(
          `Native browser exited with code ${code}, signal ${signal}`,
        );
      done(code === 0 ? 0 : 1);
    });
  });
  clearTimeout(timeout);
} finally {
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  const cleanup = resolve(root);
  if (
    dirname(cleanup) !== resolve(homedir()) ||
    !/^\.colossus-browser-acceptance-[0-9a-f-]{36}$/.test(basename(cleanup))
  )
    throw new Error("Refusing cleanup outside the generated test home");
  await rm(cleanup, {
    recursive: true,
    force: true,
    maxRetries: 10,
    retryDelay: 500,
  });
}
