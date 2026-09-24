---
title: Toolchain inventory
description: Current toolchain declarations, consumers, verification and supported development environments.
audience: developer
type: reference
---

# Toolchain inventory

This reference maps the current toolchain declarations to their consumers and checks.
Exact pins remain in the named files; this page is not an executable version manifest.
The optional mise bootstrap mirrors existing pins. CI and devcontainer setup still
use their own declarations.

## Provisioning and check owners

| Surface | Current declarations and consumers | Verification and check owner |
| --- | --- | --- |
| Stable Rust | `rust-toolchain.toml` owns channel, minimal profile, Clippy and rustfmt. Workflows and devcontainer repeat these values; mise reads the file. | rustup distribution verification; locked Cargo graphs; `cargo xtask check rust`. Host environment overrides must be checked explicitly. |
| Fuzz Rust and cargo-fuzz | `premerge.yml` owns explicit nightly selection, utility version and fuzz invocation. Not installed by the mise bootstrap. | Explicit `cargo +nightly-…`, `cargo install --locked`; bounded hosted fuzz run. |
| Node, Python and Go | `mise.toml` mirrors workflow setup pins and `.devcontainer/Dockerfile` image versions. SDK, Desktop and release jobs consume subsets. | Existing setup actions are SHA-pinned; container images are digest-pinned. Backend locks/verification parity are not yet established for mise. `xtask check sdk` and `desktop` own behavior checks. |
| npm | Release SDK preparation and `publish-sdk.yml` explicitly install npm; other consumers use the Node distribution's npm. | SDK publication checks its required minimum; npm lockfiles own package bytes. The publisher minimum is distinct from the provisioned version. |
| actionlint | `mise.toml`, the PR workflow's download block and devcontainer Go build currently repeat the version. | PR download checks SHA-256; devcontainer builds a versioned Go module. `cargo xtask check workflows` runs contract tests and actionlint. |
| cargo-deny and cargo-audit | `mise.toml`, devcontainer Cargo installs and workflow utility installers repeat versions. | Devcontainer uses `cargo install --locked`; workflow installer actions have SHA pins. `cargo xtask check dependencies` owns policy and exceptions. |
| mise itself | Optional contributor-installed executable; not repository-pinned. | Backend/platform locks are not supplied by the repository; tool resolution alone does not prove installation reproducibility. |
| SDK generators | `sdk/package.json`/lock own Buf; TypeScript manifest/lock own ts-proto; Python requirements own grpcio-tools/protobuf; `sdk/scripts/install-codegen-tools` owns Go generator pins. | `sdk/scripts/generate` checks installed generator versions; input/output digests and `xtask check sdk` detect generated drift. Python requirements pin versions but currently have no hash requirement. |
| Rust protobuf compiler | Root Cargo manifest/lock own `protoc-bin-vendored`. | Cargo integrity/locked resolution; Rust API build and tests. |
| Language linters and tests | Desktop/SDK npm manifests and locks, Python development requirements, and Go module files own tools. | Desktop TypeScript/Vitest/Prettier checks; SDK Ruff/mypy/Python tests, Go tests/vet/gofmt and TypeScript checks. |
| Browser acceptance | Desktop npm lock owns Playwright; `test:browser:install` installs its browser payloads in premerge. | Native browser acceptance jobs and Desktop tests. |
| Documentation | `scripts/docs-site` owns the Zensical image version/digest; docs jobs and developers invoke it through Docker. | Digest-pinned build, strict site checks and Rust documentation examples via `cargo xtask check docs`. |
| ORAS and OPA | `release-image.yml` owns ORAS downloads; `premerge.yml` owns OPA download. | Explicit checksums; OCI roundtrip and live OPA acceptance. |
| Containers and devcontainer features | Workflow service/acceptance images, devcontainer Docker stages and `devcontainer-lock.json` own immutable references. `oci-proxy.Dockerfile` uses `scratch`. | Preserve image digests and feature lock; live PostgreSQL, Chroma, OCI and OPA suites. |
| Native build tools and container engines | Runner images and OS package installation supply Clang, CMake, pkg-config, Tauri libraries, Docker/Podman and platform packaging/signing tools. | Native CI/release acceptance; these are not supplied by `mise install`. |
| Compiler caches and Actions | Workflow SHA pins own sccache/setup/cache actions; `scripts/cargo-sccache` is an optional local wrapper. | Plain Cargo must work without sccache; preserve cold/warm jobs and cache behavior. |
| Git hooks and task dispatch | `.githooks/` and `scripts/install-git-hooks.sh` currently own hooks; `cargo xtask` owns check selection/execution. | Commit-message and pre-commit checks remain independent of hosted gates. |

## Change selection contract

Root mise configuration variants, lockfiles, `.mise/`, `mise-tasks/`, shared
`.github/actions/`, `.devcontainer/` and Rust toolchain files select Rust, SDK,
Desktop, documentation and dependency checks. Shared setup can affect all consumers;
the classifier deliberately errs toward exercising them. `CLAUDE.md` selects docs
alongside its canonical `AGENTS.md`. Individual path tests cover these selectors in
`scripts/ci/test-contracts.sh`; unrelated changes retain their existing classification.

The PR workflow loads classification from its trusted base revision. Changes to the
classifier must land before subsequent PRs rely on new selectors. This inventory does
not enforce equality between duplicated version declarations.

## Development environments

| Environment | Requirements |
| --- | --- |
| macOS linked worktree | Native build dependencies, configured tools and a relative guide symlink resolving to `AGENTS.md`. |
| Linux contributor/devcontainer | Clang/Tauri dependencies, required Rust components, and Docker for the pinned documentation build. The devcontainer additionally includes editor components. |
| Windows linked worktree | Git symlink support, Git Bash for shell scripts, configured tools and native build dependencies. A text file containing the link target is not an equivalent guide. |
| Premerge and release runners | Representative Linux/macOS/Windows premerge tiers; six release targets: x64/ARM64 macOS, Linux musl and Windows MSVC. See [Tiered CI/CD](ci-cd.md). |
| Offline contributor setup | Already-installed tools, package caches, native libraries and container images. `mise install` needs network access on an empty machine; the runtime's offline support is separate. |
