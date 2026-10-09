// Trusted launcher environment seam. This module never reads wrapping key bytes.
import { spawn, spawnSync } from "node:child_process";
import { isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

export const selectorName = "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY";
const legacyKeys = new Set([
  "COLOSSUS_DEV_JOURNAL_KEY",
  "COLOSSUS_DEV_SIGNING_KEY",
  "COLOSSUS_JOURNAL_KEY",
  "COLOSSUS_SIGNING_KEY",
]);
export function developmentEnvironment(input, selector) {
  const output = {};
  for (const [name, value] of Object.entries(input)) {
    const normalized = name.toUpperCase();
    if (
      normalized.startsWith("COLOSSUS_DEVELOPMENT_") ||
      legacyKeys.has(normalized)
    )
      continue;
    if (value !== undefined) output[name] = value;
  }
  if (selector !== undefined) {
    if (
      !isAbsolute(selector) ||
      selector.length > 4096 ||
      /[\u0000-\u001f\u007f]/u.test(selector)
    )
      throw new Error("development authority must be an absolute bounded path");
    output[selectorName] = selector;
  }
  return output;
}
function main(argv) {
  const options = {};
  while (argv.length && argv[0] !== "--") {
    const flag = argv.shift();
    if (
      !["--authority-home", "--workspace", "--cli"].includes(flag) ||
      !argv.length ||
      options[flag] !== undefined
    )
      throw new Error(
        "usage: development-launch.mjs [--authority-home HOME --workspace ROOT --cli CLI] -- COMMAND [ARG...]",
      );
    options[flag] = argv.shift();
  }
  if (argv.shift() !== "--" || !argv.length)
    throw new Error("a development command is required");
  let selector;
  if (Object.keys(options).length) {
    for (const flag of ["--authority-home", "--workspace", "--cli"])
      if (!isAbsolute(options[flag] ?? ""))
        throw new Error("development custody paths must be absolute");
    const status = spawnSync(
      options["--cli"],
      [
        "--output",
        "json",
        "dev-credentials",
        "status",
        "--home",
        options["--authority-home"],
        "--workspace",
        options["--workspace"],
      ],
      {
        env: developmentEnvironment(process.env),
        encoding: "utf8",
        maxBuffer: 32768,
        timeout: 10000,
      },
    );
    if (status.error || status.status !== 0)
      throw new Error(
        "development authority is not prepared and active; use the reviewed dev-credentials plan/rewrap flow before launching",
      );
    const metadata = JSON.parse(status.stdout);
    if (metadata.active !== true || typeof metadata.authorityPath !== "string")
      throw new Error(
        "development authority is inactive; explicit reviewed activation is required",
      );
    selector = metadata.authorityPath;
  }
  const command = argv.shift();
  const child = spawn(command, argv, {
    env: developmentEnvironment(process.env, selector),
    stdio: "inherit",
  });
  for (const signal of ["SIGINT", "SIGTERM"])
    process.on(signal, () => child.kill(signal));
  child.on("error", () => {
    console.error("development command could not start");
    process.exitCode = 1;
  });
  child.on("exit", (code, signal) => {
    process.exitCode = code ?? (signal === "SIGINT" ? 130 : 1);
  });
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`development launch: ${error.message}`);
    process.exitCode = 1;
  }
}
