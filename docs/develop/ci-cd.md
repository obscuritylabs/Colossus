---
title: Tiered CI/CD
description: Cost-bounded pull-request, pre-merge, and release validation for Colossus.
audience: developer
type: how-to
---

# Tiered CI/CD

## Goal

Keep routine pull-request feedback cheap while preserving deliberate multi-platform and
release acceptance at the points where that evidence is needed.

## Prerequisites

- A pull request in the Colossus repository.
- Repository write permission to request `ci:full`, or administrator permission to
  bootstrap the ruleset.
- The local toolchain described in [Source setup and test tiers](setup-testing.md).

Colossus separates fast pull-request feedback, deliberate pre-merge acceptance, and
complete release validation. Expensive hosted runners are allocated only after a
repository writer requests them for a reviewed commit or an annotated release tag is
pushed.

<div class="diagram-scroll diagram-scroll--wide" markdown tabindex="0" role="region" aria-label="Tiered CI and release flow diagram">

```mermaid
flowchart LR
    PR["Pull request update"] --> C["Fail-closed path classifier"]
    C -->|"documentation only"| D["Documentation build"]
    C -->|"code, CI, dependency, or unknown"| L["Linux workspace validation"]
    C -->|"API or SDK"| SDK["SDK generation + language packages"]
    C -->|"desktop renderer or bridge"| UI["Desktop renderer validation"]
    SDK --> L
    UI --> L
    C -->|"dependency files"| S["Supply-chain policy"]
    D --> PG["Colossus PR gate"]
    L --> PG
    S --> PG
    PG --> R["Resolve human and automated review"]
    R --> F["Writer applies ci:full"]
    F --> E["Draft, actor, and current-head eligibility"]
    E --> A["macOS ARM + Windows x64 + live security"]
    A --> MG["Colossus pre-merge gate"]
    MG --> M["Merge to main"]
    M --> T["Annotated stable or approved prerelease tag"]
    T --> V["Release readiness + six native targets"]
    V --> RG["Colossus release gate"]
    RG --> DR["Draft GitHub Release for human approval"]
```

</div>

## Tiers and cost ceilings

| Tier | Trigger | Hosted coverage | Stable gate | Planning ceiling |
|---|---|---|---|---:|
| PR validation | Open, edit, reopen, synchronize, or mark ready | Linux and selected documentation/dependency jobs | `Colossus PR gate` | $0.15 per update |
| Pre-merge acceptance | Apply `ci:full` | macOS 15 ARM, Windows 2025 x64, bounded fuzzing, supply chain, Chroma, PostgreSQL, OCI, OPA, and mTLS | `Colossus pre-merge gate` | $0.75 per final run |
| Release | Push an annotated stable or approved prerelease tag | Six CLI targets, channel-specific Apple-silicon Desktop, and Windows x64 Developer Preview for prereleases | `Colossus release gate` | $4.50 per release |

These ceilings are planning targets based on hosted-runner rates and observed durations,
not billing or runtime enforcement. A job timeout remains mandatory for every hosted job.
Workflow jobs run on Blacksmith runners: `blacksmith-4vcpu-ubuntu-2404` for every x64
Linux lane, `blacksmith-6vcpu-macos-15` for Apple-silicon lanes, and
`blacksmith-8vcpu-windows-2025` for the Windows lanes. Blacksmith is roughly twice as
fast per minute of wall clock at about half the hosted per-minute rate, so the four-core
Linux tier now covers both the short control jobs and the longest CPU-bound lanes without
raising the ceilings above. Three lanes stay on the GitHub-hosted `ubuntu-latest-m`
larger runner: complete Rust PR validation and release readiness, because both install an
exact-path AppArmor profile and the Blacksmith microVM kernel does not expose the AppArmor
LSM, plus the x86_64 Linux release artifact, because the release matrix pins one runner
per published target. The repository's `.github/actionlint.yaml` registers every custom
runner label so local workflow linting recognizes them.

## Steps

## Pull-request validation

The classifier fails closed. Documentation-only paths build the documentation site and
skip Rust. Code, configuration, build, release, CI, renamed unknown paths, and unknown
new paths run the complete Linux Rust gate. API and SDK paths additionally select SDK
generation, compatibility, language tests, and release-package checks inside that job.
Desktop application, launcher, and Rust SDK paths select renderer checks there and the
native Tauri acceptance described below. Rust, npm, Go, and Python dependency manifests
and lockfiles also run license, source, ban, and advisory policy, including the standalone
desktop Cargo graph.

Pull-request classification, title validation, and aggregate gate decisions execute
the contract checked out from the PR base revision, never the proposed replacement
from the PR head. While these contracts first land, their one-time bootstrap fallback
selects every PR tier and requires every result to succeed. If a trusted base classifier
predates the SDK or desktop outputs, the workflow appends both selections as `true` so an
old base cannot silently skip either component. This prevents a CI-changing PR from
suppressing validation by weakening its own classifier or gate scripts.

The Rust job combines Conventional Commit validation, exact AppArmor installation, and
the repository-owned `cargo xtask` component checks. The Rust component covers
formatting, crate-root structure, locked metadata, Clippy, the complete workspace suite,
and fuzz-harness linting. When selected, the SDK component installs pinned Node.js,
Python, and Go toolchains for reproducible generation and packaging, while the Desktop
component checks the standalone native bridge formatting, installs the renderer
lockfile, and audits, tests, and builds the renderer. Desktop selection also exercises
the managed-sidecar protocol and host crates. The workflow retains trusted-base
classification and runner provisioning, but does not duplicate portable check recipes
or allocate macOS or Windows runners.
The aggregate gate accepts a skipped job only when the classifier explicitly marked
that job unnecessary; the stable seven-argument gate contract remains unchanged.

Documentation deployment is separate: pull requests build documentation in PR
validation, while `main` changes are deployed by the Documentation workflow.

## Request pre-merge acceptance

Apply `ci:full` only after the PR is ready to merge:

1. Make the branch current with `main` and wait for `Colossus PR gate` on the current
   PR merge commit.
2. Resolve every human and automated review conversation and address actionable findings
   in code and tests.
3. Mark the PR ready for review if it is still a draft.
4. As a repository writer, apply the label:

    ```bash
    gh pr edit PR_NUMBER --add-label ci:full
    ```

Eligibility is checked on a cheap Linux runner before macOS or Windows is allocated. It
rejects draft PRs, actors below write permission, and a missing or failed current-head PR
gate. The required pre-merge gate fails on failed, cancelled, or unexpectedly skipped
acceptance work. Separate macOS jobs keep the root native-debug graph from coexisting
with the standalone Tauri graph on the runner's bounded disk. The desktop job lints and
tests the standalone native bridge; runs pinned Chromium keyboard,
accessibility, high-contrast, drawer, approval, and 880×640 layout acceptance; deletes
its debug artifacts; then builds the bundled sidecar, CLI, and Tauri application into one
shared non-incremental release tree. The native job exercises the otherwise-ignored real sidecar
bootstrap/pinned-gRPC/guardian lifecycle and sandbox acceptance. Together they prove the
pruned locked build, then create an ad-hoc signed two-phase app bundle and verify the outer
seal plus final nested-binary manifest hashes. Ad-hoc signing
uses the explicit `ADHOC` team sentinel, tests structure only, and produces a runtime that
intentionally refuses to start Managed Local. Distributable builds embed the expected
10-character Apple Team ID, use Developer ID and notarization, and verify exact code
identifiers for the app, sidecar, and CLI.
The Windows lane runs renderer typechecking, renderer tests, and platform-sensitive
Desktop contract tests before installing Rust or starting native compilation. Those
independent checks continue into Windows native, worker, sandbox, binary preparation,
Desktop Clippy, and Desktop library-test acceptance, then report every failed outcome
together before the lane fails. Desktop checks run only when their required binaries were staged.
Portable formatting remains owned by the PR tier instead of being repeated on platform
runners.
Supply-chain acceptance audits both the root sidecar graph and the desktop's independent
lockfile.

Do not push a new commit while acceptance is running. A `synchronize` event cancels the
old run and removes `ci:full`; the old result cannot authorize the new head. After the new
PR gate passes, resolve any new review and apply the label again.

The `Colossus pre-merge gate` sentinel runs on every pre-merge workflow event. A new
commit or a label event other than `ci:full` therefore leaves a failing gate without
allocating the acceptance runners. Only a successful `ci:full` run on the current head
replaces that sentinel result; a skipped gate can never satisfy the ruleset.

## Failure path

- If classification is wrong or empty, fix the classifier or path contract; do not force
  a skipped gate through the aggregate job.
- If pre-merge eligibility fails, verify draft status, actor permission, branch currency,
  and the PR gate on the current merge commit before relabeling.
- If an acceptance job fails, diagnose that job, push the fix, wait for the new PR gate,
  and reapply `ci:full`.
- If a release target fails, do not publish partial artifacts. Fix the source and create a
  new annotated tag according to the release policy.

## Release flow

A release tag must be annotated, match either `vX.Y.Z` or `vX.Y.Z-preview.N` with
`N > 0`, point to a commit contained in `main`, and match both the workspace version and
changelog heading. Tag pushes run local
release-readiness verification and exactly six native CLI targets. Each CLI target
combines its security acceptance, locked release build, archive and checksum generation,
clean installation, offline echo/audit, and signed-bundle smoke. A credential-free macOS
15 ARM job builds the standalone Tauri graph into one shared Cargo target and uploads an
exact ditto archive of the unsigned application. A separate fresh macOS runner downloads
that archive before it imports Developer ID and notarization authority; this minimal job
runs no Rust, Cargo, Vite, or Tauri build step. It installs locked Node dependencies with
lifecycle scripts disabled only for updater signing and verification. It signs both
bundled executables, writes their final manifest, binds that exact manifest digest into
the already-built main executable, signs the desktop application, submits it to Apple notarization,
staples and assesses it, and uploads an Apple-silicon direct-download zip plus checksum.
Approved prerelease tags additionally build an x64 per-user NSIS package on the
`blacksmith-8vcpu-windows-2025` runner, run silent
install/first-launch/uninstall and process-cleanup smoke
checks, and publish its checksum, sealed manifest, and provenance. That package is
explicitly labeled unsigned and preview-only. Stable release gating requires the Windows
preview job to be skipped; it cannot accidentally promote an unsigned Windows package.
The stable Windows job remains absent until Authenticode signing authority is configured
for the nested CLI and sidecar, the app, and the installer.
The validated workspace release version and public Apple Team ID are injected during the
unsigned build, then the signing runner checks the imported identity against that Team ID,
so application metadata, code identity, and asset tag cannot drift.
For a stable tag, the build receives the Team ID from the non-secret `MACOS_TEAM_ID`
repository variable; the signing runner requires a matching protected `MACOS_TEAM_ID`
secret alongside the Developer ID and notary credentials.

```bash
git tag -a vX.Y.Z -m "Colossus vX.Y.Z"
git push origin vX.Y.Z
```

### Developer Preview channel

`vX.Y.Z-preview.N` is the only credential-free tag path that may produce a runnable
Desktop; `v0.10.1-preview.2` is the most recent example. It still runs all six CLI release
jobs. Its Desktop build uses the `developer_preview` channel,
`COLOSSUS_DESKTOP_TEAM_ID=ADHOC`, and the ad-hoc identity `-`; it never reads Apple signing
or notarization secrets. Packaging still verifies strict code signatures, fixed code
identifiers, the channel-bound sealed manifest, and the exact hashes of the bundled
sidecar and CLI.

The resulting Desktop archive is runnable for testing but is not Apple-notarized and its
ad-hoc signature does not establish publisher identity. The workflow names it
`Colossus-Desktop-DEVELOPER-PREVIEW-vX.Y.Z-preview.N-aarch64-apple-darwin.zip`, includes
an adjacent SHA-256 sidecar, sets GitHub prerelease metadata, and labels the draft
**Colossus vX.Y.Z-preview.N - Developer Preview (Unnotarized)**. The native compile-time
channel also drives a persistent in-app warning. Stable tags continue to require the
canonical Team ID, Developer ID signature, notarization, stapling, and Gatekeeper
assessment.

Within `release.yml`, only the final publication job receives `contents: write`. After
all six CLI targets and the Desktop artifact pass, automation creates or updates a draft
GitHub Release. A human reviews the draft and publishes it; the approved Developer
Preview draft is already marked as a GitHub prerelease. The stable-only
post-publication channel workflow has narrow `contents: write` authority only to replace
the fixed signed update metadata asset. Manual dispatch is artifact-only and cannot
create a release; it uses
the separate `validation_only` channel with an ad-hoc signature and `ADHOC` sentinel,
never reads production signing secrets, and does not produce a runnable Desktop
application. Its Actions artifact and archive are explicitly named
`validation-only-adhoc` / `VALIDATION-ONLY-ADHOC` so they cannot be mistaken for a
Developer Preview or stable signed release:

```bash
gh workflow run release.yml --ref BRANCH -f version=vX.Y.Z
```

### Desktop update signing and channels

Only stable builds advertise automatic updates. They require the repository variable
`DESKTOP_UPDATE_PUBLIC_KEY`, containing the one-line base64 Tauri updater public key,
plus the protected `DESKTOP_UPDATE_PRIVATE_KEY` secret and, when applicable,
`DESKTOP_UPDATE_PRIVATE_KEY_PASSWORD`. Unsigned Developer Preview and validation-only
builds reject update endpoints and keys and never produce updater artifacts; users
install later previews manually from GitHub Releases.

macOS packages the stable signed `.app.tar.gz` only after nested signing, outer signing,
notarization, stapling, and final bundle verification. The stable versioned draft
includes channel-scoped metadata whose platform entry carries that signature and an
immutable version-release URL.

When a human publishes a stable versioned release,
`desktop-update-channels.yml` revalidates the tag, release kind, platform key, HTTPS URL,
and immutable release path before replacing `stable.json` on the fixed
`desktop-update-channels` release. The native update client also uses the shared
additional-CA configuration and rejects HTTPS-to-HTTP redirects. Published Developer
Previews skip this workflow because they deliberately contain no update authority.

The application update signature is separate from platform publisher identity.
Authenticode remains mandatory before a Windows package can enter the stable channel;
the existing stable Windows release job remains absent and the release gate requires
the unsigned Windows job to be skipped for stable tags.

## Expected result

Routine PR updates allocate only selected Linux/documentation jobs, one deliberate final
run provides representative pre-merge evidence, and release tags alone allocate all six
CLI architecture jobs plus channel-specific Apple-silicon Desktop packaging. Stable
Desktop releases are notarized; the one explicit Developer Preview remains visibly
unnotarized. Each tier has one stable fail-closed aggregate check.

## Bootstrap repository enforcement

The tracked ruleset starts in evaluation mode. After this change is merged, a repository
administrator uses the audited helper to create `ci:full` and apply the ruleset:

```bash
./scripts/ci/configure-repository.sh plan OWNER/REPOSITORY
./scripts/ci/configure-repository.sh evaluate OWNER/REPOSITORY
```

Exercise a documentation PR and a code PR, then run `ci:full` once. Confirm both stable
gate names, stale-label removal, resolved-conversation enforcement, and billing entries.
Only then activate protection:

```bash
./scripts/ci/configure-repository.sh activate OWNER/REPOSITORY
```

The `main` ruleset requires a pull request with zero mandatory approvals, resolved review
conversations, an up-to-date branch, and both Colossus gates. It permits no bypass actors
and blocks direct pushes, deletion, and non-fast-forward updates. GitHub merge queues are
not part of this topology because they are unavailable for this private Team repository.

## Verification

Run the change-selected local PR gate:

```bash
cargo xtask pr --base origin/main
```

## Local completion versus hosted tiers

Hosted tiering reduces repeated platform spending; it does not weaken the local completion
contract. Before handoff, run the focused tests needed while iterating, `cargo xtask
check rust`, and the change-selected `cargo xtask pr` gate described in
[Source setup and test tiers](setup-testing.md). Release operators additionally run
`./release/verify-release-readiness.sh`.

## Next step

For a normal contribution, resolve review and follow
[Request pre-merge acceptance](#request-pre-merge-acceptance). For repository rollout,
follow [Bootstrap repository enforcement](#bootstrap-repository-enforcement) without
skipping the evaluation run.
