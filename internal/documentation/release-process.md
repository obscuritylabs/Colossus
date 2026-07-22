---
status: current
replacement:
  - /develop/ci-cd/
  - /develop/setup-testing/
  - https://github.com/obscuritylabs/Colossus/blob/main/CHANGELOG.md
---

# Release Process

Colossus releases are built from reviewed commits on `main`. Automation validates and
packages a release, but creates only a draft GitHub Release; publication remains a human
decision.

## Prepare the release

1. Update the workspace version, `CHANGELOG.md`, user documentation, security notes, and
   compatibility guidance together.
2. Confirm `Cargo.lock` and `fuzz/Cargo.lock` contain only reviewed dependency changes.
3. Resolve all human and automated review conversations, pass `Colossus PR gate`, apply
   `ci:full`, and pass `Colossus pre-merge gate` on the final PR head.
4. Merge through the protected `main` branch without bypassing the ruleset.
5. From a clean checkout of the resulting `main` commit, run:

    ```bash
    ./release/verify-release-readiness.sh
    ```

The verifier requires the pinned Rust, `cargo-deny`, and `cargo-audit` versions. It rejects
a reintroduced root Python runtime or tracked Python source outside the maintained public
SDK, and runs formatting, Clippy, the complete workspace suite, fuzz compilation, and
production/fuzz supply-chain policy.

## Dry-run artifacts

Before tagging, the release operator may exercise all six package jobs without publishing:

```bash
gh workflow run release.yml --ref main -f version=vX.Y.Z
```

Manual dispatch is artifact-only. It cannot create or update a GitHub Release. Inspect
the `Colossus release gate` result and download all six CLI archives plus sidecars and the
validation-only Desktop archive plus checksum before continuing.

## Tag and validate

Create an annotated semantic-version tag on the reviewed `main` commit:

```bash
git tag -a vX.Y.Z -m "Colossus vX.Y.Z"
git push origin vX.Y.Z
```

The Release workflow rejects lightweight tags, tags outside `main`, workspace/changelog
version mismatches, failed readiness verification, and incomplete artifact sets. It runs:

- macOS x64 and ARM64 native sandbox acceptance and packaging;
- static Linux-musl x64 and ARM64 native sandbox acceptance and packaging;
- Windows x64 and ARM64 named-pipe, AppContainer, worker, and packaging acceptance.
- macOS ARM64 standalone Desktop packaging followed by isolated identity validation,
  signing, notarization, stapling, and assessment for tag-triggered releases.

Each job builds with `--locked --release`, produces one archive and SHA-256 sidecar,
installs into a clean prefix, verifies offline echo and audit behavior, and exercises a
signed bundle. Linux jobs also prove static linkage and package the AppArmor installer.

## Review and publish the draft

Only after `Colossus release gate` succeeds does the final job receive `contents: write`.
It verifies exactly fourteen files—six CLI archives, six checksum sidecars, the signed
Desktop archive, and its checksum—then creates or idempotently updates a draft release.
It never publishes automatically.

Before publishing the draft:

1. Confirm all six CLI targets, the signed Desktop archive, and every checksum sidecar are
   present.
2. Test installation on a clean representative host where practical.
3. Review generated notes, the changelog excerpt, known limitations, and security notes.
4. Attach any required SBOM or independently generated signature material.
5. Confirm the signed bundle publisher matches
   [`release/bundle-publisher.json`](https://github.com/obscuritylabs/Colossus/blob/main/release/bundle-publisher.json).

Never publish the disposable signing material used by CI smoke tests. Preserve the
workflow run, gate status, secure-anchor/audit evidence, and artifact hashes with the
release record. The frozen Python tag and branches are not rebuilt or republished as part
of a Rust release.
