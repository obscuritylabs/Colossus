import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { createReadStream } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const serverTargets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
const sha256 = /^sha256:[a-f0-9]{64}$/u;
const imageMediaTypes = new Set([
  "application/vnd.oci.image.manifest.v1+json",
  "application/vnd.docker.distribution.manifest.v2+json",
]);
export function validateRequest(tag, commit) {
  if (typeof tag !== "string" || tag.trim() !== tag || tag.length > 128 || !/^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-preview\.[1-9]\d*)?$/u.test(tag))
    throw new Error("A stable or preview release tag is required.");
  if (commit !== undefined && (typeof commit !== "string" || commit.trim() !== commit || !/^[a-f0-9]{40}$/u.test(commit)))
    throw new Error("An exact source commit is required.");
}
async function digest(path) {
  const file = await lstat(path);
  if (!file.isFile()) throw new Error("Release assets must be regular files.");
  const hash = createHash("sha256");
  for await (const block of createReadStream(path)) hash.update(block);
  return { hash: hash.digest("hex"), size: file.size };
}
async function verify(directory, filename) {
  const checksumPath = join(directory, `${filename}.sha256`);
  const checksumFile = await lstat(checksumPath);
  if (!checksumFile.isFile() || checksumFile.size > 1024)
    throw new Error("Checksum sidecar is invalid.");
  const checksum = (await readFile(checksumPath, "utf8")).trimEnd();
  const actual = await digest(join(directory, filename));
  if (checksum !== `${actual.hash}  ${filename}`)
    throw new Error("Release asset checksum or filename disagrees.");
  return actual;
}
// Mutable release sidecars alone are insufficient: compare every byte to the successful
// protected tag build's Actions artifact before Docker loads or publishes an image.
export async function verifyControlPlaneAssets(trusted, published, tag) {
  validateRequest(tag);
  const inventory = [];
  for (const target of serverTargets) {
    for (const suffix of ["tar.gz", "docker.tar.gz"]) {
      const filename = `Colossus-Control-Plane-${tag}-${target}.${suffix}`;
      const expected = await verify(trusted, filename);
      const actual = await verify(published, filename);
      if (actual.hash !== expected.hash || actual.size !== expected.size)
        throw new Error("Release asset differs from the trusted tag build.");
      inventory.push({ filename, sha256: actual.hash, size: actual.size });
    }
  }
  return inventory;
}

export function savedImageConfigPath(manifest, imageName = "colossus-control-plane:release") {
  if (!Array.isArray(manifest) || manifest.length !== 1 ||
      !Array.isArray(manifest[0].RepoTags) || manifest[0].RepoTags.length !== 1 ||
      manifest[0].RepoTags[0] !== imageName ||
      typeof manifest[0].Config !== "string" ||
      !/^(?:blobs\/sha256\/[a-f0-9]{64}|[a-f0-9]{64}\.json)$/u.test(manifest[0].Config))
    throw new Error("Saved image must contain exactly the trusted release image.");
  return manifest[0].Config;
}

export function verifySavedImageConfig(manifest, configBytes, { arch, tag, commit, imageName }) {
  if (typeof commit !== "string") throw new Error("An exact source commit is required.");
  validateRequest(tag, commit);
  if (!["amd64", "arm64"].includes(arch)) throw new Error("Unsupported image architecture.");
  const path = savedImageConfigPath(manifest, imageName);
  const expected = path.startsWith("blobs/") ? path.slice("blobs/sha256/".length) : path.slice(0, -5);
  if (createHash("sha256").update(configBytes).digest("hex") !== expected)
    throw new Error("Saved image config digest disagrees with its bytes.");
  const config = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(configBytes));
  if (config.os !== "linux" || config.architecture !== arch || config.config?.User !== "10001:10001" ||
      config.config?.Labels?.["org.opencontainers.image.revision"] !== commit ||
      config.config?.Labels?.["org.opencontainers.image.version"] !== tag)
    throw new Error("Saved image config differs from the trusted release identity.");
  return { architecture: arch, config_digest: `sha256:${expected}` };
}

// Docker save includes manifest.json in both classic and containerd archives. Read only
// its bounded config member, never extract archive paths or assume Docker .Id is a config
// digest: containerd can report the parent index (including attestations) instead.
export function readSavedImageConfig(archive, options) {
  const member = path => execFileSync("tar", ["-xOf", archive, path], { maxBuffer: 1024 * 1024 });
  const manifest = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(member("manifest.json")));
  const path = savedImageConfigPath(manifest, options.imageName);
  return verifySavedImageConfig(manifest, member(path), options);
}

export function verifyExecutableManifest(manifest, configDigest) {
  if (!sha256.test(configDigest) || manifest?.schemaVersion !== 2 ||
      !imageMediaTypes.has(manifest.mediaType) || manifest.manifests !== undefined ||
      manifest.config?.digest !== configDigest || !Array.isArray(manifest.layers) ||
      manifest.layers.length === 0 || !manifest.layers.every(layer => sha256.test(layer.digest)))
    throw new Error("Refusing a conflicting config or non-executable platform manifest.");
}

export function executableDescriptorDigest(descriptor) {
  if (!sha256.test(descriptor?.digest) || !imageMediaTypes.has(descriptor.mediaType) || descriptor.manifests !== undefined)
    throw new Error("An immutable single-platform executable manifest is required.");
  return descriptor.digest;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [trusted, published, tag] = process.argv.slice(2);
  if (!trusted || !published || !tag || process.argv.length !== 5)
    throw new Error("usage: verify-control-plane-assets.mjs TRUSTED PUBLISHED TAG");
  console.log(JSON.stringify(await verifyControlPlaneAssets(trusted, published, tag)));
}
