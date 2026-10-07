import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { serverTargets, validateRequest, verifyControlPlaneAssets, savedImageConfigPath,
  verifySavedImageConfig, verifyExecutableManifest, executableDescriptorDigest } from "./verify-control-plane-assets.mjs";
import { readSavedImageConfig } from "./verify-control-plane-assets.mjs";

test("privileged publication uses the protected publisher revision that passed contracts", async () => {
  const workflow = await readFile(new URL("../../.github/workflows/control-plane-image.yml", import.meta.url), "utf8");
  const resolver = workflow.split("  publisher-revision:\n")[1]?.split("  contracts:\n")[0];
  assert.ok(resolver, "Publication must resolve its trusted publisher before either consumer runs");
  assert.match(resolver, /if \[\[ "\$EVENT_NAME" == pull_request \]\]; then\s+revision="\$GITHUB_SHA"\s+else\s+revision=\$\(gh api repos\/obscuritylabs\/Colossus\/git\/ref\/heads\/main --jq \.object\.sha\)/u);
  assert.match(resolver, /\[\[ "\$revision" =~ \^\[0-9a-f\]\{40\}\$ \]\]/u);
  assert.equal(workflow.split("ref: ${{ needs.publisher-revision.outputs.revision }}").length - 1, 2,
    "Contracts and credentialed publication must check out the same resolved immutable SHA");
  assert.doesNotMatch(workflow, /ref: main\b/u, "A moving main ref can bypass the checked publisher revision");
  const contracts = workflow.split("  contracts:\n")[1]?.split("  publish:\n")[0];
  assert.match(contracts, /needs: publisher-revision/u);
  const publish = workflow.split("  publish:\n")[1];
  assert.match(publish, /needs: \[publisher-revision, contracts\]/u);
});

test("publication rejects path/tag injection and mutable release substitutions", async () => {
  assert.throws(() => validateRequest("../../secret"));
  assert.throws(() => validateRequest("v0.11.7-preview.0"));
  assert.throws(() => validateRequest("v0.11.7\n"));
  assert.throws(() => validateRequest("v0.11.7", "main"));
  const root = await mkdtemp(join(tmpdir(), "colossus-image-candidate-"));
  const tag = "v0.11.7-preview.3";
  const trusted = join(root, "trusted");
  const published = join(root, "published");
  try {
    await mkdir(trusted); await mkdir(published);
    async function asset(directory, filename, bytes) {
      await writeFile(join(directory, filename), bytes);
      await writeFile(join(directory, `${filename}.sha256`), `${createHash("sha256").update(bytes).digest("hex")}  ${filename}\n`);
    }
    for (const target of serverTargets) for (const suffix of ["tar.gz", "docker.tar.gz"]) {
      const filename = `Colossus-Control-Plane-${tag}-${target}.${suffix}`;
      await asset(trusted, filename, "trusted tested candidate");
      await asset(published, filename, "trusted tested candidate");
    }
    assert.equal((await verifyControlPlaneAssets(trusted, published, tag)).length, 4);
    const filename = `Colossus-Control-Plane-${tag}-${serverTargets[0]}.docker.tar.gz`;
    // An attacker can replace a release asset and recompute its sidecar, but cannot
    // replace the independently retained successful tag build in this trust model.
    await asset(published, filename, "substituted image with a valid recomputed sidecar");
    await assert.rejects(verifyControlPlaneAssets(trusted, published, tag), /trusted tag build/u);
    await asset(published, filename, "trusted tested candidate");
    await writeFile(join(published, `${filename}.sha256`), `${"0".repeat(64)}  another-file\n`);
    await assert.rejects(verifyControlPlaneAssets(trusted, published, tag), /checksum or filename/u);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("classic and containerd archives identify the executable config independently of an index", async () => {
  const identity = { arch: "arm64", tag: "v0.11.7-preview.3", commit: "a".repeat(40) };
  const config = Buffer.from(JSON.stringify({
    architecture: identity.arch, os: "linux", config: { User: "10001:10001", Labels: {
      "org.opencontainers.image.revision": identity.commit,
      "org.opencontainers.image.version": identity.tag,
    } },
  }));
  const hash = createHash("sha256").update(config).digest("hex");
  const root = await mkdtemp(join(tmpdir(), "colossus-image-config-"));
  try { for (const [position, path] of [`${hash}.json`, `blobs/sha256/${hash}`].entries()) {
    const manifest = [{ Config: path, RepoTags: ["colossus-control-plane:release"], Layers: [] }];
    assert.equal(savedImageConfigPath(manifest), path);
    assert.deepEqual(verifySavedImageConfig(manifest, config, identity), {
      architecture: identity.arch, config_digest: `sha256:${hash}`,
    });
    assert.throws(() => verifySavedImageConfig(manifest, Buffer.from("substitution"), identity), /config digest/u);
    assert.throws(() => verifySavedImageConfig(manifest, config, { ...identity, arch: "amd64" }), /release identity/u);
    assert.throws(() => verifySavedImageConfig(manifest, config, { ...identity, commit: undefined }), /source commit/u);
    const stage = join(root, `stage-${position}`);
    await mkdir(dirname(join(stage, path)), { recursive: true });
    await writeFile(join(stage, "manifest.json"), JSON.stringify(manifest));
    await writeFile(join(stage, path), config);
    const archive = join(root, `image-${position}.tar.gz`);
    execFileSync("tar", ["-czf", archive, "-C", stage, "manifest.json", path]);
    assert.deepEqual(readSavedImageConfig(archive, identity), {
      architecture: identity.arch, config_digest: `sha256:${hash}`,
    });
  } } finally { await rm(root, { recursive: true, force: true }); }
  assert.throws(() => savedImageConfigPath([{ Config: "../../config", RepoTags: ["colossus-control-plane:release"] }]));
  assert.throws(() => savedImageConfigPath([{ Config: `${hash}.json`, RepoTags: ["other:release"] }]));
});

test("publication accepts executable manifests and refuses conflicting configs or attestation indexes", () => {
  const configDigest = `sha256:${"c".repeat(64)}`;
  const executableDigest = `sha256:${"e".repeat(64)}`;
  for (const mediaType of ["application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json"]) {
    const manifest = { schemaVersion: 2, mediaType, config: { digest: configDigest },
      layers: [{ digest: `sha256:${"d".repeat(64)}` }] };
    assert.doesNotThrow(() => verifyExecutableManifest(manifest, configDigest));
    assert.throws(() => verifyExecutableManifest(manifest, `sha256:${"f".repeat(64)}`), /conflicting config/u);
    assert.equal(executableDescriptorDigest({ mediaType, digest: executableDigest }), executableDigest);
  }
  const index = { schemaVersion: 2, mediaType: "application/vnd.oci.image.index.v1+json", digest: `sha256:${"a".repeat(64)}`,
    manifests: [
      { digest: executableDigest, platform: { os: "linux", architecture: "arm64" } },
      { digest: `sha256:${"b".repeat(64)}`, platform: { os: "unknown", architecture: "unknown" },
        annotations: { "vnd.docker.reference.type": "attestation-manifest", "vnd.docker.reference.digest": executableDigest } },
    ] };
  assert.throws(() => verifyExecutableManifest(index, configDigest), /non-executable/u);
  assert.throws(() => executableDescriptorDigest(index), /single-platform/u);
});
