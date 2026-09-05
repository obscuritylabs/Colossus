---
title: Bundle format
description: Signed offline bundle layout, manifest canonicalization, verification, and installation rules.
audience: developer
type: reference
---

# Bundle format

An offline bundle is a directory whose complete regular-file payload is hash-allowlisted
by a strict signed `manifest.json`. Verification uses no network and requires an
Ed25519 signature bound to an explicitly trusted publisher and public key.

## Layout

```text
bundle/
  manifest.json
  artifacts/
    aarch64-apple-darwin/colossus
    x86_64-apple-darwin/colossus
    aarch64-unknown-linux-musl/colossus
    x86_64-unknown-linux-musl/colossus
    aarch64-pc-windows-msvc/colossus.exe
    x86_64-pc-windows-msvc/colossus.exe
  sbom/colossus.spdx.json
  policy/production-bundle.tar.gz
  workflows/release.yaml
```

Every regular payload file appears in `files`. Missing, undeclared, linked, special,
absolute, non-normalized, traversing, wrong-sized, or hash-mismatched entries fail
verification.

## Manifest

```json
{
  "format_version": 1,
  "name": "colossus-offline",
  "version": "RELEASE",
  "publisher": "colossus",
  "created_at": "UTC_RFC3339_TIMESTAMP",
  "source_revision": "GIT_COMMIT",
  "files": [
    {
      "path": "artifacts/aarch64-apple-darwin/colossus",
      "sha256": "64-character-lowercase-hex-digest",
      "size": 12345678
    }
  ],
  "signatures": [
    {
      "algorithm": "ed25519",
      "key_id": "sha256-of-raw-public-key",
      "signature": "base64-signature"
    }
  ]
}
```

Format `1` denies unknown fields. Signatures cover compact UTF-8 JSON after:

1. strict deserialization and default materialization;
2. recursive lexicographic object-key sorting;
3. replacement of `signatures` with an empty array.

Array order remains significant. Every present signature uses Ed25519, resolves to the
exact publisher/key binding, and verifies. An unknown or malformed additional signature
fails even if another signature is valid.

The official release publisher identity is recorded in
[`release/bundle-publisher.json`](https://github.com/obscuritylabs/Colossus/blob/main/release/bundle-publisher.json).
Compare it with the copy attached to the release before adding trust. The key ID is the
SHA-256 digest of the decoded public key.

## Verification

```bash
colossus --approval-mode ask bundle key-info \
  --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED
colossus bundle verify ./bundle
```

Bind the returned key ID and public key in strict configuration before verification:

```yaml
bundles:
  trustedPublishers:
    colossus:
      SHA256_KEY_ID: BASE64_ED25519_PUBLIC_KEY
```

Verification returns bounded bundle identity, canonical manifest hash, file count, total
bytes, trusted key ID, and optional source revision. It does not install or execute
payloads.

## Deterministic construction

```bash
colossus --approval-mode ask bundle build \
  ./bundle-stage ./bundle \
  --name colossus-offline \
  --version RELEASE \
  --publisher colossus \
  --created-at UTC_RFC3339_TIMESTAMP \
  --source-revision GIT_COMMIT \
  --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED
```

The destination must not exist. Construction resolves the signing seed only after permit
issuance, copies a link-free bounded tree, hashes copied bytes, writes deterministic
manifest order, signs, re-verifies, and atomically publishes.

## Installation

```bash
colossus --approval-mode ask bundle install \
  ./bundle --prefix "$HOME/.local"
```

Installation re-verifies the complete bundle, selects only the running platform's exact
artifact, checks the copied hash, and atomically creates `bin/colossus` or
`bin/colossus.exe`. Prefix and target must be link-free and authorized. Existing targets
fail closed; installation is clean-prefix and no-clobber.
