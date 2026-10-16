import { spawnSync } from "node:child_process";
import { nativeExecutables } from "./cargo-native.mjs";

const packages = ["colossus-app-host", "colossus-sidecar"];
const built = spawnSync(
  "cargo",
  [
    "build",
    "--manifest-path",
    "../../Cargo.toml",
    "--locked",
    "--profile",
    "vscode",
    "--message-format=json",
    ...packages.flatMap((name) => ["-p", name]),
  ],
  {
    env: { ...process.env, CARGO_INCREMENTAL: "0" },
    stdio: ["inherit", "pipe", "inherit"],
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  },
);
if (built.error) throw new Error("Native runtime build failed.");
const messages = built.stdout
  .split(/\r?\n/u)
  .filter(Boolean)
  .map((line) => JSON.parse(line));
for (const message of messages)
  if (message.reason === "compiler-message" && message.message?.rendered)
    process.stderr.write(message.message.rendered);
if (built.status !== 0) throw new Error("Native runtime build failed.");
const artifacts = nativeExecutables(messages, packages);
const staged = spawnSync(
  process.execPath,
  [
    "scripts/stage-native.mjs",
    `${process.platform}-${process.arch}`,
    "--cargo-artifacts",
    JSON.stringify(artifacts),
  ],
  { stdio: "inherit" },
);
if (staged.status !== 0) throw new Error("Native runtime staging failed.");
