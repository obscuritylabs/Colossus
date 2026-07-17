# Offline Bundle Format

An offline bundle is a directory whose complete payload is hash-allowlisted by a strict
signed `manifest.json`. Verification uses no network and requires at least one Ed25519
signature bound to an explicitly trusted publisher/key pair.

## Example Layout

```text
bundle/
  manifest.json
  artifacts/
    aarch64-apple-darwin/
      colossus
    x86_64-apple-darwin/
      colossus
    aarch64-unknown-linux-musl/
      colossus
    x86_64-unknown-linux-musl/
      colossus
    aarch64-pc-windows-msvc/
      colossus.exe
    x86_64-pc-windows-msvc/
      colossus.exe
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
  "version": "0.9.0",
  "publisher": "colossus",
  "created_at": "2026-07-11T00:00:00Z",
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

Version 1 denies unknown fields. Signatures cover compact UTF-8 JSON after strict
deserialization, default materialization, recursive lexicographic object-key sorting,
and replacement of `signatures` with an empty array. Array order remains significant.

Every present signature must use Ed25519, resolve to the exact publisher/key binding,
and verify. An unknown or malformed additional signature fails even when another
signature is valid.

## Establish Trust And Verify

Derive the safe public identity from the private-seed reference, then bind it to the
publisher through an audited approval-required lifecycle:

```bash
colossus --config .colossus/config.yaml --approval-mode ask bundle key-info \
  --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED
colossus --config .colossus/config.yaml --approval-mode ask \
  packs trust add colossus --public-key BASE64_ED25519_PUBLIC_KEY
colossus --config .colossus/config.yaml packs trust list
colossus --config .colossus/config.yaml bundle verify ./bundle
```

Official Colossus release bundles use the Ed25519 publisher identity recorded in
[`release/bundle-publisher.json`](https://github.com/obscuritylabs/Colossus/blob/main/release/bundle-publisher.json). Before verifying an
official bundle, compare that file with the copy attached to the GitHub release and add
its `public_key` for publisher `colossus`. The expected `key_id` is the SHA-256 digest of
the decoded public key; Colossus derives and checks that binding when trust is added.

Verification returns bounded evidence: bundle name/version, canonical manifest hash,
file count, total verified bytes, trusted key ID, and optional source revision. It does
not install or execute payloads.

## Build A Signed Bundle

Stage one or more exact native executables under the target paths above, plus any
additional hash-listed payload such as license, SBOM, policies, workflows, or skills.
The staging directory must not contain `manifest.json`. Register the `bundle key-info`
public key, keep the 32-byte Ed25519 signing seed in an environment credential, and use
an explicit timestamp for reproducible output:

```bash
export COLOSSUS_BUNDLE_SIGNING_SEED=...
colossus --config .colossus/config.yaml --approval-mode ask bundle build \
  ./bundle-stage ./bundle \
  --name colossus-offline \
  --version 0.9.0 \
  --publisher colossus \
  --created-at 2026-07-11T00:00:00Z \
  --source-revision GIT_COMMIT \
  --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED
```

`bundle build` crosses the effect gateway, requires read and write roots, resolves the
seed only after permit issuance, rejects links/special files/oversized payloads, copies
through a destination-local temporary directory, hashes the copied bytes, writes files
in deterministic manifest order, signs canonical JSON, re-verifies against publisher
trust, and atomically publishes a previously absent destination. Equal payload, metadata,
and key inputs produce identical manifests.

## Install The Current Target

```bash
colossus --config .colossus/config.yaml --approval-mode ask bundle install \
  ./bundle --prefix "$HOME/.local"
```

Installation re-verifies the complete signed bundle, selects only the exact target for
the running OS/architecture, rechecks the artifact hash after copying, makes Unix output
executable, and atomically creates `bin/colossus` or `bin/colossus.exe`. The prefix must
fit an authorized write root. Existing targets and linked directories fail closed;
bundle installation is intentionally clean-prefix/no-clobber.

## Production Contents

A production bundle should hash-list:

- installable native executables under exact supported target paths, plus native
  archive(s), installers, and SHA-256 sidecars when archive distribution is included;
- SBOM and detached artifact-signature material;
- exact source revision, license, release notes, security policy, and Rust lockfiles;
- any reviewed OPA bundles, workflows, skills, packs, MCP servers, local model assets,
  or Chroma dependencies needed inside the airgap;
- vendored crate source only when reproducible source rebuilding inside the airgap is an
  explicit requirement.

Bundle verification authenticates the payload set. Each native archive still goes
through its clean-prefix installer smoke and the installed runtime still performs
`audit verify` and secure-anchor checks.
