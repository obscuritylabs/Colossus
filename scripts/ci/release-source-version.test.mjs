import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { resolveSourceVersion } from "./release-source-version.mjs";

test("stable and versioned preview releases still require matching source versions", () => {
  assert.equal(resolveSourceVersion("v0.11.2", "0.11.2"), "0.11.2");
  assert.equal(resolveSourceVersion("v0.11.2-preview.1", "0.11.2-preview.1"), "0.11.2-preview.1");
  for (const [tag, version] of [
    ["v0.11.3", "0.11.2"],
    ["v0.11.2", "0.11.2-preview.1"],
    ["v0.11.2-preview.2", "0.11.2-preview.1"],
    ["v0.11.3-preview.1", "0.11.2"],
  ]) assert.throws(() => resolveSourceVersion(tag, version));
});

test("a test preview may retain its exact stable source version without a bump", () => {
  assert.equal(resolveSourceVersion("v0.11.2-preview.1", "0.11.2"), "0.11.2");
  assert.equal(resolveSourceVersion("v0.11.2-preview.2", "0.11.2"), "0.11.2");
});

test("malformed or moving version identifiers are rejected", () => {
  for (const tag of ["main", "latest", "0.11.2", "v00.11.2", "v0.11.2-test", "v0.11.2-preview.0", "v0.11.2-preview.01", "v0.11.2\n", "v0.11.2+test"]) {
    assert.throws(() => resolveSourceVersion(tag, "0.11.2"));
  }
  for (const version of ["main", "v0.11.2", "00.11.2", "0.11.2\n", "0.11.2+test"]) {
    assert.throws(() => resolveSourceVersion("v0.11.2-preview.1", version));
  }
});

test("the workflow helper reports a source version or fails without output", () => {
  const script = new URL("./release-source-version.mjs", import.meta.url);
  const run = (...args) => spawnSync(process.execPath, [fileURLToPath(script), ...args], { encoding: "utf8" });
  const accepted = run("v0.11.2-preview.1", "0.11.2");
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.equal(accepted.stdout.trim(), "0.11.2");
  for (const args of [["v0.11.3", "0.11.2"], ["v0.11.2"], ["v0.11.2", "0.11.2", "extra"]]) {
    const rejected = run(...args);
    assert.equal(rejected.status, 1);
    assert.equal(rejected.stdout, "");
  }
});

test("test releases keep signing and omit incompatible bootstrap installers", () => {
  const workflow = readFileSync(new URL("../../.github/workflows/release.yml", import.meta.url), "utf8");
  assert.ok(workflow.includes('version=$(node scripts/ci/release-source-version.mjs "$tag" "$workspace_version")'));
  const bootstrap = workflow.slice(workflow.indexOf("  bootstrap_installers:"), workflow.indexOf("  sdk_release:"));
  assert.ok(bootstrap.includes("if: needs.validate.outputs.test_release != 'true'"));
  const gate = workflow.slice(workflow.indexOf("  gate:"), workflow.indexOf("  draft-release:"));
  assert.ok(gate.includes('test "$BOOTSTRAP_RESULT" = skipped'));
  assert.ok(gate.includes('windows_cli_sign="$WINDOWS_CLI_SIGN_RESULT"'));
  assert.ok(gate.includes('desktop_windows_signed="$WINDOWS_SIGNED_DESKTOP_RESULT"'));
  const draft = workflow.slice(workflow.indexOf("  draft-release:"));
  assert.ok(draft.includes('expected_assets=38'));
  assert.ok(gate.includes('control_plane=${{ needs.control_plane.result }}'));
  assert.ok(gate.includes('vscode=${{ needs.vscode.result }}'));
  assert.ok(draft.includes('Colossus-Control-Plane-${RELEASE_TAG}-${target}.docker.tar.gz'));
  assert.ok(draft.includes('Colossus-VSCode-${RELEASE_TAG}-${target}.vsix'));
  assert.ok(draft.includes('if [ "$TEST_RELEASE" != true ]; then'));
  assert.ok(draft.includes("CLI archives retain the source version"));
});
