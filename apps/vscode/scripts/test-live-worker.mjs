import { spawn } from "node:child_process";
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
  "../../../target/debug/examples/sdk_ephemeral_local",
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
} finally {
  if (previous) await writeFile(runner, previous);
  else await rm(runner, { force: true });
  await rm(home, { recursive: true, force: true });
}
