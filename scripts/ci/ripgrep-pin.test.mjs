import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const pin = JSON.parse(readFileSync(join(repository, "release/ripgrep.json"), "utf8"));

test("ripgrep release pin covers every native CLI target with complete digests", () => {
  assert.equal(pin.version, "15.2.0");
  assert.equal(
    pin.upstreamRelease,
    `https://github.com/BurntSushi/ripgrep/releases/tag/${pin.version}`,
  );
  assert.deepEqual(pin.license, ["MIT", "Unlicense"]);
  assert.deepEqual(Object.keys(pin.archives).sort(), [
    "aarch64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-musl",
  ]);
  for (const digest of Object.values(pin.archives)) {
    assert.match(digest, /^[0-9a-f]{64}$/u);
  }
});
