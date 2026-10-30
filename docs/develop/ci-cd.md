---
title: Tiered CI/CD
description: Cost-bounded pull-request, pre-merge, and release validation for Colossus.
audience: developer
type: how-to
---

# Tiered CI/CD

Routine PR updates run change-selected checks. Request full acceptance once the final
head is reviewed and ready to merge; release tags run the platform and packaging matrix.
See [Contributing](contributing.md) for the local development and review sequence and
[Source setup](setup-testing.md) for toolchain prerequisites.

| Tier | Trigger | Coverage | Required gate |
| --- | --- | --- | --- |
| PR validation | Open, edit, reopen, synchronize, or mark ready | Selected Linux formatting, lint, unit, SDK, Desktop, documentation and dependency jobs | `Colossus PR gate` |
| Pre-merge acceptance | A repository writer applies `ci:full` to the reviewed final head | Representative macOS/Windows, Linux integration, native Desktop/VS Code, fuzzing, supply chain, PostgreSQL, OCI, OPA and mTLS | `Colossus pre-merge gate` |
| Release | Annotated stable or approved prerelease tag | Six CLI targets, signed Windows packages, Control Plane/VSIX artifacts, stable SDK or macOS preview | `Colossus release gate` |

Every hosted job needs a timeout. Routine PR jobs use standard public Linux runners.
Complete Linux integration, OCI/OPA acceptance, release readiness and x86_64 Linux
release jobs use `ubuntu-24.04`. Windows Desktop acceptance, x86_64 CLI packaging and
signed Desktop releases use `windows-2025`. Stable SDK builds and publication retain
`ubuntu-latest-m`; manual unsigned Windows Desktop validation retains
`windows-latest-l`. Those paths still require the corresponding paid runner allocation.
ARM64 and macOS jobs keep their native runners; short control, service-backed and
bounded single-process fuzzing jobs retain their existing standard or slim runners.
Provisioned runner names belong in `.github/actionlint.yaml`.

## Rust build caches

GitHub scopes PR-written caches to that PR's merge ref, so another PR cannot restore
them. `Warm Rust build caches` writes dependency build archives on `main` after Rust
manifests, lockfiles, toolchains or cache workflows change. Writers may dispatch it
manually; it uses standard Linux/macOS runners and is not a merge gate. Each cache
family shares an effective namespace between its producer and consumers. The pinned
[`rust-cache` implementation](https://github.com/Swatinem/rust-cache/blob/c19371144df3bb44fab255c43d04cbc2ab54d1c4/src/config.ts#L70-L96)
ignores `key` when `shared-key` is set. When changing a family's warm-up recipe,
version its `shared-key`, as in `macos-desktop-bundle-recipe-v2`, and update the
producer and every consumer together. GitHub cannot replace an existing entry.

Linux PR lint/unit jobs share a dependency archive. macOS Desktop acceptance and bundle
jobs use separate debug/release archives; unsigned macOS releases consume the same
release archive as the warmer. Match runner, target directory, recipe and compiler
environment, including the absence of `RUSTC_WRAPPER`. Each `rust-cache` target is
relative to its workspace (`apps/desktop/src-tauri -> target`). These jobs do not save
duplicate PR/tag archives. The cache retains dependency artifacts, not workspace crates
or application binaries; misses build normally.

The macOS bundle warmer builds the credential-free unsigned package using the same
final Tauri configuration as acceptance and release. Pre-merge packaging invokes the
canonical packager once; it does not first build a second Tauri configuration.

Other configured PR/pre-merge lanes use GitHub `sccache` read-only, or R2 read-only when
credentials are available. This covers the SDK/Desktop PR jobs and the five existing
pre-merge `sccache` consumers. Forks and unconfigured repositories keep the GitHub
fallback. `Warm R2 compiler cache` writes from `main` on Linux, macOS and Windows without
replacing the dependency archives. It runs after relevant dependency, toolchain and
cache-recipe changes reach `main`, and writers may dispatch it manually. Its protected
writer environment and main-only guard remain required; PRs cannot populate this cache.

All compiler-cache actions select the same explicit sccache binary version, separately
from the action's pinned installer revision. A new binary can change cache keys even
when Rust is unchanged. Update producers and consumers together, refill from trusted
`main`, and verify hits against the current inputs. The Windows warmer uses the same
verified `clang-cl` compiler selection as Windows Desktop acceptance.

Validated tagged CLI builds and stable SDK candidates use R2 read/write when enabled;
the stable SDK publisher reads R2. Manual release validation and signed Windows Desktop
retain GitHub `sccache`. macOS acceptance, bundles and unsigned releases retain their
credential-free archive path and do not receive R2 or signing credentials.

To enable R2:

1. Keep the bucket private. Set repository secrets `SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`
   (`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`, without a bucket path) and
   `SCCACHE_REGION` (`auto`).
2. Store an **Object Read only** token's S3 keys as `SCCACHE_R2_READ_ACCESS_KEY_ID` and
   `SCCACHE_R2_READ_SECRET_ACCESS_KEY`. Same-repository PRs receive these secrets, so
   they must grant neither writes nor access to another bucket. Forks receive none.
3. Restrict `v*` tag creation to administrators through the release tag ruleset. Create
   `sccache-r2-write`, allowing protected `main` and `v*` tags, with an **Object Read &
   Write** token stored as `SCCACHE_R2_WRITE_ACCESS_KEY_ID` and
   `SCCACHE_R2_WRITE_SECRET_ACCESS_KEY`. Only the main warmer and validated tagged builds
   use these write credentials.
4. Set repository variable `SCCACHE_R2_ENABLED=true`, then dispatch `Warm R2 compiler
   cache` on `main`. Incomplete credentials, endpoint or region fail the job.

Refill R2 manually after initial enablement or when automatic warming missed an input
change. Existing objects remain reusable while compiler inputs and cache-key formats
match. Inspect hits, misses, errors and
writes in the warmer and job summaries, then compare total duration and runner usage;
a hit alone does not prove a shorter critical path. Three-platform warm-ups increase
object storage and request usage.

Inspect or refill dependency archives with:

```bash
gh cache list --key v0-rust --ref refs/heads/main
gh workflow run cache-warm.yml --ref main
```

## Pull-request validation

The path classifier fails closed. Documentation-only changes build the site and skip
Rust. Code, configuration, CI, renamed unknown paths and unknown new files select Linux
formatting/structure, Clippy including fuzz harnesses, and workspace library tests.
API/SDK changes add pinned Node/Python/Go generation, compatibility and packaging checks.
Desktop, launcher and Rust SDK paths add managed-sidecar checks and renderer audit,
tests and build. Changes confined to `native/computer/` independently select Rust and
Desktop validation. The Desktop PR gate formats, lints and tests the portable native
bridge on Linux; it does not capture a desktop or send input.

Dependency manifests and lockfiles add license, source, ban and advisory policy for the
root sidecar and independent Desktop, browser and native-computer graphs. Tantivy and
Ratatui use patched `lru` 0.18.2 without an advisory exception. Any new exception must
name the exact advisory in every reporting scanner, explain unreachability and define removal.

The exact `RUSTSEC-2026-0387` exception covers only OpenID Connect's remaining public
P-256/P-384 verification stack. Application certificate signing uses patched arithmetic,
and wrapper bans reject direct use of the legacy APIs. The reachability review and
removal condition are recorded in [ADR 0006](adr/0006-cloud-control-plane.md#consequences).

Classification, Conventional Commit title validation and aggregate decisions execute
contracts from the **PR base revision**, never their proposed replacements. The initial
bootstrap fallback selects every tier and requires success. If an older classifier lacks
SDK/Desktop outputs, the workflow selects both, preventing CI-changing PRs from skipping
validation by weakening their own contracts.

Selected jobs run concurrently after classification with independent toolchains and
caches. Workflows retain trusted-base decisions and runner provisioning; `cargo xtask`
owns portable recipes. The PR aggregate runs the trusted seven-argument selector three
times across formatting, lint, unit, SDK, Desktop, documentation and dependency results.
A skipped job is accepted only when classification explicitly marked it unnecessary.
Documentation PR builds remain separate from deployment of `main` by the Documentation
workflow. The PR tier allocates no macOS or Windows runners.

## Request pre-merge acceptance

Keep full acceptance at the end of development:

1. Finish focused iteration and the local completion gate below. Wait for
   `Colossus PR gate` on the current head and resolve merge conflicts.
2. Inspect and resolve every human and automated review conversation, addressing
   actionable findings in code and tests. Mark a draft PR ready for review.
3. As a repository writer, request acceptance for that final head:

   ```bash
   gh pr edit PR_NUMBER --add-label ci:full
   ```

A cheap Linux eligibility job checks writer permission, non-draft status, the exact head
and its successful PR gate before allocating acceptance runners. Current policy requires
**all** pre-merge acceptance and packaging jobs, including for small or documentation-only
PRs; routine path selection does not implement a lighter pre-merge tier. Shared CI and
toolchain changes can affect every consumer and need broad evidence.

Linux's complete Rust/native sandbox suite runs on Ubuntu 24.04 with an exact-path
AppArmor profile granting the temporary root-owned CLI Linux user-namespace authority.
Three macOS jobs run concurrently: native sandbox/sidecar acceptance, Desktop acceptance
and Desktop packaging. The native job exercises real sidecar bootstrap, pinned gRPC,
guardian lifecycle and sandboxing. Desktop acceptance lints/tests the standalone bridge
and checks Chromium keyboard behavior, accessibility, high contrast, drawers, approvals
and 880×640 layout. Packaging independently builds release sidecar/CLI/Tauri binaries
and verifies bundle structure. The Desktop jobs have separate build trees and transfer
no build artifacts to each other. Acceptance allows 90 minutes for cold builds and
the later native command-approval compilation. Packaging retains its 75-minute budget.

macOS native-debug and standalone Tauri graphs remain separate on bounded disks. Bundle
verification covers the pruned lockfile, ad-hoc two-phase signing, outer seal and final
nested-binary hashes. Ad-hoc `ADHOC` team identity proves structure and intentionally
refuses Managed Local startup. Distributable builds require the expected 10-character
Apple Team ID, Developer ID, notarization and exact app/sidecar/CLI code identifiers.

Windows runtime and Desktop jobs also run concurrently. The standard Windows 2025
runtime lane covers renderer typechecking/tests, platform contracts, worker, AppContainer
sandbox and computer protocol tests; the computer check lints all native targets/features
and builds acceptance examples. GitHub-hosted Windows runners are
[administrators](https://docs.github.com/en/actions/reference/runners/github-hosted-runners#administrative-privileges),
so this lane cannot establish ordinary-user capture/input acceptance. Computer-control
changes also need maintainer [interactive receipts from the final clean commit](testing.md#windows-computer-control-acceptance).
The Windows 2025 Desktop job prepares binaries and exercises the bridge, credentials,
WebView2, plugins and approvals. Desktop checks require staged binaries. Each job reports
all independent failures; the aggregate waits for both. Linux Desktop, credential-vault,
VS Code lifecycle, fuzzing, supply-chain and live-service lanes also remain mandatory.
Portable formatting belongs to the PR tier and is not repeated on platform runners.

The Windows Desktop job allows 120 minutes for cold compilation and the complete
acceptance sequence on a standard runner. Its CEF step retains a separate
45-minute limit, and every acceptance result remains required.

The Windows runtime lane allows 75 minutes for cold compilation and every required
acceptance command. Its final outcome aggregation remains mandatory. The release
smoke configuration gives verified inventory operations a bounded 30-second effect
budget for cold disk I/O; its echo provider retains a separate five-second budget.
The Windows fixture reports inventory elapsed time on success or failure and keeps
native errors fatal. These test budgets do not change production defaults or the
fixture's filesystem, executable, network, process, memory or custody restrictions.

The Windows Desktop pre-merge job resolves the runner's
[`clang-cl`](https://clang.llvm.org/docs/UsersManual.html#clang-cl) executable and checks
its version before the first Cargo build, then sets that exact path as `CC` and `CXX`
for this job. This avoids MSVC compiler stdout interleaving with Cargo instructions
during inkjet's parallel parser builds while retaining compiler diagnostics. The Rust
MSVC target and native CEF Visual Studio toolset remain unchanged. This CI selection
does not change local or release compiler configuration.

Do not push another commit during acceptance. `synchronize` cancels the old run and
removes `ci:full`; the old result cannot authorize a new head. After any fix, wait for the
new PR gate, resolve review and reapply the label. The pre-merge sentinel runs on every
workflow event: synchronization or another label leaves a failing gate without allocating
acceptance runners. Only successful `ci:full` acceptance on the current head replaces it;
failed, cancelled or unexpectedly skipped work cannot satisfy protection.

The ruleset does not require a fresh commit solely because `main` advanced. If newer
main changes affect the same behavior or integration contract, update the branch and
rerun both gates; a conflict-free merge alone does not prove the combination was tested.
If classification is wrong or empty, repair its contract rather than forcing skipped
checks through the aggregate.

## Release validation and publication

The six CLI platform/architecture builds belong to the release tier, separate from PR
acceptance. Annotated `vX.Y.Z` or approved `vX.Y.Z-preview.N` tags must point to `main`
and satisfy the source-version/changelog rules in
[release operations](releasing.md#prepare-a-stable-version). Each target runs security
acceptance, a locked release build, archive/checksum creation, clean installation,
offline echo/audit and signed-bundle smoke. The exact source generates changelog and
draft notes retained in `release-history`; see [changelog generation](releasing.md#generate-the-changelog-and-release-notes).

Stable releases additionally generate/test/package TypeScript, Python and Go SDKs,
bind candidate metadata and checksums to the source commit, and require signed Windows
x64 Desktop and x64/ARM64 CLI archives. SDK compatibility uses the latest reachable
stable tag, or the release commit's parent for the first release, so rerunning an old tag
does not treat newer APIs as removals. Stable releases need no Apple or Tauri updater keys.

### Developer Preview channel

Developer Previews retain all six CLI targets and add ad-hoc signed macOS ARM and signed
Windows x64 Desktop previews without stable SDK publication. macOS uses the
`developer_preview` channel, `COLOSSUS_DESKTOP_TEAM_ID=ADHOC` and signing identity `-`,
with no Apple signing/notarization secrets. It remains unnotarized; packaging verifies
code identifiers, channel-bound manifest and nested hashes. The native banner identifies
the preview when **Show security warnings** is enabled (off by default).
See [preview installation, source-version exceptions and browser edition](releasing.md#developer-previews-and-desktop).

The macOS archive is `Colossus-Desktop-DEVELOPER-PREVIEW-vX.Y.Z-preview.N-aarch64-apple-darwin.zip`
with an adjacent `.sha256`. Its draft is a prerelease titled
**Colossus vX.Y.Z-preview.N - Developer Preview (Unnotarized)**.

### Drafts and manual validation

Manual validation does not publish or receive signing authority:

```bash
gh workflow run release.yml --ref BRANCH -f version=vX.Y.Z
```

Stable dispatches validate SDK candidates and skip Desktop; preview dispatches use the
non-runnable `validation_only` channel. Failures block publication of partial artifacts;
repair the source and create a new annotated tag.

Only the release draft job has `contents: write`. A human reviews and publishes the
draft after its selected gate passes. The protected stable SDK publisher receives
short-lived OIDC/tag authority after `sdk-production` approval and publishes only bytes
matching the immutable candidate; see [create and approve the release](releasing.md#create-and-approve-the-release).
Registry publication, production Desktop authority and macOS signing/notarization remain
separately protected.

### Desktop update authority

Automatic updates require separately authorized stable builds, repository variable
`DESKTOP_UPDATE_PUBLIC_KEY` (one-line base64), protected secret
`DESKTOP_UPDATE_PRIVATE_KEY` and, when applicable, `DESKTOP_UPDATE_PRIVATE_KEY_PASSWORD`.
Windows stable and preview channels use manual updates until that authority and feed are
configured. Validation builds never produce updater artifacts.

macOS `.app.tar.gz` packaging requires nested/outer signing, notarization, stapling and
bundle verification. Stable draft metadata binds its signature to an immutable release
URL. `desktop-update-channels.yml` runs only when a published release contains an
independently produced `stable.json`; it validates tag, release kind, platform, HTTPS URL
and immutable path before replacing that asset on `desktop-update-channels`. Ordinary
stable core and preview releases omit it. Update clients use the shared additional-CA
configuration and reject HTTPS-to-HTTP redirects.

Update signatures are distinct from platform publisher identity. Windows signing hashes
the signed CLI/sidecar into the manifest before Tauri patches and signs the app, support
DLLs, uninstaller and installer. Installed Authenticode publisher/timestamp verification
precedes upload; both CLI architectures are signed on x64 before ZIP hashing.
See [Windows signing authority](releasing.md#windows-artifact-signing-authority).

### Server, extension and documentation artifacts

The coordinated gate also requires native Linux x64/ARM64 Control Plane server/web and
offline container bundles and six platform-specific VSIX packages. After release
publication, `control-plane-image.yml` publishes exact tested candidates and verifies the
two-platform index; conflicting immutable tags fail.
See [Control Plane and VS Code release operations](releasing.md#control-plane-containers-and-vs-code-packages).

`documentation-candidate.yml` builds and HTTP-smokes the public documentation image on
native Linux amd64/ARM64. Documentation/container PRs exercise the same smoke. Annotated
tag candidates on `main` retain tested offline images and source-bound manifests for
30 days without registry write credentials. `documentation-image.yml` uses one resolved
protected-main publisher revision, requires successful exact-tag release/candidate runs,
verifies retained archives/source identities and refuses conflicting version tags before
registry writes. It verifies the two-platform executable index and adds no CLI assets.
See [recovery and anonymous verification](releasing.md#documentation-container).

## Repository enforcement

The tracked ruleset starts in evaluation mode; an administrator uses the audited helper
to create `ci:full` and evaluate it before activating protection:

```bash
./scripts/ci/configure-repository.sh plan OWNER/REPOSITORY
./scripts/ci/configure-repository.sh evaluate OWNER/REPOSITORY
```

Exercise documentation and code PRs plus one `ci:full` run. Confirm both gate names,
stale-label removal, resolved conversations and billing, then activate:

```bash
./scripts/ci/configure-repository.sh activate OWNER/REPOSITORY
```

The main ruleset requires a conflict-free PR, resolved conversations and both gates on
the current head, with zero mandatory approvals, no bypass actors and no latest-main
requirement. It blocks direct pushes, deletion and non-fast-forward updates. Merge
queues are outside this topology; the ruleset uses the current-head PR and pre-merge
gates directly.

## Local development and completion

Iterate with focused package checks/tests from [Source setup](setup-testing.md). Run the
change-selected completion gate when the final change is ready for handoff or a PR:

```bash
cargo xtask pr --base origin/main
```

When Rust is selected, this includes the complete Rust gate; do not repeat
`cargo xtask check rust` on unchanged source. Use that standalone command for Rust-only
completion without a PR. Hosted cost tiers do not replace local completion. Release
operators additionally run `./release/verify-release-readiness.sh`.

## Control Plane Helm validation and publication

`control-plane-chart.yml` owns `cargo xtask check helm` for affected pull requests and
release tags. The static gate covers Kubernetes 1.34 rendering, routing variants,
OpenShift security contexts, authentication, external database trust, version parity,
and packaging with vendored dependencies. It does not install or upgrade a cluster.
Successful annotated tag builds retain the exact chart candidate for thirty days.
`control-plane-chart-publish.yml` publishes those bytes to GHCR after reviewed
release publication using protected-main code, a dedicated registry environment,
and a job-scoped `GITHUB_TOKEN` with `packages: write`.
See [Helm release operations](releasing.md#control-plane-helm-chart).
