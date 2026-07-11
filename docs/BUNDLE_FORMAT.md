# Offline Bundle Format

An offline bundle is a directory whose complete payload is hash-allowlisted by a strict
signed `manifest.json`. Verification uses no network and requires at least one Ed25519
signature bound to an explicitly trusted publisher/key pair.

## Example Layout

```text
bundle/
  manifest.json
  artifacts/
    colossus-0.6.0-alpha.1-aarch64-apple-darwin.tar.gz
    colossus-0.6.0-alpha.1-aarch64-apple-darwin.tar.gz.sha256
  sbom/
    colossus.spdx.json
  policy/
    production-bundle.tar.gz
  workflows/
    release.yaml
```

Every regular payload file must appear in `files`. Undeclared files, missing files,
symlinks, special entries, absolute/non-normalized paths, traversal, size mismatch, and
hash mismatch fail verification.

## Manifest

```json
{
  "format_version": 1,
  "name": "colossus-offline",
  "version": "0.6.0-alpha.1",
  "publisher": "colossus",
  "created_at": "2026-07-11T00:00:00Z",
  "source_revision": "GIT_COMMIT",
  "files": [
    {
      "path": "artifacts/colossus-0.6.0-alpha.1-aarch64-apple-darwin.tar.gz",
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

Version 1 denies unknown fields. Signatures cover compact UTF-8 JSON after strict
deserialization, default materialization, recursive lexicographic object-key sorting,
and replacement of `signatures` with an empty array. Array order remains significant.

Every present signature must use Ed25519, resolve to the exact publisher/key binding,
and verify. An unknown or malformed additional signature fails even when another
signature is valid.

## Establish Trust And Verify

Trust is an audited, approval-required local lifecycle:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  packs trust add colossus --public-key BASE64_ED25519_PUBLIC_KEY
colossus --config .colossus/config.yaml packs trust list
colossus --config .colossus/config.yaml bundle verify ./bundle
```

Verification returns bounded evidence: bundle name/version, canonical manifest hash,
file count, total verified bytes, trusted key ID, and optional source revision. It does
not install or execute payloads.

## Production Contents

A production bundle should hash-list:

- native archive(s), installers, and SHA-256 sidecars for represented targets;
- SBOM and detached artifact-signature material;
- exact source revision, license, release notes, security policy, and Rust lockfiles;
- any reviewed OPA bundles, workflows, skills, packs, MCP servers, local model assets,
  or Chroma dependencies needed inside the airgap;
- vendored crate source only when reproducible source rebuilding inside the airgap is an
  explicit requirement.

Bundle verification authenticates the payload set. Each native archive still goes
through its clean-prefix installer smoke and the installed runtime still performs
`audit verify` and secure-anchor checks.
