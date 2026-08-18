#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { assessPypiRelease } from "./check-sdk-registry-state.mjs";
import { validateReleaseIdentity } from "./package-sdk-release.mjs";
import {
  compareSdkReleaseDirectories,
  expectedSdkFiles,
  validateManifest,
  validateTrustedBytes,
} from "./verify-sdk-release.mjs";

const versions = {
  rust: "1.2.3",
  npm: "1.2.3",
  python: "1.2.3",
  npmName: "@obscuritylabs/colossus-sdk",
  goModule: "github.com/obscuritylabs/colossus/sdk/go",
};

test("release identity requires one stable version across every SDK", () => {
  assert.doesNotThrow(() =>
    validateReleaseIdentity("1.2.3", "v1.2.3", "a".repeat(40), versions),
  );
  assert.throws(() =>
    validateReleaseIdentity("1.2.3-preview.1", "v1.2.3-preview.1", "a".repeat(40), versions),
  );
  assert.throws(() =>
    validateReleaseIdentity("1.2.3", "v1.2.3", "a".repeat(40), {
      ...versions,
      python: "1.2.2",
    }),
  );
});

test("candidate manifest binds npm, PyPI, and Go identities to one commit", () => {
  const version = "1.2.3";
  const commit = "b".repeat(40);
  const files = expectedSdkFiles(version);
  const digests = {
    [files.npm]: "1".repeat(64),
    [files.wheel]: "2".repeat(64),
    [files.sdist]: "3".repeat(64),
  };
  const manifest = {
    schemaVersion: 1,
    sourceCommit: commit,
    releaseTag: `v${version}`,
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
      tag: `sdk/go/v${version}`,
    },
  };
  assert.doesNotThrow(() => validateManifest(manifest, version, commit, files, digests));
  assert.throws(() =>
    validateManifest({ ...manifest, sourceCommit: "c".repeat(40) }, version, commit, files, digests),
  );
});

test("publication requires every candidate byte to match an exact tag rebuild", () => {
  const root = mkdtempSync(join(tmpdir(), "colossus-sdk-release-test-"));
  const candidate = join(root, "candidate");
  const rebuilt = join(root, "rebuilt");
  mkdirSync(candidate);
  mkdirSync(rebuilt);
  try {
    for (const file of Object.values(expectedSdkFiles("1.2.3"))) {
      writeFileSync(join(candidate, file), `trusted ${file}\n`);
      writeFileSync(join(rebuilt, file), `trusted ${file}\n`);
    }
    assert.doesNotThrow(() =>
      compareSdkReleaseDirectories(candidate, rebuilt, "1.2.3"),
    );
    const npm = expectedSdkFiles("1.2.3").npm;
    writeFileSync(join(candidate, npm), "replacement bytes\n");
    assert.throws(
      () => compareSdkReleaseDirectories(candidate, rebuilt, "1.2.3"),
      /is not the byte-exact artifact built from the release commit/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Python source distributions normalize to reproducible bytes", () => {
  const root = mkdtempSync(join(tmpdir(), "colossus-python-sdist-test-"));
  const source = join(root, "source");
  const packageRoot = join(source, "example-1.2.3");
  const member = join(packageRoot, "README.md");
  const first = join(root, "first.tar.gz");
  const second = join(root, "second.tar.gz");
  const normalizer = join(import.meta.dirname, "normalize_python_sdist.py");
  const tar =
    process.platform === "win32"
      ? join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe")
      : "tar";
  const python = process.env.PYTHON ?? (process.platform === "win32" ? "python" : "python3");
  mkdirSync(packageRoot, { recursive: true });
  writeFileSync(member, "deterministic contents\n");
  try {
    utimesSync(member, new Date(1_700_000_000_000), new Date(1_700_000_000_000));
    execFileSync(tar, ["-czf", first, "-C", source, "example-1.2.3"]);
    utimesSync(member, new Date(1_800_000_000_000), new Date(1_800_000_000_000));
    execFileSync(tar, ["-czf", second, "-C", source, "example-1.2.3"]);
    assert.notDeepEqual(readFileSync(first), readFileSync(second));
    execFileSync(python, [normalizer, first, "1"]);
    execFileSync(python, [normalizer, second, "1"]);
    assert.deepEqual(readFileSync(first), readFileSync(second));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("candidate bytes must match the trusted release build for every asset", () => {
  const files = expectedSdkFiles("1.2.3");
  const trusted = Object.fromEntries(
    Object.values(files).map((file, index) => [file, String(index + 1).repeat(64)]),
  );
  assert.doesNotThrow(() => validateTrustedBytes(files, { ...trusted }, trusted));
  assert.throws(
    () => validateTrustedBytes(files, { ...trusted, [files.wheel]: "f".repeat(64) }, trusted),
    /is not the byte-exact artifact built from the release commit/u,
  );
  assert.throws(
    () =>
      validateTrustedBytes(
        files,
        { ...trusted },
        Object.fromEntries(
          Object.entries(trusted).filter(([file]) => file !== files.manifest),
        ),
    ),
    /trusted release build does not contain/u,
  );
});

test("PyPI recovery publishes only missing files and rejects conflicts", () => {
  const expected = { "one.whl": "1".repeat(64), "one.tar.gz": "2".repeat(64) };
  assert.deepEqual(assessPypiRelease(undefined, expected), {
    publish: true,
    reason: "version is absent",
  });
  assert.equal(
    assessPypiRelease(
      { urls: [{ filename: "one.whl", digests: { sha256: "1".repeat(64) } }] },
      expected,
    ).publish,
    true,
  );
  assert.equal(
    assessPypiRelease(
      {
        urls: Object.entries(expected).map(([filename, digest]) => ({
          filename,
          digests: { sha256: digest },
        })),
      },
      expected,
    ).publish,
    false,
  );
  assert.throws(() =>
    assessPypiRelease(
      { urls: [{ filename: "one.whl", digests: { sha256: "9".repeat(64) } }] },
      expected,
    ),
  );
});
