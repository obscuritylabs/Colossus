#!/usr/bin/env node

// Operator distribution only. Release bytes are data: never extract or execute them.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { constants, createReadStream } from "node:fs";
import { lstat, mkdir, open, readFile, readdir, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { pathToFileURL } from "node:url";

export const repository = "obscuritylabs/Colossus";
export const artifactType = "application/vnd.colossus.release.v1";
export const limits = { files: 256, file: 2 ** 31, total: 8 * 2 ** 30 };
const digestPattern = /^sha256:[0-9a-f]{64}$/u;
const sortNames = (a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
const json = (value) => `${JSON.stringify(value, null, 2)}\n`;

export function validateTag(tag) {
  assert.match(tag, /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-preview\.[1-9][0-9]*)?$/u, "Use an exact vX.Y.Z or vX.Y.Z-preview.N release tag");
  assert.ok(tag.length <= 128, "Release tag is too long");
  return tag;
}

export function validateRelease(release, assets, tag, sourceCommit) {
  validateTag(tag);
  assert.equal(release.tag_name, tag, "Release tag mismatch");
  assert.equal(release.draft, false, "Draft releases cannot be distributed");
  assert.equal(release.prerelease, tag.includes("-preview."), "Release channel mismatch");
  assert.match(sourceCommit, /^[0-9a-f]{40}$/u, "Unresolved release commit");
  assert.ok(Number.isSafeInteger(release.id) && release.id > 0, "Invalid release ID");
  assert.ok(typeof release.published_at === "string" && Number.isFinite(Date.parse(release.published_at)), "Release must be published");
  assert.equal(release.html_url, `https://github.com/${repository}/releases/tag/${tag}`);
  assert.ok(Array.isArray(assets) && assets.length > 0 && assets.length <= limits.files, "Release asset count is out of bounds");
  const names = new Set();
  const ids = new Set();
  let total = 0;
  const inventory = assets.map((asset) => {
    assert.match(asset.name, /^[A-Za-z0-9][A-Za-z0-9._+-]{0,199}$/u, "Unsafe release asset name");
    assert.ok(!asset.name.includes(".."), "Unsafe release asset name");
    const folded = asset.name.toLowerCase();
    assert.ok(!names.has(folded), "Duplicate release asset name");
    names.add(folded);
    assert.ok(Number.isSafeInteger(asset.id) && asset.id > 0 && !ids.has(asset.id), "Invalid or duplicate asset ID");
    ids.add(asset.id);
    assert.equal(asset.state, "uploaded", "Release contains an incomplete upload");
    assert.ok(Number.isSafeInteger(asset.size) && asset.size > 0 && asset.size <= limits.file, "Release asset size is out of bounds");
    total += asset.size;
    assert.ok(total <= limits.total, "Release exceeds total byte limit");
    assert.match(asset.digest ?? "", digestPattern, "Every asset needs a GitHub SHA-256 digest; legacy undigested releases are unsupported");
    assert.equal(asset.browser_download_url, `https://github.com/${repository}/releases/download/${tag}/${asset.name}`, "Unexpected release download URL");
    return { name: asset.name, id: asset.id, size: asset.size, digest: asset.digest };
  }).sort(sortNames);
  return {
    schemaVersion: 1, repository, tag, sourceCommit,
    releaseId: release.id, publishedAt: release.published_at,
    prerelease: release.prerelease, releaseUrl: release.html_url, assets: inventory,
  };
}

function command(executable, args, options = {}) {
  try {
    return execFileSync(executable, args, {
      encoding: "utf8", maxBuffer: 4 * 1024 * 1024, timeout: 10 * 60 * 1000,
      stdio: ["ignore", "pipe", "pipe"], ...options,
    }).trim();
  } catch {
    // Do not print subprocess buffers: registry errors may contain credentials/URLs.
    throw new Error(`${executable} ${args[0]} failed; check authentication, permissions, and connectivity`);
  }
}

export function loadRelease(tag) {
  validateTag(tag);
  const api = (path, extra = []) => JSON.parse(command("gh", ["api", path, ...extra], { timeout: 60_000 }));
  const release = api(`repos/${repository}/releases/tags/${tag}`);
  assert.ok(Number.isSafeInteger(release.id) && release.id > 0, "Invalid release ID");
  const pages = api(`repos/${repository}/releases/${release.id}/assets?per_page=100`, ["--paginate", "--slurp"]);
  const commit = api(`repos/${repository}/commits/${tag}`).sha;
  return validateRelease(release, pages.flat(), tag, commit);
}

export function validateDownloadUrl(value, initial = false) {
  const url = new URL(value);
  assert.equal(url.protocol, "https:", "Release downloads require HTTPS");
  assert.ok(!url.username && !url.password && !url.port && !url.hash, "Unsafe release download URL");
  assert.ok((initial ? ["github.com"] : ["github.com", "release-assets.githubusercontent.com"]).includes(url.hostname), "Unapproved release download origin");
  return url;
}

export async function downloadAsset(asset, tag, destination, request = fetch) {
  let url = validateDownloadUrl(`https://github.com/${repository}/releases/download/${tag}/${asset.name}`, true);
  const signal = AbortSignal.timeout(5 * 60 * 1000);
  let response;
  for (let redirects = 0; redirects <= 4; redirects += 1) {
    // Deliberately anonymous. No registry or GitHub token follows an asset redirect.
    response = await request(url, { redirect: "manual", signal });
    if (![301, 302, 303, 307, 308].includes(response.status)) break;
    await response.body?.cancel();
    assert.ok(redirects < 4, "Too many release download redirects");
    const location = response.headers.get("location");
    assert.ok(location, "Missing release download redirect");
    url = validateDownloadUrl(new URL(location, url).href);
  }
  assert.equal(response.status, 200, "Release asset download failed");
  const length = response.headers.get("content-length");
  if (length !== null) assert.equal(Number(length), asset.size, "Asset Content-Length mismatch");
  assert.ok(response.body, "Missing release asset body");
  const file = await open(destination, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW, 0o600);
  let size = 0;
  const hash = createHash("sha256");
  try {
    for await (const chunk of response.body) {
      size += chunk.length;
      assert.ok(size <= asset.size, "Release asset exceeded its declared size");
      hash.update(chunk);
      await file.writeFile(chunk);
    }
    assert.equal(size, asset.size, "Truncated release asset");
    assert.equal(`sha256:${hash.digest("hex")}`, asset.digest, "Release asset digest mismatch");
  } finally {
    await file.close();
  }
}

async function hashFile(path, expectedSize) {
  const stat = await lstat(path);
  assert.ok(stat.isFile() && !stat.isSymbolicLink(), "Only regular asset files are allowed");
  assert.equal(stat.size, expectedSize, "Asset size mismatch");
  const hash = createHash("sha256");
  const stream = createReadStream(path, { flags: constants.O_RDONLY | constants.O_NOFOLLOW });
  let size = 0;
  for await (const chunk of stream) {
    size += chunk.length;
    assert.ok(size <= expectedSize, "Asset changed while reading");
    hash.update(chunk);
  }
  assert.equal(size, expectedSize, "Asset changed while reading");
  return `sha256:${hash.digest("hex")}`;
}

export async function verifyAssets(directory, inventory) {
  assert.ok((await lstat(directory)).isDirectory(), "Assets must be a real directory");
  const entries = (await readdir(directory)).sort();
  assert.deepEqual(entries, inventory.assets.map((asset) => asset.name).sort(), "Asset set differs from the published release");
  const assets = new Map(inventory.assets.map((asset) => [asset.name, asset]));
  for (const asset of inventory.assets) {
    assert.equal(await hashFile(join(directory, asset.name), asset.size), asset.digest, `Asset integrity failure: ${asset.name}`);
    if (asset.name.endsWith(".sha256") || asset.name.endsWith("SHA256SUMS")) {
      assert.ok(asset.size <= 1024 * 1024, "Checksum document is too large");
      const lines = (await readFile(join(directory, asset.name), "utf8")).trimEnd().split(/\r?\n/u);
      if (asset.name.endsWith(".sha256")) assert.equal(lines.length, 1, "Sidecar must contain exactly one checksum");
      const seen = new Set();
      for (const line of lines) {
        const match = /^([0-9a-f]{64}) [ *]([A-Za-z0-9][A-Za-z0-9._+-]*)$/u.exec(line);
        assert.ok(match, "Malformed release checksum");
        const [, hash, name] = match;
        assert.ok(assets.has(name) && !seen.has(name), "Checksum references an absent or duplicate asset");
        seen.add(name);
        if (asset.name.endsWith(".sha256")) assert.equal(name, asset.name.slice(0, -7), "Checksum sidecar target mismatch");
        assert.equal(`sha256:${hash}`, assets.get(name).digest, "Checksum disagrees with GitHub asset digest");
      }
    }
  }
}

export function mediaType(name) {
  if (name.endsWith(".json")) return "application/json";
  if (name.endsWith(".zip") || name.endsWith(".whl")) return "application/zip";
  if (name.endsWith(".gz") || name.endsWith(".tgz")) return "application/gzip";
  if (name.endsWith(".sha256") || name.endsWith("SHA256SUMS") || name.endsWith(".sh") || name.endsWith(".ps1")) return "text/plain";
  return "application/octet-stream";
}

export function oras(args, options) {
  return command(process.env.ORAS ?? "oras", args, options);
}

export async function buildLayout(payload, layout, inventory) {
  validateTag(inventory.tag);
  await verifyAssets(join(payload, "assets"), inventory);
  assert.deepEqual((await readdir(payload)).sort(), ["assets"], "Payload directory must contain only release assets before packaging");
  await writeFile(join(payload, "release-inventory.json"), json(inventory), { flag: "wx", mode: 0o600 });
  await mkdir(layout, { mode: 0o700 }); // Refuse reuse of a partially built layout.
  oras([
    "push", "--oci-layout", `${layout}:${inventory.tag}`, "--image-spec", "v1.1",
    "--artifact-type", artifactType,
    "--annotation", `org.opencontainers.image.created=${inventory.publishedAt}`,
    "--annotation", `org.opencontainers.image.source=https://github.com/${repository}`,
    "--annotation", `org.opencontainers.image.revision=${inventory.sourceCommit}`,
    "--annotation", `org.opencontainers.image.version=${inventory.tag}`,
    "--annotation", "org.opencontainers.image.title=Colossus release assets",
    "release-inventory.json:application/json",
    ...inventory.assets.map((asset) => `assets/${asset.name}:${mediaType(asset.name)}`),
  ], { cwd: payload });
  const digest = oras(["resolve", "--oci-layout", `${layout}:${inventory.tag}`]);
  assert.match(digest, digestPattern);
  return digest;
}

export async function verifyPulled(directory, inventory) {
  assert.deepEqual((await readdir(directory)).sort(), ["assets", "release-inventory.json"]);
  assert.equal(await readFile(join(directory, "release-inventory.json"), "utf8"), json(inventory), "Registry inventory differs from prepared release");
  await verifyAssets(join(directory, "assets"), inventory);
}

export function validateDestination(destination, tag) {
  validateTag(tag);
  assert.ok([
    `ghcr.io/obscuritylabs/colossus-release:${tag}`,
    `docker.io/obscuritylabs/colossus-release:${tag}`,
  ].includes(destination), "Destination must be the exact release tag in the approved OL repository");
  return destination;
}

export function assessExisting(result, expected, destination) {
  if (result.status === 0) {
    assert.equal(result.stdout.trim(), expected, "Registry tag already exists with different content; never overwrite a release tag");
    return true;
  }
  // Only a protocol-level missing manifest/repository permits a new publication.
  const typedNotFound = destination && result.stderr?.trim() === `Error response from registry: failed to resolve digest: ${destination}: not found`;
  assert.ok(result.status !== null && (typedNotFound || /\b(MANIFEST_UNKNOWN|NAME_UNKNOWN|404 Not Found)\b/u.test(result.stderr ?? "")), "Cannot establish registry tag state; check authentication, RBAC, and connectivity");
  return false;
}

async function prepare(tag, output) {
  const inventory = loadRelease(tag);
  await mkdir(output, { mode: 0o700 });
  const payload = join(output, "payload");
  const assets = join(payload, "assets");
  await mkdir(payload, { mode: 0o700 });
  await mkdir(assets, { mode: 0o700 });
  for (const asset of inventory.assets) {
    process.stdout.write(`Downloading ${asset.name}\n`);
    await downloadAsset(asset, tag, join(assets, asset.name));
  }
  assert.deepEqual(loadRelease(tag), inventory, "Release changed during download; start again with a fresh output directory");
  const digest = await buildLayout(payload, join(output, "layout"), inventory);
  await writeFile(join(output, "digest.txt"), `${digest}\n`, { flag: "wx", mode: 0o600 });
  console.log(`Prepared ${inventory.assets.length} assets: ${digest}`);
}

async function publish(output, destination) {
  const inventory = JSON.parse(await readFile(join(output, "payload/release-inventory.json"), "utf8"));
  validateDestination(destination, inventory.tag);
  assert.deepEqual(loadRelease(inventory.tag), inventory, "Published release no longer matches the prepared artifact");
  const auth = process.env.REGISTRY_AUTH_FILE;
  assert.ok(auth && (await lstat(auth)).isFile(), "An explicit registry auth file is required; no ambient credentials");
  const layout = `${join(output, "layout")}:${inventory.tag}`;
  const digest = oras(["resolve", "--oci-layout", layout]);
  assert.match(digest, digestPattern);
  assert.equal((await readFile(join(output, "digest.txt"), "utf8")).trim(), digest);
  const lookup = spawnSync(process.env.ORAS ?? "oras", ["resolve", "--registry-config", auth, destination], {
    encoding: "utf8", timeout: 60_000, maxBuffer: 1024 * 1024,
  });
  if (!assessExisting(lookup, digest, destination)) {
    oras(["cp", "--from-oci-layout", "--to-registry-config", auth, layout, destination]);
  }
  assert.equal(oras(["resolve", "--registry-config", auth, destination]), digest, "Published registry digest mismatch");
  const pinned = `${destination.slice(0, destination.lastIndexOf(":"))}@${digest}`;
  const pulled = join(output, "verified-pull");
  await mkdir(pulled, { mode: 0o700 });
  oras(["pull", "--registry-config", auth, "--output", pulled, pinned]);
  await verifyPulled(pulled, inventory);
  const receipt = { tag: inventory.tag, reference: destination, pinnedReference: pinned, digest, assets: inventory.assets.length };
  await writeFile(join(output, "publication.json"), json(receipt), { flag: "wx", mode: 0o600 });
  if (process.env.GITHUB_STEP_SUMMARY) {
    await writeFile(process.env.GITHUB_STEP_SUMMARY, `## Colossus release artifact\n\nPublished and pulled back all ${receipt.assets} verified release assets.\n\n\`${destination}\`\n\n\`${pinned}\`\n\nDownload: \`oras pull ${pinned} --output release-assets\`\n`, { flag: "a" });
  }
  console.log(json(receipt));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const [operation, first, second, ...extra] = process.argv.slice(2);
    assert.ok(first && second && extra.length === 0, "Usage: release-oci.mjs prepare <tag> <new-output-directory> | publish <prepared-directory> <registry-reference>");
    if (operation === "prepare") await prepare(validateTag(first), resolve(second));
    else if (operation === "publish") await publish(resolve(first), second);
    else throw new Error("Expected prepare or publish");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
