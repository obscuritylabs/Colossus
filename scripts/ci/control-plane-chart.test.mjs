import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { verifyCandidate, verifySource, verifyVersions, chartName } from "./control-plane-chart.mjs";

test("chart publication binds exact candidate bytes, source identity, and version", () => {
  const bytes = Buffer.from("trusted chart archive");
  const commit = "a".repeat(40);
  const candidate = { tag: "v0.11.9", commit, version: "0.11.9", name: chartName,
    archive: `${chartName}-0.11.9.tgz`, sha256: createHash("sha256").update(bytes).digest("hex") };
  assert.equal(verifyCandidate(candidate, bytes, candidate.tag, commit), candidate.sha256);
  for (const changed of [{ tag: "v0.11.8" }, { commit: "b".repeat(40) }, { version: "0.11.8" },
    { name: "other" }, { archive: "../other.tgz" }, { sha256: "0".repeat(64) }]) {
    assert.throws(() => verifyCandidate({ ...candidate, ...changed }, bytes, candidate.tag, commit));
  }
  assert.throws(() => verifyCandidate(candidate, Buffer.from("substituted"), candidate.tag, commit));
});
test("chart and workspace versions must equal the published stable or preview tag", () => {
  for (const version of ["0.11.9", "0.12.0-preview.1"]) {
    const chart = `name: ${chartName}\nversion: ${version}\nappVersion: "${version}"\n`;
    const cargo = `[workspace.package]\nversion = "${version}"\nedition = "2024"\n`;
    assert.equal(verifyVersions(chart, cargo, `v${version}`), version);
    assert.throws(() => verifyVersions(chart.replace(`appVersion: "${version}"`, 'appVersion: "0.0.0"'), cargo, `v${version}`));
    assert.throws(() => verifyVersions(chart, cargo, "v0.0.1"));
  }
});

test("candidate creation rejects a different or dirty source checkout", () => {
  const commit = "a".repeat(40);
  assert.doesNotThrow(() => verifySource(commit, commit, ""));
  assert.throws(() => verifySource(commit, "b".repeat(40), ""));
  assert.throws(() => verifySource(commit, commit, " M deploy/control-plane/chart/values.yaml\n"));
  assert.throws(() => verifySource(commit, commit, "?? deploy/control-plane/chart/extra-values.yaml\n"));
});
