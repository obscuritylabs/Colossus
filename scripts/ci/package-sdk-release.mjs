#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repository = realpathSync(join(dirname(fileURLToPath(import.meta.url)), "../.."));
const stableVersionPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u;
const commitPattern = /^[0-9a-f]{40}$/u;

function fail(message) {
  throw new Error(message);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function oneFile(directory, expectedName) {
  const files = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name);
  if (!files.includes(expectedName)) {
    fail(`expected ${directory} to contain ${expectedName}; found ${files.join(", ")}`);
  }
  return join(directory, expectedName);
}

function packageVersions() {
  const cargo = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--locked", "--no-deps", "--format-version", "1"],
      { cwd: repository, encoding: "utf8" },
    ),
  );
  const cli = cargo.packages.find(({ name }) => name === "colossus-cli");
  if (!cli) fail("cargo metadata did not contain colossus-cli");

  const npm = JSON.parse(
    readFileSync(join(repository, "sdk/typescript/package.json"), "utf8"),
  );
  const pyproject = readFileSync(
    join(repository, "sdk/python/pyproject.toml"),
    "utf8",
  );
  const pythonVersion = pyproject.match(
    /^\[project\][\s\S]*?^version = "([^"]+)"$/mu,
  )?.[1];
  if (!pythonVersion) fail("could not read the Python SDK version");

  const goModule = readFileSync(join(repository, "sdk/go/go.mod"), "utf8").match(
    /^module ([^\s]+)$/mu,
  )?.[1];
  if (!goModule) fail("could not read the Go SDK module");
  return {
    rust: cli.version,
    npmName: npm.name,
    npm: npm.version,
    python: pythonVersion,
    goModule,
  };
}

export function validateReleaseIdentity(version, tag, commit, versions) {
  if (!stableVersionPattern.test(version)) {
    fail(`SDK releases require a stable X.Y.Z version; received ${version}`);
  }
  if (tag !== `v${version}`) fail(`release tag must be v${version}`);
  if (!commitPattern.test(commit)) fail("source commit must be a lowercase 40-character SHA");
  if (versions.rust !== version) fail(`Rust version ${versions.rust} does not match ${version}`);
  if (versions.npm !== version) fail(`npm version ${versions.npm} does not match ${version}`);
  if (versions.python !== version) {
    fail(`Python version ${versions.python} does not match ${version}`);
  }
  if (versions.npmName !== "@obscuritylabs/colossus-sdk") {
    fail(`unexpected npm package name: ${versions.npmName}`);
  }
  if (versions.goModule !== "github.com/obscuritylabs/colossus/sdk/go") {
    fail(`unexpected Go module: ${versions.goModule}`);
  }
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      fail("usage: package-sdk-release.mjs --version X.Y.Z --tag vX.Y.Z --commit SHA --output PATH");
    }
    values.set(name, value);
  }
  for (const required of ["--version", "--tag", "--commit", "--output"]) {
    if (!values.has(required)) fail(`missing ${required}`);
  }
  return Object.fromEntries([...values].map(([key, value]) => [key.slice(2), value]));
}

function main() {
  const { version, tag, commit, output } = parseArguments(process.argv.slice(2));
  const versions = packageVersions();
  validateReleaseIdentity(version, tag, commit, versions);
  const sourceDateEpoch = process.env.SOURCE_DATE_EPOCH;
  if (!/^[1-9][0-9]*$/u.test(sourceDateEpoch ?? "")) {
    fail("SOURCE_DATE_EPOCH must be a positive integer");
  }

  const destination = resolve(repository, output);
  const distRoot = resolve(repository, "dist");
  const allowedRoot = `${distRoot}/`;
  if (!destination.startsWith(allowedRoot)) {
    fail("SDK release output must be a new directory below repository dist/");
  }
  mkdirSync(distRoot, { recursive: true, mode: 0o755 });
  mkdirSync(destination, { recursive: false, mode: 0o755 });

  const npmReport = JSON.parse(
    execFileSync(
      "npm",
      ["pack", "--json", "--silent", "--pack-destination", destination],
      { cwd: join(repository, "sdk/typescript"), encoding: "utf8" },
    ),
  );
  if (!Array.isArray(npmReport) || npmReport.length !== 1) {
    fail("npm pack returned an unexpected report");
  }
  const npmFile = `obscuritylabs-colossus-sdk-${version}.tgz`;
  if (npmReport[0]?.filename !== npmFile) {
    fail(`npm pack produced ${npmReport[0]?.filename}; expected ${npmFile}`);
  }
  oneFile(destination, npmFile);

  const pythonDist = join(repository, "sdk/python/dist");
  const wheelFile = `obscuritylabs_colossus_sdk-${version}-py3-none-any.whl`;
  const sdistFile = `obscuritylabs_colossus_sdk-${version}.tar.gz`;
  copyFileSync(oneFile(pythonDist, wheelFile), join(destination, wheelFile));
  const sdistPath = join(destination, sdistFile);
  copyFileSync(oneFile(pythonDist, sdistFile), sdistPath);
  execFileSync(
    join(repository, "sdk/python/.codegen/bin/python"),
    [
      join(repository, "scripts/ci/normalize_python_sdist.py"),
      sdistPath,
      sourceDateEpoch,
    ],
    { cwd: repository, stdio: "inherit" },
  );

  const packages = [npmFile, wheelFile, sdistFile].map((file) => ({
    file,
    sha256: sha256(join(destination, file)),
  }));
  const manifestFile = `colossus-sdk-${tag}-manifest.json`;
  const manifest = {
    schemaVersion: 1,
    sourceCommit: commit,
    releaseTag: tag,
    version,
    npm: {
      name: versions.npmName,
      ...packages[0],
    },
    pypi: {
      name: "obscuritylabs-colossus-sdk",
      files: packages.slice(1),
    },
    go: {
      module: versions.goModule,
      tag: `sdk/go/${tag}`,
    },
  };
  writeFileSync(
    join(destination, manifestFile),
    `${JSON.stringify(manifest, null, 2)}\n`,
    { encoding: "utf8", mode: 0o644 },
  );

  const checksumFiles = [...packages.map(({ file }) => file), manifestFile].sort();
  const checksums = checksumFiles
    .map((file) => `${sha256(join(destination, file))}  ${file}`)
    .join("\n");
  writeFileSync(
    join(destination, `colossus-sdk-${tag}-SHA256SUMS`),
    `${checksums}\n`,
    { encoding: "ascii", mode: 0o644 },
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
