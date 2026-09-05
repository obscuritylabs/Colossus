import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
function run(executable, args, cwd, env = process.env) {
  const result = spawnSync(executable, args, {
    cwd,
    env,
    stdio: "inherit",
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run(
  "cargo",
  ["build", "--locked", "-p", "colossus-cli", "--bin", "colossus"],
  repository,
);
run(
  "cargo",
  [
    "test",
    "--locked",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--example",
    "plugin-test-bridge",
    "--features",
    "plugin-test-bridge",
  ],
  repository,
);
run(
  "cargo",
  [
    "build",
    "--locked",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--example",
    "plugin-test-bridge",
    "--features",
    "plugin-test-bridge",
  ],
  repository,
);
const suffix = process.platform === "win32" ? ".exe" : "";
const sharedTarget =
  process.env.CARGO_TARGET_DIR === undefined
    ? undefined
    : resolve(repository, process.env.CARGO_TARGET_DIR);
run(
  process.execPath,
  [
    join(desktop, "node_modules/@playwright/test/cli.js"),
    "test",
    "tests/browser/acceptance-processes.spec.ts",
    "tests/browser/plugin-runtime.spec.ts",
  ],
  desktop,
  {
    ...process.env,
    COLOSSUS_PLUGIN_RUNTIME_ACCEPTANCE: "1",
    COLOSSUS_PLUGIN_TEST_CLI: join(
      sharedTarget ?? join(repository, "target"),
      `debug/colossus${suffix}`,
    ),
    COLOSSUS_PLUGIN_TEST_BRIDGE: join(
      sharedTarget ?? join(desktop, "src-tauri/target"),
      `debug/examples/plugin-test-bridge${suffix}`,
    ),
  },
);
