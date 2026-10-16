import { spawnSync } from "node:child_process";

const packages = ["colossus-app-host", "colossus-sidecar"];
const common = [
  "--manifest-path",
  "../../Cargo.toml",
  "--locked",
  "--profile",
  "test",
];
const run = (program, args) => {
  const result = spawnSync(program, args, { stdio: "inherit" });
  if (result.status !== 0) throw new Error("Native runtime build failed.");
};
if (process.platform === "win32") {
  // MSVC still creates large temporary PDBs with strip=debuginfo. Disable the
  // final executables' PDB output without changing dependency compiler flags.
  for (const name of packages)
    run("cargo", [
      "rustc",
      ...common,
      "-p",
      name,
      "--bin",
      name,
      "--",
      "-C",
      "link-arg=/DEBUG:NONE",
    ]);
} else {
  run("cargo", [
    "build",
    ...common,
    ...packages.flatMap((name) => ["-p", name]),
  ]);
}
run(process.execPath, ["scripts/stage-native.mjs"]);
