// This validates actual TLS receipts from our unique server; mock/fixture TLS
// clients do not become Chromium evidence merely by producing the same receipt.
export function verifyPkiReceipts(metadata, report) {
  if (
    report.schema_version !== 1 ||
    report.fixture_id !== metadata.fixture_id ||
    report.overflow !== false ||
    !Array.isArray(report.events) ||
    report.events.length > 512
  )
    throw new Error(
      "PKI fixture evidence was absent, mismatched, or exceeded bounds",
    );
  const event = (endpoint, path, status, fingerprint = null) =>
    report.events.some(
      (value) =>
        value.endpoint === endpoint &&
        value.path === path &&
        value.status === status &&
        value.client_fingerprint_sha256 === fingerprint,
    );
  if (!event("public", "/public", 200))
    throw new Error("Chromium did not request the trusted private CA fixture");
  for (const endpoint of ["untrusted", "wrong_hostname", "expired"]) {
    if (
      !event(endpoint, "tls_denied", 0) ||
      report.events.some(
        (value) => value.endpoint === endpoint && value.status > 0,
      )
    )
      throw new Error(`Chromium did not enforce the ${endpoint} TLS fixture`);
  }
  if (
    !event("mtls", "/client-check", 200, metadata.fingerprints_sha256.client) ||
    !event(
      "mtls",
      "/client-check",
      403,
      metadata.fingerprints_sha256.alternate_client,
    ) ||
    !event("mtls", "tls_denied", 0)
  )
    throw new Error(
      "The server did not observe exact accepted/rejected native client identities",
    );
  if (
    !event("public", "/redirect-denied", 302) ||
    !event("redirect_mtls", "tls_denied", 0) ||
    report.events.some(
      (value) =>
        value.endpoint === "redirect_mtls" &&
        (value.status > 0 || value.client_fingerprint_sha256 !== null),
    )
  )
    throw new Error("A client identity followed an unreviewed redirect origin");
  if (
    report.events.some(
      (value) =>
        value.endpoint === "mtls" &&
        value.status === 200 &&
        value.client_fingerprint_sha256 !== metadata.fingerprints_sha256.client,
    )
  )
    throw new Error("The mTLS fixture accepted a different client identity");
  return true;
}
