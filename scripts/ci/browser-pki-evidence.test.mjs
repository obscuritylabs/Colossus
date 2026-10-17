import assert from "node:assert/strict";
import test from "node:test";

import { verifyPkiReceipts } from "../../apps/desktop/scripts/browser-pki-evidence.mjs";

function evidence() {
  const metadata = {
    fixture_id: "unique-fixture",
    fingerprints_sha256: {
      client: "a".repeat(64),
      alternate_client: "b".repeat(64),
    },
  };
  const event = (endpoint, path, status, client_fingerprint_sha256 = null) => ({
    endpoint,
    path,
    status,
    client_fingerprint_sha256,
  });
  const report = {
    schema_version: 1,
    fixture_id: metadata.fixture_id,
    overflow: false,
    events: [
      event("public", "/public", 200),
      event("untrusted", "tls_denied", 0),
      event("wrong_hostname", "tls_denied", 0),
      event("expired", "tls_denied", 0),
      event("mtls", "/client-check", 200, metadata.fingerprints_sha256.client),
      event(
        "mtls",
        "/client-check",
        403,
        metadata.fingerprints_sha256.alternate_client,
      ),
      event("mtls", "tls_denied", 0),
      event("public", "/redirect-denied", 302),
      event("redirect_mtls", "tls_denied", 0),
    ],
  };
  return { metadata, report, event };
}

test("native PKI receipt check requires server-observed exact identity and every negative case", () => {
  const { metadata, report } = evidence();
  assert.equal(verifyPkiReceipts(metadata, report), true);
  for (let index = 0; index < report.events.length; index += 1)
    assert.throws(() =>
      verifyPkiReceipts(metadata, {
        ...report,
        events: report.events.filter((_, position) => position !== index),
      }),
    );
});

test("successful invalid server trust cannot be hidden by a later TLS-denied event", () => {
  const { metadata, report, event } = evidence();
  for (const endpoint of ["untrusted", "wrong_hostname", "expired"])
    assert.throws(() =>
      verifyPkiReceipts(metadata, {
        ...report,
        events: [...report.events, event(endpoint, "/public", 200)],
      }),
    );
});

test("redirected or unexpected client identities invalidate native PKI receipts", () => {
  const { metadata, report, event } = evidence();
  for (const additional of [
    event(
      "redirect_mtls",
      "/client-check",
      200,
      metadata.fingerprints_sha256.client,
    ),
    event(
      "redirect_mtls",
      "tls_denied",
      0,
      metadata.fingerprints_sha256.client,
    ),
    event(
      "mtls",
      "/client-check",
      200,
      metadata.fingerprints_sha256.alternate_client,
    ),
  ])
    assert.throws(() =>
      verifyPkiReceipts(metadata, {
        ...report,
        events: [...report.events, additional],
      }),
    );
});

test("lost, overflowing, or mismatched fixture evidence is rejected", () => {
  const { metadata, report } = evidence();
  for (const changes of [
    { overflow: true },
    { fixture_id: "other" },
    { schema_version: 2 },
    { events: [] },
    { events: Array(513).fill(report.events[0]) },
  ])
    assert.throws(() => verifyPkiReceipts(metadata, { ...report, ...changes }));
});
