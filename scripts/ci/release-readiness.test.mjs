import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import test from "node:test";

function verifySources(t, files) {
  const root = mkdtempSync(join(tmpdir(), "colossus-release-readiness."));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, "bin");
  mkdirSync(bin);
  mkdirSync(join(root, "release"));
  copyFileSync(new URL("../../release/verify-release-readiness.sh", import.meta.url),
    join(root, "release/verify-release-readiness.sh"));
  // The source inventory guard uses real Git. Only unrelated compiler/scanner
  // prerequisites are stubbed so these cases cannot invoke full release builds.
  for (const [tool, version] of [
    ["rustc", "rustc 1.96.0 (fixture)"],
    ["cargo-deny", "cargo-deny 0.20.2"],
    ["cargo-audit", "cargo-audit 0.22.2"],
    ["cargo", ""],
  ]) {
    writeFileSync(join(bin, tool),
      `#!/bin/sh\nif [ "$1" = --version ]; then printf '%s\\n' '${version}'; fi\nexit 0\n`,
      { mode: 0o755 });
  }
  for (const file of files) {
    const path = join(root, file);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, "# source inventory fixture\n");
  }
  const git = (...args) => {
    const result = spawnSync("git", args, { cwd: root, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
  };
  git("-c", "init.templateDir=", "init", "--quiet");
  if (files.length) git("add", "--", ...files);
  return spawnSync("sh", ["release/verify-release-readiness.sh"], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, PATH: bin + delimiter + process.env.PATH },
    timeout: 10_000,
  });
}

test("release readiness permits maintained SDK, build, and plugin Python sources", (t) => {
  const result = verifySources(t, [
    "sdk/python/src/colossus_sdk/client.py",
    "deploy/documentation/build-config.py",
    "scripts/ci/normalize_python_sdist.py",
    "examples/sdk/integration/server.py",
    "examples/sdk/provider-failure/server.py",
    "bundled-plugins/colossus/skills/security-review/scripts/init_review.py",
    "scripts/tests/test_security_review_workspace.py",
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /local release-readiness verification passed/u);
});

for (const file of [
  "pyproject.toml",
  "colossus/runtime.py",
  "bundled-plugins/colossus/skills/security-review/scripts/unapproved.py",
  "scripts/tests/unapproved.py",
]) {
  test("release readiness rejects unapproved source " + file, (t) => {
    const result = verifySources(t, [file]);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /retired root Python package/u);
    assert.doesNotMatch(result.stdout, /local release-readiness verification passed/u);
  });
}
