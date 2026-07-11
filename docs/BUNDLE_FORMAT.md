# Offline Bundle Format

Offline bundles are directory-based artifacts that Colossus verifies without network
access before use. The Rust verifier requires a strict `manifest.json`, a complete
payload allowlist, SHA-256 checksums, and at least one trusted Ed25519 signature.

## Required layout

```text
bundle/
  manifest.json
  wheelhouse/
    colossus-0.1.0-py3-none-any.whl
  skills/
    example-skill/
      manifest.json
      SKILL.md
  sbom/
    sbom.spdx.json
```

Every regular payload file must be listed in `manifest.files`. Undeclared files,
symlinks, special filesystem entries, and non-normalized paths are rejected.

## Manifest schema

```json
{
  "format_version": 1,
  "name": "colossus-offline-bundle",
  "version": "0.1.0",
  "publisher": "colossus",
  "created_at": "2026-06-08T00:00:00Z",
  "files": [
    {
      "path": "wheelhouse/colossus-0.1.0-py3-none-any.whl",
      "sha256": "64-character lowercase hex digest",
      "size": 1234
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

Verifier requirements:

- `manifest.json` must exist at the bundle root.
- `files` must be a list.
- Each entry must be an object with string `path` and `sha256` fields.
- Each listed path must point to a regular file under the bundle directory.
- The SHA-256 digest of each file must match the manifest entry.
- Every present size must match exactly.
- Every payload file must be declared; symlinks and traversal are rejected.
- At least one signature must resolve to an exact trusted publisher/key binding, and
  every present signature must verify.

Recommended metadata:

- `format_version`: bundle manifest format version.
- `name`: human-readable bundle name.
- `version`: Colossus or release version represented by the bundle.
- `created_at`: UTC timestamp.
- `source_revision`: Git commit or signed source provenance.
- `files`: include SBOM, release notes, lock material, and any detached artifact
  signatures as normal hash-listed payloads.
- `signatures`: embedded Ed25519 signatures over compact UTF-8 JSON after strict
  deserialization, default materialization, recursive lexicographic object-key sorting,
  and clearing this array. Array order remains significant.

## Verification

```bash
cargo run --offline -q --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- \
  bundle verify ./bundle
```

Verification fails if the manifest is missing or malformed, a file is missing,
undeclared, oversized, linked, outside the bundle, or mismatched, or a signature is
missing, unknown, malformed, or invalid. The retained evidence includes manifest hash,
trusted key id, source revision, file count, and total verified bytes.

## Release expectations

Production bundles should include:

- Rust executables for each represented target plus source needed for compliance.
- `rust/Cargo.lock` and any vendored crate source required for airgapped rebuilds.
- SBOM output for the package and bundled dependencies.
- Embedded manifest signatures and hash-listed detached release-artifact signatures.
- Skill manifests and skill content intended for the isolated environment.
- A copy of the release notes and security policy.
