import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  assessExisting, downloadAsset, limits, repository, validateDestination,
  validateDownloadUrl, validateRelease, validateTag, verifyAssets,
} from "./release-oci.mjs";

export const tag = "v1.2.3-preview.4";
export const commit = "a".repeat(40);
const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
export const release = {
  id: 42, tag_name: tag, draft: false, prerelease: true,
  published_at: "2026-09-08T20:12:35Z", html_url: `https://github.com/${repository}/releases/tag/${tag}`,
};
export function asset(name = "example.zip", bytes = Buffer.from([0, 255, 127, 1]), id = 1) {
  return { name, id, size: bytes.length, digest: digest(bytes), state: "uploaded", browser_download_url: `https://github.com/${repository}/releases/download/${tag}/${name}` };
}
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), "colossus-release-oci-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

test("stable and preview tags are exact, shell-safe, and channel-bound", () => {
  for (const good of ["v0.0.0", "v1.2.3", tag]) assert.equal(validateTag(good), good);
  for (const bad of ["main", "latest", "v01.2.3", "v1.2.3-preview.0", "v1.2.3;id", "v1.2.3\n", "v1.2.3-rc.1"]) assert.throws(() => validateTag(bad));
  assert.throws(() => validateRelease({ ...release, draft: true }, [asset()], tag, commit));
  assert.throws(() => validateRelease({ ...release, prerelease: false }, [asset()], tag, commit));
  assert.throws(() => validateRelease({ ...release, published_at: null }, [asset()], tag, commit));
  assert.throws(() => validateRelease(release, [asset()], tag, "main"));
});

test("inventory preserves every original asset and sorts deterministically", () => {
  const assets = [asset("z.zip"), asset("a.json", Buffer.from("{}"), 2)];
  const inventory = validateRelease(release, assets, tag, commit);
  assert.deepEqual(inventory, validateRelease(release, [...assets].reverse(), tag, commit));
  assert.deepEqual(inventory.assets.map((entry) => entry.name), ["a.json", "z.zip"]);
  assert.equal(inventory.sourceCommit, commit);
  assert.equal(inventory.prerelease, true);
});

test("unsafe names, duplicates, missing digests, incomplete and oversized assets fail closed", () => {
  for (const name of ["../escape", "/absolute", "a/b", "a\\b", "a:tar", "-flag", "a\nfile", "..", "a..b"]) assert.throws(() => validateRelease(release, [asset(name)], tag, commit));
  for (const override of [{ digest: null }, { digest: "sha256:bad" }, { state: "new" }, { size: 0 }, { size: limits.file + 1 }, { id: -1 }, { browser_download_url: "https://evil.example/file" }]) assert.throws(() => validateRelease(release, [{ ...asset(), ...override }], tag, commit));
  assert.throws(() => validateRelease(release, [], tag, commit));
  assert.throws(() => validateRelease(release, [asset(), asset()], tag, commit));
  assert.throws(() => validateRelease(release, [asset("a.zip"), asset("A.zip", Buffer.from("a"), 2)], tag, commit));
  assert.throws(() => validateRelease(release, Array.from({ length: 257 }, (_, i) => asset(`f${i}`, Buffer.from("a"), i + 1)), tag, commit));
  assert.throws(() => validateRelease(release, Array.from({ length: 5 }, (_, i) => ({ ...asset(`f${i}`, Buffer.from("a"), i + 1), size: limits.file })), tag, commit));
});

test("download redirects are HTTPS-only and exact-origin, with no credentials", async (t) => {
  for (const url of ["http://github.com/x", "https://github.com.evil.example/x", "https://user:secret@github.com/x", "https://127.0.0.1/x", "https://github.com:444/x"]) assert.throws(() => validateDownloadUrl(url));
  const root = await fixture(t);
  const bytes = Buffer.from([0, 255, 127, 1]);
  let calls = 0;
  await downloadAsset(asset(), tag, join(root, "file"), async (url, options) => {
    assert.equal(options.headers, undefined);
    assert.equal(options.redirect, "manual");
    calls += 1;
    if (calls === 1) return new Response(null, { status: 302, headers: { location: "https://release-assets.githubusercontent.com/blob?signature=redacted" } });
    assert.equal(url.hostname, "release-assets.githubusercontent.com");
    return new Response(bytes, { headers: { "content-length": String(bytes.length) } });
  });
  assert.equal(calls, 2);
  assert.deepEqual(await readFile(join(root, "file")), bytes);
  await assert.rejects(downloadAsset(asset(), tag, join(root, "bad"), async () => new Response(null, { status: 302, headers: { location: "https://evil.example/x" } })));
  calls = 0;
  await assert.rejects(downloadAsset(asset(), tag, join(root, "loop"), async () => { calls += 1; return new Response(null, { status: 302, headers: { location: "https://github.com/loop" } }); }), /Too many/u);
  assert.equal(calls, 5);
});

test("truncation, digest mismatch, oversized responses and clobbering are rejected", async (t) => {
  const root = await fixture(t);
  for (const [name, bytes] of [["short", [1]], ["long", [1, 2, 3, 4, 5]], ["hash", [1, 2, 3, 4]]]) {
    await assert.rejects(downloadAsset(asset(), tag, join(root, name), async () => new Response(Buffer.from(bytes))));
  }
  await assert.rejects(downloadAsset(asset(), tag, join(root, "length"), async () => new Response("a", { headers: { "content-length": "1" } })));
  const existing = join(root, "existing");
  await writeFile(existing, "preserved");
  await assert.rejects(downloadAsset(asset(), tag, existing, async () => new Response(Buffer.from([0, 255, 127, 1]))));
  assert.equal(await readFile(existing, "utf8"), "preserved");
});

test("verification binds checksum sidecars and rejects extra, missing, linked or changed files", async (t) => {
  const root = await fixture(t);
  const bytes = Buffer.from([0, 255, 127, 1]);
  const main = asset();
  const checksum = Buffer.from(`${main.digest.slice(7)}  example.zip\n`);
  const sum = asset("example.zip.sha256", checksum, 2);
  const inventory = validateRelease(release, [main, sum], tag, commit);
  await writeFile(join(root, main.name), bytes);
  await writeFile(join(root, sum.name), checksum);
  await verifyAssets(root, inventory);
  await writeFile(join(root, "extra"), "extra");
  await assert.rejects(verifyAssets(root, inventory), /Asset set/u);
  await rm(join(root, "extra"));
  await writeFile(join(root, main.name), Buffer.from([1, 2, 3, 4]));
  await assert.rejects(verifyAssets(root, inventory), /integrity/u);
  await rm(join(root, main.name));
  await assert.rejects(verifyAssets(root, inventory), /Asset set/u);
  await symlink(join(root, sum.name), join(root, main.name));
  await assert.rejects(verifyAssets(root, inventory), /regular/u);
  await rm(join(root, main.name));
  await mkdir(join(root, main.name));
  await assert.rejects(verifyAssets(root, inventory), /regular/u);
});

test("sidecar checksum disagreement fails even when its own asset digest is valid", async (t) => {
  const root = await fixture(t);
  const bytes = Buffer.from([0, 255, 127, 1]);
  const wrong = Buffer.from(`${"0".repeat(64)}  example.zip\n`);
  await writeFile(join(root, "example.zip"), bytes);
  await writeFile(join(root, "example.zip.sha256"), wrong);
  await assert.rejects(verifyAssets(root, validateRelease(release, [asset(), asset("example.zip.sha256", wrong, 2)], tag, commit)), /disagrees/u);
});

test("only exact OL destinations and non-conflicting publication are permitted", () => {
  for (const host of ["ghcr.io", "docker.io"]) assert.equal(validateDestination(`${host}/obscuritylabs/colossus-release:${tag}`, tag), `${host}/obscuritylabs/colossus-release:${tag}`);
  for (const target of ["ghcr.io/obscuritylabs/colossus-release:latest", `evil.example/obscuritylabs/colossus-release:${tag}`, `docker.io/another-owner/colossus-release:${tag}`]) assert.throws(() => validateDestination(target, tag));
  const expected = asset().digest;
  assert.equal(assessExisting({ status: 0, stdout: `${expected}\n` }, expected), true);
  assert.equal(assessExisting({ status: 1, stderr: "HEAD: 404 Not Found" }, expected), false);
  assert.equal(assessExisting({ status: 1, stderr: "MANIFEST_UNKNOWN" }, expected), false);
  const destination = `ghcr.io/obscuritylabs/colossus-release:${tag}`;
  assert.equal(assessExisting({ status: 1, stderr: `Error response from registry: failed to resolve digest: ${destination}: not found\n` }, expected, destination), false);
  assert.throws(() => assessExisting({ status: 1, stderr: "Error response from registry: failed to resolve digest: another-repo: not found" }, expected, destination));
  assert.throws(() => assessExisting({ status: 0, stdout: `sha256:${"0".repeat(64)}` }, expected), /never overwrite/u);
  for (const stderr of ["401 Unauthorized", "403 Forbidden", "timeout", "not found", "500 Internal Server Error"]) assert.throws(() => assessExisting({ status: 1, stderr }, expected));
  assert.throws(() => assessExisting({ status: null, stderr: "" }, expected));
});

test("CI publication is separate from PR checks, uses scoped auth, and preserves exact tags", async () => {
  const source = await readFile(new URL("../../.github/workflows/release-image.yml", import.meta.url), "utf8");
  const publication = source.slice(source.indexOf("\n  publish:\n"));
  assert.ok(publication.includes("needs: test"));
  assert.ok(publication.includes("github.ref == 'refs/heads/main'"));
  assert.ok(publication.includes("github.event_name == 'release'"));
  assert.ok(publication.includes("github.event.release.draft == false"));
  assert.ok(!publication.includes("github.event_name == 'pull_request'"));
  assert.ok(!source.includes("pull_request_target"));
  assert.ok(!source.includes("contents: write"));
  assert.ok(publication.includes("packages: write"));
  assert.ok(publication.includes("--password-stdin"));
  assert.ok(publication.includes("--registry-config"));
  assert.ok(publication.includes("if: always()"));
  assert.ok(source.includes("checksum: f27adb935022d94df8dc77719c322dda592c78a0d57a6f7dcdd8d900b248c454"));
  assert.ok(!source.includes("colossus-release:latest"));
});
