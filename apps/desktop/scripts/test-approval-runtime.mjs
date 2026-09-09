import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acceptanceTargets } from "./acceptance-targets.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
const targets = acceptanceTargets(
  repository,
  desktop,
  process.env.CARGO_TARGET_DIR,
);
function run(binary, args, cwd, env = process.env) {
  const result = spawnSync(binary, args, {
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
  ["build", "--locked", "-p", "colossus-sidecar", "--bin", "colossus-sidecar"],
  repository,
);
const native = [
  "--locked",
  "--manifest-path",
  "apps/desktop/src-tauri/Cargo.toml",
  "--target-dir",
  targets.native,
  "--example",
  "approval-test-bridge",
  "--features",
  "approval-test-bridge",
];
run("cargo", ["test", ...native], repository);
run("cargo", ["build", ...native], repository);
const suffix = process.platform === "win32" ? ".exe" : "";
run(
  process.execPath,
  [
    join(desktop, "node_modules/@playwright/test/cli.js"),
    "test",
    "--retries=0",
    "tests/browser/approval-runtime.spec.ts",
  ],
  desktop,
  {
    ...process.env,
    COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE: "1",
    COLOSSUS_APPROVAL_TEST_SIDECAR: join(
      targets.runtime,
      `debug/colossus-sidecar${suffix}`,
    ),
    COLOSSUS_APPROVAL_TEST_BRIDGE: join(
      targets.native,
      `debug/examples/approval-test-bridge${suffix}`,
    ),
  },
);
