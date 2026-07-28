import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";

import { buildDesktopUpdateManifest } from "../write-desktop-update-manifest.mjs";

function fixture(channel) {
  const directory = mkdtempSync(join(tmpdir(), "colossus-update-manifest-"));
  const tag = channel === "stable" ? "v1.2.3" : "v1.2.3-preview.4";
  const version = tag.slice(1);
  const macName =
    channel === "stable"
      ? `Colossus-Desktop-${tag}-aarch64-apple-darwin.app.tar.gz`
      : `Colossus-Desktop-DEVELOPER-PREVIEW-${tag}-aarch64-apple-darwin.app.tar.gz`;
  writeFileSync(join(directory, macName), "mac");
  writeFileSync(join(directory, `${macName}.sig`), "signed-mac");
  if (channel === "developer_preview") {
    const windowsName = `Colossus-Desktop-UNSIGNED-DEVELOPER-PREVIEW-${tag}-x86_64-pc-windows-msvc-setup.exe`;
    writeFileSync(join(directory, windowsName), "windows");
    writeFileSync(join(directory, `${windowsName}.sig`), "signed-windows");
  }
  return { channel, directory, tag, version };
}

test("stable update metadata exposes only the stable macOS target", () => {
  const manifest = buildDesktopUpdateManifest({
    ...fixture("stable"),
    repository: "obscuritylabs/Colossus",
  });
  assert.equal(manifest.channel, "stable");
  assert.deepEqual(Object.keys(manifest.platforms), ["macos-aarch64-stable"]);
});

test("preview update metadata cannot cross into stable targets", () => {
  const manifest = buildDesktopUpdateManifest({
    ...fixture("developer_preview"),
    repository: "obscuritylabs/Colossus",
  });
  assert.deepEqual(Object.keys(manifest.platforms).sort(), [
    "macos-aarch64-developer_preview",
    "windows-x86_64-developer_preview",
  ]);
  assert.match(
    manifest.platforms["windows-x86_64-developer_preview"].url,
    /UNSIGNED-DEVELOPER-PREVIEW/u,
  );
});

test("metadata generation rejects a mismatched release tag", () => {
  assert.throws(
    () =>
      buildDesktopUpdateManifest({
        ...fixture("stable"),
        version: "1.2.4",
        repository: "obscuritylabs/Colossus",
      }),
    /same canonical release/u,
  );
});
