#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const stableVersionPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u;
const commitPattern = /^[0-9a-f]{40}$/u;

function fail(message) {
  throw new Error(message);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function metadataField(contents, field) {
  const value = contents.match(new RegExp(`^${field}: (.+)$`, "mu"))?.[1];
  if (!value) fail(`package metadata is missing ${field}`);
  return value.trim();
}

function archiveEntry(archive, listArguments, suffix) {
  const entries = execFileSync(listArguments[0], [...listArguments.slice(1), archive], {
    encoding: "utf8",
  })
    .split("\n")
    .filter((entry) => entry.endsWith(suffix));
  if (entries.length !== 1) {
    fail(`${archive} must contain exactly one ${suffix}; found ${entries.join(", ")}`);
  }
  return entries[0];
}

export function expectedSdkFiles(version) {
  const tag = `v${version}`;
  return {
    npm: `obscuritylabs-colossus-sdk-${version}.tgz`,
    wheel: `obscuritylabs_colossus_sdk-${version}-py3-none-any.whl`,
    sdist: `obscuritylabs_colossus_sdk-${version}.tar.gz`,
    manifest: `colossus-sdk-${tag}-manifest.json`,
    checksums: `colossus-sdk-${tag}-SHA256SUMS`,
  };
}

export function validateManifest(manifest, version, commit, files, digests) {
  const tag = `v${version}`;
  const expected = {
    schemaVersion: 1,
    sourceCommit: commit,
    releaseTag: tag,
    version,
    npm: {
      name: "@obscuritylabs/colossus-sdk",
      file: files.npm,
      sha256: digests[files.npm],
    },
    pypi: {
      name: "obscuritylabs-colossus-sdk",
      files: [
        { file: files.wheel, sha256: digests[files.wheel] },
        { file: files.sdist, sha256: digests[files.sdist] },
      ],
    },
    go: {
      module: "github.com/obscuritylabs/colossus/sdk/go",
      tag: `sdk/go/${tag}`,
    },
  };
  if (JSON.stringify(manifest) !== JSON.stringify(expected)) {
    fail("SDK release manifest does not match the exact release identity and artifacts");
  }
}

function digestCandidate(directory, files) {
  const expectedNames = Object.values(files).sort();
  const entries = readdirSync(directory).sort();
  if (JSON.stringify(entries) !== JSON.stringify(expectedNames)) {
    fail(`SDK candidate files differ: expected ${expectedNames}; found ${entries}`);
  }
  for (const file of entries) {
    const path = resolve(directory, file);
    const stat = lstatSync(path);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0 || stat.size > 100 * 1024 * 1024) {
      fail(`invalid SDK candidate file: ${file}`);
    }
  }
  return Object.fromEntries(entries.map((file) => [file, sha256(resolve(directory, file))]));
}

export function validateTrustedBytes(files, candidateDigests, trustedDigests) {
  for (const file of Object.values(files)) {
    const trusted = trustedDigests[file];
    if (!trusted) fail(`the trusted release build does not contain ${file}`);
    if (candidateDigests[file] !== trusted) {
      fail(`${file} is not the byte-exact artifact built from the release commit`);
    }
  }
}

function main() {
  const [directoryArgument, version, commit, trustedArgument] = process.argv.slice(2);
  if (
    !directoryArgument ||
    !version ||
    !commit ||
    process.argv.length < 5 ||
    process.argv.length > 6
  ) {
    fail("usage: verify-sdk-release.mjs DIRECTORY X.Y.Z SOURCE_COMMIT [TRUSTED_DIRECTORY]");
  }
  if (!stableVersionPattern.test(version)) fail("SDK release version must be stable X.Y.Z");
  if (!commitPattern.test(commit)) fail("source commit must be a lowercase 40-character SHA");

  const directory = realpathSync(resolve(directoryArgument));
  const files = expectedSdkFiles(version);
  const digests = digestCandidate(directory, files);
  const manifestPath = resolve(directory, files.manifest);
  validateManifest(
    JSON.parse(readFileSync(manifestPath, "utf8")),
    version,
    commit,
    files,
    digests,
  );

  const checksumNames = [files.npm, files.wheel, files.sdist, files.manifest].sort();
  const expectedChecksums = `${checksumNames
    .map((file) => `${digests[file]}  ${file}`)
    .join("\n")}\n`;
  if (readFileSync(resolve(directory, files.checksums), "ascii") !== expectedChecksums) {
    fail("SDK checksum set is incomplete, unordered, or does not match the candidate bytes");
  }

  if (trustedArgument) {
    const trusted = realpathSync(resolve(trustedArgument));
    if (trusted === directory) fail("the trusted release build must be an independent copy");
    validateTrustedBytes(files, digests, digestCandidate(trusted, files));
  }

  const npmPath = resolve(directory, files.npm);
  const npmPackageEntry = archiveEntry(npmPath, ["tar", "-tzf"], "/package.json");
  const npmPackage = JSON.parse(
    execFileSync("tar", ["-xOzf", npmPath, npmPackageEntry], { encoding: "utf8" }),
  );
  if (npmPackage.name !== "@obscuritylabs/colossus-sdk" || npmPackage.version !== version) {
    fail("npm artifact identity does not match the SDK release");
  }

  const wheelPath = resolve(directory, files.wheel);
  const wheelMetadata = archiveEntry(wheelPath, ["unzip", "-Z1"], ".dist-info/METADATA");
  const wheelContents = execFileSync("unzip", ["-p", wheelPath, wheelMetadata], {
    encoding: "utf8",
  });
  if (
    metadataField(wheelContents, "Name") !== "obscuritylabs-colossus-sdk" ||
    metadataField(wheelContents, "Version") !== version
  ) {
    fail("Python wheel identity does not match the SDK release");
  }

  const sdistPath = resolve(directory, files.sdist);
  const sdistRoot = files.sdist.slice(0, -".tar.gz".length);
  const sdistMetadata = archiveEntry(sdistPath, ["tar", "-tzf"], `${sdistRoot}/PKG-INFO`);
  const sdistContents = execFileSync("tar", ["-xOzf", sdistPath, sdistMetadata], {
    encoding: "utf8",
  });
  if (
    metadataField(sdistContents, "Name") !== "obscuritylabs-colossus-sdk" ||
    metadataField(sdistContents, "Version") !== version
  ) {
    fail("Python source distribution identity does not match the SDK release");
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
