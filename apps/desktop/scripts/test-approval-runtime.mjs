import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
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
  "--example",
  "approval-test-bridge",
  "--features",
  "approval-test-bridge",
];
run("cargo", ["test", ...native], repository);
run("cargo", ["build", ...native], repository);
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
    "--retries=0",
    "tests/browser/approval-runtime.spec.ts",
  ],
  desktop,
  {
    ...process.env,
    COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE: "1",
    COLOSSUS_APPROVAL_TEST_SIDECAR: join(
      sharedTarget ?? join(repository, "target"),
      `debug/colossus-sidecar${suffix}`,
    ),
    COLOSSUS_APPROVAL_TEST_BRIDGE: join(
      sharedTarget ?? join(desktop, "src-tauri/target"),
      `debug/examples/approval-test-bridge${suffix}`,
    ),
  },
);
