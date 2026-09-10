import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import { acceptanceTargets } from "./acceptance-targets.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
const targets = acceptanceTargets(
  repository,
  desktop,
  process.env.CARGO_TARGET_DIR,
);
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
    "--target-dir",
    targets.native,
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
    "--target-dir",
    targets.native,
    "--example",
    "plugin-test-bridge",
    "--features",
    "plugin-test-bridge",
  ],
  repository,
);
const suffix = process.platform === "win32" ? ".exe" : "";
run(
  process.execPath,
  [
    join(desktop, "node_modules/@playwright/test/cli.js"),
    "test",
    "--retries=0",
    "--output=test-results/plugin-runtime",
    "tests/browser/acceptance-operations.spec.ts",
    "tests/browser/acceptance-processes.spec.ts",
    "tests/browser/plugin-runtime.spec.ts",
  ],
  desktop,
  {
    ...process.env,
    COLOSSUS_PLUGIN_RUNTIME_ACCEPTANCE: "1",
    COLOSSUS_PLUGIN_TEST_CLI: join(targets.runtime, `debug/colossus${suffix}`),
    COLOSSUS_PLUGIN_TEST_BRIDGE: join(
      targets.native,
      `debug/examples/plugin-test-bridge${suffix}`,
    ),
  },
);
