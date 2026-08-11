---
title: Core release operations
description: Bootstrap, validate, publish, and recover coordinated CLI and SDK releases.
audience: developer
type: how-to
---

# Core release operations

## Goal

Publish one stable Colossus core version as six GitHub CLI archives, two reviewed
bootstrap installers, `@obscuritylabs/colossus-sdk` on npm,
`obscuritylabs-colossus-sdk` on PyPI,
and `sdk/go/vX.Y.Z` from the same immutable source commit. Stable core releases do not
contain Desktop artifacts and do not require Apple, Tauri updater, or Authenticode
credentials.

## Prerequisites

Complete these account-owned steps before publishing the first stable draft:

1. Create the GitHub environment `sdk-production`. Require an operator review, prevent
   self-review where the organization supports it, restrict deployment to protected
   release tags, and keep environment secrets empty. The publisher uses GitHub OIDC,
   not stored npm or PyPI credentials.
2. Confirm Actions may create tags with the workflow `GITHUB_TOKEN`. The publisher grants
   `contents: write` and `id-token: write` only to its protected publication job.
3. Confirm control of the npm `@obscuritylabs` scope. npm trusted publishers are
   configured from an existing package's settings; npm does not offer PyPI-style pending
   publishers. If `@obscuritylabs/colossus-sdk` does not yet exist, reserve it once with
   an intentionally non-release bootstrap version using an interactive maintainer login,
   then configure its trusted publisher before publishing a real Colossus version.
4. In the npm package's **Trusted Publisher** settings, configure:

   - organization or user: `obscuritylabs`
   - repository: `Colossus`
   - workflow filename: `publish-sdk.yml`
   - environment: `sdk-production`
   - allowed action: `npm publish`

   After a trusted publication succeeds, disallow token-based publication and revoke
   any bootstrap automation token. The public repository allows npm to attach a
   provenance statement to each trusted OIDC publication; keep `--provenance` enabled.
5. The normalized PyPI name `colossus-sdk` belongs to an unrelated project. Create a
   **pending trusted publisher** for the unclaimed project
   `obscuritylabs-colossus-sdk` with owner `obscuritylabs`, repository `Colossus`,
   workflow `publish-sdk.yml`, and environment `sdk-production`. The installed import
   remains `colossus_sdk`.

Publisher configuration fields are case-sensitive. A missing or mismatched publisher
fails at the registry without exposing a reusable credential.

## Steps

### Prepare a stable version

The following identities must all be the same stable `X.Y.Z` value:

- `[workspace.package].version` and all exact internal dependency versions;
- the npm package and lockfile;
- the Python distribution;
- TypeScript, Python, and Go SDK user-agent versions;
- the `CHANGELOG.md` heading; and
- the requested `vX.Y.Z` tag.

Release SDK compatibility is pinned to the most recent earlier stable `vX.Y.Z`
tag reachable from the release commit. It never uses a moving branch as the
compatibility baseline. Package builds use the release commit timestamp as
`SOURCE_DATE_EPOCH` and the same pinned Node, npm, Python, Go, and Rust toolchain so the
protected publisher can reproduce every candidate byte. The release packager also
normalizes the Python source archive's order, ownership, permissions, and timestamps;
setuptools does not apply `SOURCE_DATE_EPOCH` to all sdist metadata itself.

All internal Rust packages must retain `publish = false`. Regenerate the SDK input
digest after changing package metadata, then run the completion gates:

```bash
./sdk/scripts/install-codegen-tools
./sdk/scripts/generate
cargo xtask check rust
cargo xtask pr --base origin/main
```

Validate the hosted stable path from the release branch before merging. Manual dispatch
cannot create a GitHub Release or publish a registry package:

```bash
gh workflow run release.yml --ref BRANCH -f version=vX.Y.Z
```

For a stable target this proves release readiness, all six native CLI jobs, SDK
generation and tests, package construction, intrinsic package metadata, the candidate
manifest, and checksums. All Desktop jobs must be skipped. Download the
`colossus-sdk-release` Actions artifact if manual package inspection is needed.

### Create and approve the release

After the reviewed version commit is on `main`, create an annotated tag:

```bash
git tag -a vX.Y.Z -m "Colossus vX.Y.Z"
git push origin vX.Y.Z
```

The tag workflow creates a draft only after the six CLI archives and immutable SDK
candidate pass. Before publishing the draft, verify that it contains exactly:

- six CLI archives and six adjacent `.sha256` files;
- `colossus-install.sh` and `colossus-install.ps1`, each with an adjacent `.sha256`;
- one npm `.tgz`;
- one Python wheel and one source distribution;
- `colossus-sdk-vX.Y.Z-manifest.json`; and
- `colossus-sdk-vX.Y.Z-SHA256SUMS`.

Publishing the stable draft triggers `publish-sdk.yml`. Approve its one
`sdk-production` deployment. The job reverifies the exact release assets against the
`colossus-sdk-release` artifact of the successful `release.yml` run for the tag, so
release-asset write access alone cannot substitute bytes that the tag never produced;
a recomputed manifest and checksum file do not satisfy this comparison. Inside the
protected environment it independently rebuilds the SDK packages from the exact stable
tag and requires every release asset byte to match. It then reconciles npm and PyPI,
publishes only missing bytes, and finally creates the annotated `sdk/go/vX.Y.Z` tag on
the core tag's commit.

## Expected result

The stable GitHub Release contains exactly the six CLI archives, their checksums, the
two repository-owned bootstrap installers and their checksums, and the five immutable
SDK candidate files. The protected publisher releases the same version to npm and PyPI
and creates the Go module tag at the identical source commit. No stable core job
requests or produces Desktop signing material.

## Verification

```bash
gh release view vX.Y.Z
npm view @obscuritylabs/colossus-sdk@X.Y.Z version dist.tarball
python -m pip index versions obscuritylabs-colossus-sdk
go list -m github.com/obscuritylabs/colossus/sdk/go@vX.Y.Z
```

Also verify that `git rev-list -n 1 vX.Y.Z` and
`git rev-list -n 1 sdk/go/vX.Y.Z` are identical. A stable core GitHub Release must not
contain an unsigned Desktop asset. The Desktop update-channel workflow runs only for a
separately produced stable release that contains a verified `stable.json` asset.
Confirm that the public bootstrap route resolves to the newly published, byte-identical
release asset:

```bash
curl -fsSL \
  https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh \
  -o /tmp/colossus-install.sh
sh /tmp/colossus-install.sh --version vX.Y.Z --dry-run --yes
```

The `Verify public distribution` workflow also runs automatically when the stable draft
is published. It uses no repository token, compares the exact-tag and `latest` bootstrap
bytes, verifies the bootstrap sidecar, performs a clean direct install on macOS, Linux,
and Windows, validates the receipt, and runs structured update discovery. Do not treat
the release as installation-ready until all three jobs pass.

Generate the exact Homebrew formula only from the published macOS checksum sidecars:

```bash
node scripts/ci/render-homebrew-formula.mjs \
  --version X.Y.Z \
  --assets PATH_TO_RELEASE_ASSETS \
  --output colossus.rb
```

Review the generated formula, verify `brew test colossus`, and publish it to
`obscuritylabs/homebrew-tap` only after the public-distribution workflow passes. The
formula installs prebuilt upstream bytes and adds only an advisory Homebrew ownership
marker; it never writes a direct-install receipt. Update the version and four platform
hashes in `flake.nix` from the same published sidecars, refresh `flake.lock` only when
the nixpkgs input changes, and run `nix flake check` before merging the package metadata.
Package definitions in the release-preparation commit therefore continue to identify
the latest already-published stable release; never guess the next release's hashes.

After the public distribution jobs pass, update the root README and install guide only
if the final commands differ from the reviewed bootstrap contract. Confirm that the
README's `latest/download` commands, the review-before-running flow, the exact-version
flags, `colossus update`, Nix ownership, manual archive verification, and uninstall
guidance all remain represented before closing a distribution epic.

## Failure path

Registries and Git tags cannot be updated atomically. If one external system fails,
rerun the protected publisher against the already-published GitHub Release:

```bash
gh workflow run publish-sdk.yml --ref vX.Y.Z \
  -f tag=vX.Y.Z -f publish=true
```

The recovery path independently rebuilds the packages from the exact tag and refuses
publication unless every byte matches the immutable GitHub Release candidate. It
accepts an existing version only when the registry bytes match the release manifest,
publishes missing PyPI files with `skip-existing`, and accepts an existing Go tag only
when it resolves to the recorded source commit. Any conflicting immutable version or
tag fails closed; investigate it instead of changing or overwriting the release.

Recovery also requires the trusted `colossus-sdk-release` artifact for the tag. That
artifact is retained for fourteen days, so after it expires rerun the tag's `release.yml`
run before dispatching the publisher:

```bash
gh run rerun RUN_ID
```

## Next step

After the first coordinated stable release succeeds, keep the registry trusted-publisher
settings and `sdk-production` environment protected, and use this same candidate-first
flow for later stable versions.

### Developer Previews and Desktop

Annotated `vX.Y.Z-preview.N` tags retain the visibly unsigned macOS and Windows Desktop
Developer Preview path. They do not build stable SDK registry candidates and cannot
publish npm, PyPI, or Go versions. Production Desktop signing and update-channel
publication remain an independent release track.
