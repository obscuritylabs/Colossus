#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  fsyncSync,
  lstatSync,
  openSync,
  readFileSync,
  readSync,
  realpathSync,
  writeSync,
} from "node:fs";
import { basename, isAbsolute } from "node:path";

const MAX_EXECUTABLE_BYTES = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES = 16 * 1024;
const SHA256_PATTERN = /^[0-9a-f]{64}$/u;
const TARGET_PATTERN = /^[A-Za-z0-9_.-]+$/u;
const RELEASE_CHANNELS = new Set([
  "stable",
  "developer_preview",
  "validation_only",
]);
const BINDING_PREFIX = Buffer.from(
  "COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256_V1=",
  "ascii",
);
const BINDING_SUFFIX = Buffer.from(
  ":END_COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256",
  "ascii",
);
const PLACEHOLDER_DIGEST = Buffer.from("0".repeat(64), "ascii");
const PLACEHOLDER_BINDING = Buffer.concat([
  BINDING_PREFIX,
  PLACEHOLDER_DIGEST,
  BINDING_SUFFIX,
]);
const MACH_O_MAGICS = new Set([
  0xfeedface, 0xcefaedfe, 0xfeedfacf, 0xcffaedfe, 0xcafebabe, 0xbebafeca,
  0xcafebabf, 0xbfbafeca,
]);

function fail(message) {
  process.stderr.write(`patch-desktop-manifest-binding: ${message}\n`);
  process.exit(1);
}

function parseArguments(argv) {
  if (argv.length !== 4) {
    fail("expected --executable and --manifest");
  }
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (
      !new Set(["--executable", "--manifest"]).has(key) ||
      values.has(key) ||
      !value
    ) {
      fail("arguments are missing, duplicated, or unknown");
    }
    values.set(key, value);
  }
  return values;
}

function openValidatedFile(path, { maximumBytes, executable, expectedName }) {
  if (
    !isAbsolute(path) ||
    (expectedName !== undefined && basename(path) !== expectedName)
  ) {
    fail(`${expectedName ?? "executable"} must use its exact absolute path`);
  }
  const metadata = lstatSync(path);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size <= 0 ||
    metadata.size > maximumBytes ||
    metadata.nlink !== 1 ||
    (metadata.mode & 0o022) !== 0 ||
    (executable && (metadata.mode & 0o111) === 0)
  ) {
    fail(`${expectedName ?? "executable"} is not a bounded regular file`);
  }
  if (realpathSync(path) !== path) {
    fail(`${expectedName ?? "executable"} must already be canonical`);
  }
  const flags =
    (executable ? constants.O_RDWR : constants.O_RDONLY) |
    (constants.O_CLOEXEC ?? 0) |
    (constants.O_NOFOLLOW ?? 0);
  const descriptor = openSync(path, flags);
  const opened = fstatSync(descriptor);
  if (
    !opened.isFile() ||
    opened.dev !== metadata.dev ||
    opened.ino !== metadata.ino ||
    opened.size !== metadata.size ||
    opened.nlink !== 1 ||
    (opened.mode & 0o022) !== 0
  ) {
    closeSync(descriptor);
    fail(`${expectedName ?? "executable"} changed while it was opened`);
  }
  return descriptor;
}

function exactObject(value, keys) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.keys(value).sort().join("\0") === [...keys].sort().join("\0")
  );
}

function validExecutableEntry(value, fileName) {
  return (
    exactObject(value, ["fileName", "sha256"]) &&
    value.fileName === fileName &&
    typeof value.sha256 === "string" &&
    SHA256_PATTERN.test(value.sha256)
  );
}

function validateManifest(bytes) {
  let manifest;
  try {
    const source = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    manifest = JSON.parse(source);
  } catch {
    fail("manifest must be valid UTF-8 JSON");
  }
  if (
    !exactObject(manifest, [
      "schemaVersion",
      "targetTriple",
      "profile",
      "releaseChannel",
      "sidecar",
      "cli",
    ]) ||
    manifest.schemaVersion !== 2 ||
    manifest.profile !== "release" ||
    !RELEASE_CHANNELS.has(manifest.releaseChannel) ||
    typeof manifest.targetTriple !== "string" ||
    !TARGET_PATTERN.test(manifest.targetTriple) ||
    !validExecutableEntry(manifest.sidecar, "colossus-sidecar") ||
    !validExecutableEntry(manifest.cli, "colossus")
  ) {
    fail("manifest does not have the canonical release schema");
  }
}

const values = parseArguments(process.argv.slice(2));
const executablePath = values.get("--executable");
const manifestPath = values.get("--manifest");
let executableDescriptor;
let manifestDescriptor;
try {
  executableDescriptor = openValidatedFile(executablePath, {
    maximumBytes: MAX_EXECUTABLE_BYTES,
    executable: true,
  });
  manifestDescriptor = openValidatedFile(manifestPath, {
    maximumBytes: MAX_MANIFEST_BYTES,
    executable: false,
    expectedName: "colossus-bundle-manifest.json",
  });
  const executable = readFileSync(executableDescriptor);
  const manifest = readFileSync(manifestDescriptor);
  validateManifest(manifest);
  if (executable.length < 4 || !MACH_O_MAGICS.has(executable.readUInt32BE(0))) {
    fail("executable is not a Mach-O image");
  }

  const bindingOffset = executable.indexOf(PLACEHOLDER_BINDING);
  if (
    bindingOffset < 0 ||
    executable.indexOf(PLACEHOLDER_BINDING, bindingOffset + 1) >= 0
  ) {
    fail("executable must contain exactly one unset manifest binding");
  }
  const digest = createHash("sha256").update(manifest).digest("hex");
  if (!SHA256_PATTERN.test(digest) || /^0{64}$/u.test(digest)) {
    fail("manifest digest is not canonical SHA-256");
  }
  const digestBytes = Buffer.from(digest, "ascii");
  const digestOffset = bindingOffset + BINDING_PREFIX.length;
  if (
    writeSync(
      executableDescriptor,
      digestBytes,
      0,
      digestBytes.length,
      digestOffset,
    ) !== digestBytes.length
  ) {
    fail("manifest binding could not be written completely");
  }
  fsyncSync(executableDescriptor);
  const confirmation = Buffer.alloc(digestBytes.length);
  if (
    readSync(
      executableDescriptor,
      confirmation,
      0,
      confirmation.length,
      digestOffset,
    ) !== confirmation.length ||
    !confirmation.equals(digestBytes)
  ) {
    fail("manifest binding could not be verified after writing");
  }
} finally {
  if (manifestDescriptor !== undefined) {
    closeSync(manifestDescriptor);
  }
  if (executableDescriptor !== undefined) {
    closeSync(executableDescriptor);
  }
}
