#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  createReadStream,
  lstatSync,
  readFileSync,
  realpathSync,
} from "node:fs";
import { basename, isAbsolute, join } from "node:path";
import { finished } from "node:stream/promises";

const MAX_MANIFEST_BYTES = 16 * 1024;
const MAX_BINARY_BYTES = 512 * 1024 * 1024;
const TARGET_PATTERN = /^[A-Za-z0-9_.-]+$/;
const HASH_PATTERN = /^[0-9a-f]{64}$/;

function fail(message) {
  process.stderr.write(`verify-desktop-bundle: ${message}\n`);
  process.exit(1);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) ===
      JSON.stringify([...expected].sort())
  );
}

function parseArguments(argv) {
  if (argv.length !== 4 || argv[0] !== "--app" || argv[2] !== "--target") {
    fail("usage: scripts/verify-desktop-bundle.mjs --app PATH --target TRIPLE");
  }
  return { app: argv[1], target: argv[3] };
}

function validateFile(path, executable) {
  const metadata = lstatSync(path);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size <= 0 ||
    metadata.size > MAX_BINARY_BYTES ||
    (metadata.mode & 0o022) !== 0 ||
    (executable && (metadata.mode & 0o111) === 0) ||
    realpathSync(path) !== path
  ) {
    fail(`unsafe bundle file: ${path}`);
  }
  return metadata;
}

async function sha256(path) {
  const digest = createHash("sha256");
  const input = createReadStream(path);
  input.on("data", (chunk) => digest.update(chunk));
  await finished(input);
  return digest.digest("hex");
}

const { app, target } = parseArguments(process.argv.slice(2));
if (
  !isAbsolute(app) ||
  !basename(app).endsWith(".app") ||
  !TARGET_PATTERN.test(target)
) {
  fail("application path or target triple is invalid");
}
const appMetadata = lstatSync(app);
if (
  !appMetadata.isDirectory() ||
  appMetadata.isSymbolicLink() ||
  realpathSync(app) !== app
) {
  fail("application bundle must be a canonical non-symlink directory");
}
const manifestPath = join(
  app,
  "Contents",
  "Resources",
  "colossus-bundle-manifest.json",
);
const manifestMetadata = validateFile(manifestPath, false);
if (manifestMetadata.size > MAX_MANIFEST_BYTES) {
  fail("bundle manifest exceeds its size limit");
}
let manifest;
try {
  manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
} catch {
  fail("bundle manifest is not strict JSON");
}
if (
  !exactKeys(manifest, [
    "schemaVersion",
    "targetTriple",
    "profile",
    "sidecar",
    "cli",
  ]) ||
  manifest.schemaVersion !== 1 ||
  manifest.targetTriple !== target ||
  manifest.profile !== "release"
) {
  fail("bundle manifest envelope is invalid");
}

for (const [key, expectedName] of [
  ["sidecar", "colossus-sidecar"],
  ["cli", "colossus"],
]) {
  const entry = manifest[key];
  if (
    !exactKeys(entry, ["fileName", "sha256"]) ||
    entry.fileName !== expectedName ||
    !HASH_PATTERN.test(entry.sha256)
  ) {
    fail(`bundle manifest ${key} entry is invalid`);
  }
  const binary = join(app, "Contents", "MacOS", expectedName);
  validateFile(binary, true);
  if ((await sha256(binary)) !== entry.sha256) {
    fail(`bundle manifest ${key} digest does not match`);
  }
}

process.stdout.write("Verified sealed desktop bundle manifest.\n");
