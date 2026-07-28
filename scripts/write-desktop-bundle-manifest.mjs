#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  chmodSync,
  closeSync,
  createReadStream,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, isAbsolute, join } from "node:path";
import { finished } from "node:stream/promises";

const MAX_BINARY_BYTES = 512 * 1024 * 1024;
const TARGET_PATTERN = /^[A-Za-z0-9_.-]+$/;
const RELEASE_CHANNELS = new Set([
  "stable",
  "developer_preview",
  "validation_only",
]);
const EXPECTED_ARGUMENTS = new Set([
  "--target",
  "--release-channel",
  "--sidecar",
  "--cli",
  "--output",
]);

function fail(message) {
  process.stderr.write(`write-desktop-bundle-manifest: ${message}\n`);
  process.exit(1);
}

function parseArguments(argv) {
  if (argv.length !== 10) {
    fail(
      "expected --target, --release-channel, --sidecar, --cli, and --output",
    );
  }
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!EXPECTED_ARGUMENTS.has(key) || values.has(key) || !value) {
      fail("arguments are missing, duplicated, or unknown");
    }
    values.set(key, value);
  }
  return values;
}

function validatedBinary(path, expectedNames) {
  const actualName = basename(path);
  if (!isAbsolute(path) || !expectedNames.includes(actualName)) {
    fail(`${expectedNames.join(" or ")} must use its exact absolute bundle path`);
  }
  const metadata = lstatSync(path);
  const unsafePosixMode =
    process.platform !== "win32" &&
    ((metadata.mode & 0o111) === 0 || (metadata.mode & 0o022) !== 0);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size <= 0 ||
    metadata.size > MAX_BINARY_BYTES ||
    unsafePosixMode
  ) {
    fail(`${actualName} is not a bounded, non-writable executable`);
  }
  const canonical = realpathSync(path);
  if (canonical !== path) {
    fail(`${actualName} must already be canonical`);
  }
  return canonical;
}

async function sha256(path) {
  const digest = createHash("sha256");
  const input = createReadStream(path);
  input.on("data", (chunk) => digest.update(chunk));
  await finished(input);
  return digest.digest("hex");
}

const values = parseArguments(process.argv.slice(2));
const target = values.get("--target");
if (!TARGET_PATTERN.test(target)) {
  fail("target triple contains unsafe characters");
}
const releaseChannel = values.get("--release-channel");
if (!RELEASE_CHANNELS.has(releaseChannel)) {
  fail("release channel is not stable, developer_preview, or validation_only");
}
const windowsTarget = target.includes("-windows-");
const sidecarName = windowsTarget ? "colossus-sidecar.exe" : "colossus-sidecar";
const cliName = windowsTarget ? "colossus.exe" : "colossus";
const sidecar = validatedBinary(values.get("--sidecar"), [
  sidecarName,
  `colossus-sidecar-${target}${windowsTarget ? ".exe" : ""}`,
]);
const cli = validatedBinary(values.get("--cli"), [
  cliName,
  `colossus-${target}${windowsTarget ? ".exe" : ""}`,
]);
const output = values.get("--output");
if (
  !isAbsolute(output) ||
  basename(output) !== "colossus-bundle-manifest.json"
) {
  fail("output must be the exact absolute bundle-manifest path");
}
if (existsSync(output) && lstatSync(output).isSymbolicLink()) {
  fail("output manifest must not be a symlink");
}
const parent = dirname(output);
mkdirSync(parent, { recursive: true, mode: 0o755 });
if (realpathSync(parent) !== parent) {
  fail("output directory must already be canonical");
}

const manifest = {
  schemaVersion: 2,
  targetTriple: target,
  profile: "release",
  releaseChannel,
  sidecar: {
    fileName: sidecarName,
    sha256: await sha256(sidecar),
  },
  cli: {
    fileName: cliName,
    sha256: await sha256(cli),
  },
};
const temporary = join(parent, `.colossus-bundle-manifest.${process.pid}.tmp`);
let descriptor;
try {
  descriptor = openSync(temporary, "wx", 0o600);
  writeFileSync(descriptor, `${JSON.stringify(manifest)}\n`, "utf8");
  closeSync(descriptor);
  descriptor = undefined;
  chmodSync(temporary, 0o644);
  renameSync(temporary, output);
} finally {
  if (descriptor !== undefined) {
    closeSync(descriptor);
  }
  rmSync(temporary, { force: true });
}
