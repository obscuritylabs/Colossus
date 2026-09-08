// Mandatory ORAS integration check in release-image.yml; no registry or socket needed.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { artifactType, buildLayout, oras, validateRelease, verifyPulled } from "./release-oci.mjs";

const root = await mkdtemp(join(tmpdir(), "colossus-release-oci-roundtrip-"));
const tag = "v1.2.3-preview.4";
const base = "https://github.com/obscuritylabs/Colossus/releases";
const bytes = Buffer.from([0, 255, 127, 1]);
const asset = { name: "binary.zip", id: 1, size: bytes.length, digest: `sha256:${createHash("sha256").update(bytes).digest("hex")}`, state: "uploaded", browser_download_url: `${base}/download/${tag}/binary.zip` };
const inventory = validateRelease({ id: 42, tag_name: tag, draft: false, prerelease: true, published_at: "2026-09-08T20:12:35Z", html_url: `${base}/tag/${tag}` }, [asset], tag, "a".repeat(40));
try {
  const digests = [];
  for (const name of ["first", "second"]) {
    const payload = join(root, name);
    await mkdir(join(payload, "assets"), { recursive: true });
    await writeFile(join(payload, "assets/binary.zip"), bytes);
    const layout = join(root, `${name}-layout`);
    const digest = await buildLayout(payload, layout, inventory);
    digests.push(digest);
    const manifest = JSON.parse(oras(["manifest", "fetch", "--oci-layout", `${layout}@${digest}`]));
    assert.equal(manifest.artifactType, artifactType);
    assert.equal(manifest.mediaType, "application/vnd.oci.image.manifest.v1+json");
    assert.equal(manifest.layers.length, 2);
    assert.equal(manifest.layers[1].digest, asset.digest);
    assert.equal(manifest.layers[1].annotations["org.opencontainers.image.title"], "assets/binary.zip");
    const pulled = join(root, `${name}-pull`);
    await mkdir(pulled);
    oras(["pull", "--oci-layout", `${layout}@${digest}`, "--output", pulled]);
    await verifyPulled(pulled, inventory);
    assert.deepEqual(await readFile(join(pulled, "assets/binary.zip")), bytes);
  }
  assert.equal(digests[0], digests[1], "Identical release payloads must produce identical OCI identities");
  console.log(`PASS: real ORAS offline round trip, binary fidelity, and deterministic digest ${digests[0]}`);
} finally {
  await rm(root, { recursive: true, force: true });
}
