import assert from "node:assert/strict";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const versionPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-preview\.[1-9][0-9]*)?$/u;

export function resolveSourceVersion(tag, sourceVersion) {
  assert.equal(typeof tag, "string");
  assert.ok(tag.startsWith("v"), "Release tags must start with v");
  const tagVersion = tag.slice(1);
  assert.equal(typeof sourceVersion, "string");
  assert.equal(tagVersion.match(versionPattern)?.[0], tagVersion, "Invalid release tag");
  assert.equal(sourceVersion.match(versionPattern)?.[0], sourceVersion, "Invalid source version");
  assert.ok(
    tagVersion === sourceVersion ||
      (tagVersion.includes("-preview.") &&
        tagVersion.split("-preview.")[0] === sourceVersion),
    "Release tag must match the source version or preview that exact stable version",
  );
  return sourceVersion;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    assert.equal(process.argv.length, 4, "Usage: release-source-version.mjs <tag> <source-version>");
    console.log(resolveSourceVersion(process.argv[2], process.argv[3]));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
