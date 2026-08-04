#!/usr/bin/env node

import { createHash } from "node:crypto";
import { appendFileSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function registryJson(url) {
  const response = await fetch(url, { redirect: "error" });
  if (response.status === 404) return undefined;
  if (!response.ok) fail(`registry request failed with HTTP ${response.status}: ${url}`);
  return response.json();
}

export function assessPypiRelease(metadata, expected) {
  if (metadata === undefined) return { publish: true, reason: "version is absent" };
  if (!Array.isArray(metadata.urls)) fail("PyPI response is missing release files");
  const existing = new Map(
    metadata.urls.map(({ filename, digests }) => [filename, digests?.sha256]),
  );
  for (const [filename, digest] of Object.entries(expected)) {
    const published = existing.get(filename);
    if (published !== undefined && published !== digest) {
      fail(`PyPI already contains different bytes for ${filename}`);
    }
  }
  for (const filename of existing.keys()) {
    if (!(filename in expected)) fail(`PyPI contains an unexpected file for this version: ${filename}`);
  }
  const missing = Object.keys(expected).filter((filename) => !existing.has(filename));
  return missing.length === 0
    ? { publish: false, reason: "exact files are already published" }
    : { publish: true, reason: `missing ${missing.join(", ")}` };
}

async function assessNpmRelease(metadata, expectedDigest) {
  if (metadata === undefined) return { publish: true, reason: "version is absent" };
  const tarball = metadata.dist?.tarball;
  if (typeof tarball !== "string" || !tarball.startsWith("https://registry.npmjs.org/")) {
    fail("npm registry returned an untrusted tarball URL");
  }
  const response = await fetch(tarball, { redirect: "error" });
  if (!response.ok) fail(`npm tarball request failed with HTTP ${response.status}`);
  const digest = sha256(Buffer.from(await response.arrayBuffer()));
  if (digest !== expectedDigest) fail("npm already contains different bytes for this version");
  return { publish: false, reason: "exact tarball is already published" };
}

function writeOutput(name, value) {
  const output = process.env.GITHUB_OUTPUT;
  if (!output) fail("GITHUB_OUTPUT is required");
  appendFileSync(output, `${name}=${value}\n`, "utf8");
}

async function main() {
  const [registry, directoryArgument] = process.argv.slice(2);
  if (!registry || !directoryArgument || process.argv.length !== 4) {
    fail("usage: check-sdk-registry-state.mjs npm|pypi DIRECTORY");
  }
  const directory = resolve(directoryArgument);
  const manifestFile = process.env.SDK_MANIFEST;
  if (!manifestFile) fail("SDK_MANIFEST is required");
  const manifest = JSON.parse(readFileSync(resolve(directory, manifestFile), "utf8"));
  let assessment;
  if (registry === "npm") {
    const metadata = await registryJson(
      `https://registry.npmjs.org/@obscuritylabs%2Fcolossus-sdk/${manifest.version}`,
    );
    assessment = await assessNpmRelease(metadata, manifest.npm.sha256);
  } else if (registry === "pypi") {
    const metadata = await registryJson(
      `https://pypi.org/pypi/obscuritylabs-colossus-sdk/${manifest.version}/json`,
    );
    assessment = assessPypiRelease(
      metadata,
      Object.fromEntries(manifest.pypi.files.map(({ file, sha256: digest }) => [file, digest])),
    );
  } else {
    fail(`unsupported registry: ${registry}`);
  }
  console.log(`${registry}: ${assessment.reason}`);
  writeOutput("publish", assessment.publish ? "true" : "false");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
