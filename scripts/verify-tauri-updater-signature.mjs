#!/usr/bin/env node

import {
  createPublicKey,
  createHash,
  timingSafeEqual,
  verify,
} from "node:crypto";
import { lstatSync, readFileSync } from "node:fs";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

const MAX_KEY_BYTES = 16 * 1024;
const MAX_SIGNATURE_BYTES = 16 * 1024;
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function fail(message) {
  throw new Error(`verify-tauri-updater-signature: ${message}`);
}

function exactFile(path) {
  const metadata = lstatSync(path);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    fail(`${basename(path)} must be a regular non-symlink file`);
  }
}

function boundedBase64(value, maximum, label) {
  if (
    value.length === 0 ||
    Buffer.byteLength(value) > maximum ||
    !/^[A-Za-z0-9+/]+={0,2}$/u.test(value)
  ) {
    fail(`${label} is not bounded canonical base64`);
  }
  const decoded = Buffer.from(value, "base64");
  if (decoded.toString("base64") !== value) {
    fail(`${label} is not canonical base64`);
  }
  return decoded;
}

function decodedWrapperLines(value, maximum, label) {
  const lines = boundedBase64(value, maximum, label)
    .toString("utf8")
    .split(/\r?\n/u);
  if (lines.at(-1) === "") {
    lines.pop();
  }
  if (lines.some((line) => line.length === 0)) {
    fail(`${label} wrapper contains an empty line`);
  }
  return lines;
}

function publicKey(encoded) {
  const lines = decodedWrapperLines(encoded, MAX_KEY_BYTES, "public key");
  if (lines.length !== 2 || !lines[0].startsWith("untrusted comment: ")) {
    fail("public key wrapper is invalid");
  }
  const payload = boundedBase64(lines[1], MAX_KEY_BYTES, "public key payload");
  if (payload.length !== 42 || payload[0] !== 0x45 || payload[1] !== 0x64) {
    fail("public key payload is invalid");
  }
  return {
    id: payload.subarray(2, 10),
    key: createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, payload.subarray(10)]),
      format: "der",
      type: "spki",
    }),
  };
}

function signature(path) {
  exactFile(path);
  const encoded = readFileSync(path, "utf8").trim();
  const lines = decodedWrapperLines(encoded, MAX_SIGNATURE_BYTES, "signature");
  if (
    lines.length !== 4 ||
    !lines[0].startsWith("untrusted comment: ") ||
    !lines[2].startsWith("trusted comment: ")
  ) {
    fail("signature wrapper is invalid");
  }
  const primary = boundedBase64(
    lines[1],
    MAX_SIGNATURE_BYTES,
    "primary signature",
  );
  const global = boundedBase64(
    lines[3],
    MAX_SIGNATURE_BYTES,
    "global signature",
  );
  if (
    primary.length !== 74 ||
    primary[0] !== 0x45 ||
    ![0x44, 0x64].includes(primary[1]) ||
    global.length !== 64
  ) {
    fail("signature payload is invalid");
  }
  return {
    id: primary.subarray(2, 10),
    prehashed: primary[1] === 0x44,
    primary: primary.subarray(10),
    trustedComment: lines[2].slice("trusted comment: ".length),
    global,
  };
}

export function verifyTauriUpdaterSignature({
  file,
  signatureFile,
  encodedPublicKey,
}) {
  exactFile(file);
  const publicMaterial = publicKey(encodedPublicKey);
  const signatureMaterial = signature(signatureFile);
  if (!timingSafeEqual(publicMaterial.id, signatureMaterial.id)) {
    fail("signature key ID does not match the configured public key");
  }
  const content = readFileSync(file);
  const signedContent = signatureMaterial.prehashed
    ? createHash("blake2b512").update(content).digest()
    : content;
  if (
    !verify(null, signedContent, publicMaterial.key, signatureMaterial.primary)
  ) {
    fail("primary updater signature is invalid");
  }
  const globalContent = Buffer.concat([
    signatureMaterial.primary,
    Buffer.from(signatureMaterial.trustedComment, "utf8"),
  ]);
  if (
    !verify(null, globalContent, publicMaterial.key, signatureMaterial.global)
  ) {
    fail("trusted-comment updater signature is invalid");
  }
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      fail("arguments must be --name value pairs");
    }
    values.set(name.slice(2), value);
  }
  for (const name of ["file", "signature", "public-key"]) {
    if (!values.has(name)) {
      fail(`missing --${name}`);
    }
  }
  return values;
}

function main() {
  const values = parseArguments(process.argv.slice(2));
  verifyTauriUpdaterSignature({
    file: values.get("file"),
    signatureFile: values.get("signature"),
    encodedPublicKey: values.get("public-key"),
  });
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
