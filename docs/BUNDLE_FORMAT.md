# Offline Bundle Format

Offline bundles are directory-based artifacts that Colossus can verify before use.
The current verifier requires a `manifest.json` file and SHA-256 checksums for every
listed file.

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
  signatures/
    manifest.json.sig
```

Only `manifest.json` and the files listed in `manifest.files` are enforced by the
current verifier. Additional directories are recommended release content for airgapped
operation.

## Manifest schema

```json
{
  "format_version": 1,
  "name": "colossus-offline-bundle",
  "version": "0.1.0",
  "created_at": "2026-06-08T00:00:00Z",
  "files": [
    {
      "path": "wheelhouse/colossus-0.1.0-py3-none-any.whl",
      "sha256": "64-character lowercase hex digest"
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

Recommended metadata:

- `format_version`: bundle manifest format version.
- `name`: human-readable bundle name.
- `version`: Colossus or release version represented by the bundle.
- `created_at`: UTC timestamp.
- `source_revision`: Git commit or signed source provenance.
- `sbom`: relative path to SBOM material.
- `signatures`: relative paths to detached signatures.

## Verification

```bash
uv run colossus bundle verify ./bundle
```

Verification fails if the manifest is missing, malformed, references a missing file, or
contains a checksum mismatch.

## Release expectations

Production bundles should include:

- Wheels and source distributions for Colossus.
- A complete dependency wheelhouse for the target platform.
- `uv.lock` or equivalent lock material.
- SBOM output for the package and bundled dependencies.
- Detached signatures for the manifest and release artifacts.
- Skill manifests and skill content intended for the isolated environment.
- A copy of the release notes and security policy.
