import { spawnSync } from "node:child_process";

const result = spawnSync(
  process.platform === "win32" ? "npm.cmd" : "npm",
  ["run", "build"],
  {
    cwd: new URL("../../../sdk/typescript/", import.meta.url),
    stdio: "inherit",
  },
);
process.exit(result.status ?? 1);
