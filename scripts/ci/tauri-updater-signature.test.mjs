import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";

import { verifyTauriUpdaterSignature } from "../verify-tauri-updater-signature.mjs";

const PUBLIC_KEY = Buffer.from(
  "untrusted comment: minisign public key E7620F1842B4E81F\n" +
    "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
).toString("base64");
const SIGNATURE = Buffer.from(
  "untrusted comment: signature from minisign secret key\n" +
    "RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n" +
    "trusted comment: timestamp:1555779966\tfile:test\n" +
    "QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==",
).toString("base64");

function fixture(content = "test") {
  const directory = mkdtempSync(join(tmpdir(), "colossus-updater-signature-"));
  const file = join(directory, "package");
  const signatureFile = join(directory, "package.sig");
  writeFileSync(file, content);
  writeFileSync(signatureFile, SIGNATURE);
  return { file, signatureFile, encodedPublicKey: PUBLIC_KEY };
}

test("verifies a known Tauri-compatible minisign fixture", () => {
  assert.doesNotThrow(() => verifyTauriUpdaterSignature(fixture()));
});

test("rejects modified package bytes and malformed wrappers", () => {
  assert.throws(
    () => verifyTauriUpdaterSignature(fixture("tampered")),
    /primary updater signature is invalid/u,
  );
  const malformed = fixture();
  writeFileSync(malformed.signatureFile, "not-base64");
  assert.throws(
    () => verifyTauriUpdaterSignature(malformed),
    /signature is not bounded canonical base64/u,
  );
});
