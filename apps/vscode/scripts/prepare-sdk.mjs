import { spawnSync } from "node:child_process";
import { npmInvocation } from "./npm.mjs";

const result = spawnSync(...npmInvocation(["run", "build"]), {
  cwd: new URL("../../../sdk/typescript/", import.meta.url),
  stdio: "inherit",
});
process.exit(result.status ?? 1);
