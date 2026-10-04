#!/usr/bin/env node
// Release/dev setup only. Audio is never involved in model acquisition.
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { copyFile, lstat, mkdir, readFile, rename, rm } from "node:fs/promises";
import { get } from "node:https";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { Transform } from "node:stream";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pin = JSON.parse(
  await readFile(join(repository, "release/dictation/models.json"), "utf8"),
);
const model = pin.models.tiny_english;
const hosts = new Set([
  "huggingface.co",
  "cdn-lfs.huggingface.co",
  "cdn-lfs-us-1.hf.co",
  "cas-bridge.xethub.hf.co",
  "us.aws.cdn.hf.co",
]);
const args = process.argv.slice(2);
const options = new Map();
if (args.length % 2 !== 0) throw new Error("expected paired stage arguments");
for (let index = 0; index < args.length; index += 2) {
  if (
    !["--model-file", "--output"].includes(args[index]) ||
    options.has(args[index]) ||
    !args[index + 1]
  )
    throw new Error("unknown or duplicate stage argument");
  options.set(args[index], args[index + 1]);
}
const directory = resolve(
  options.get("--output") ??
    join(repository, "apps/desktop/src-tauri/dictation-assets"),
);
await mkdir(directory, { recursive: true });
if (!(await lstat(directory)).isDirectory())
  throw new Error("model output must be a regular directory");
const destination = join(directory, model.filename);
async function verified(path) {
  const metadata = await lstat(path);
  if (
    !metadata.isFile() ||
    metadata.nlink !== 1 ||
    metadata.size !== model.bytes
  )
    return false;
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex") === model.sha256;
}
function upstream(url, redirects = 0) {
  if (
    redirects > 5 ||
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    (url.port && url.port !== "443") ||
    !hosts.has(url.hostname)
  )
    throw new Error("model redirect outside reviewed hosts");
  return new Promise((resolveResponse, reject) => {
    const request = get(url, { timeout: 30000 }, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
        response.resume();
        if (!response.headers.location)
          return reject(new Error("model redirect has no location"));
        try {
          upstream(new URL(response.headers.location, url), redirects + 1).then(
            resolveResponse,
            reject,
          );
        } catch (error) {
          reject(error);
        }
      } else if (response.statusCode === 200) resolveResponse(response);
      else {
        response.resume();
        reject(new Error(`model HTTP ${response.statusCode}`));
      }
    });
    request.on("timeout", () =>
      request.destroy(new Error("model download timed out")),
    );
    request.on("error", reject);
  });
}
if (!(await verified(destination).catch(() => false))) {
  const temporary = `${destination}.${process.pid}.part`;
  try {
    if (options.has("--model-file")) {
      const source = resolve(options.get("--model-file"));
      if (!(await verified(source)))
        throw new Error("local model checksum/size mismatch");
      await copyFile(source, temporary);
    } else {
      let bytes = 0;
      const bounded = new Transform({
        transform(chunk, encoding, callback) {
          bytes += chunk.length;
          callback(
            bytes > model.bytes ? new Error("model exceeds pinned size") : null,
            chunk,
          );
        },
      });
      const response = await upstream(
        new URL(`${pin.upstream}/resolve/${pin.revision}/${model.filename}`),
      );
      await pipeline(
        response,
        bounded,
        createWriteStream(temporary, { flags: "wx", mode: 0o644 }),
      );
    }
    if (!(await verified(temporary)))
      throw new Error("model checksum/size mismatch");
    await rename(temporary, destination);
  } finally {
    await rm(temporary, { force: true });
  }
}
for (const name of ["models.json", "LICENSE-MIT"])
  await copyFile(
    join(repository, "release/dictation", name),
    join(directory, name),
  );
console.log("Staged verified Tiny English dictation model (78 MB).");
