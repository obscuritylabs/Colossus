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
| Fuzz Rust and cargo-fuzz | `premerge.yml` owns explicit nightly selection, utility version and fuzz invocation. Not installed by the mise bootstrap. | Explicit `cargo +nightly-â€¦`, `cargo install --locked`; bounded hosted fuzz run. |
| Node, Python and Go | `mise.toml` mirrors workflow setup pins and `.devcontainer/Dockerfile` image versions. SDK, Desktop and release jobs consume subsets. | Existing setup actions are SHA-pinned; container images are digest-pinned. Backend locks/verification parity are not yet established for mise. `xtask check sdk` and `desktop` own behavior checks. |
| npm | Release SDK preparation and `publish-sdk.yml` explicitly install npm; other consumers use the Node distribution's npm. | SDK publication checks its required minimum; npm lockfiles own package bytes. The publisher minimum is distinct from the provisioned version. |
| actionlint | `mise.toml`, the PR workflow's download block and devcontainer Go build currently repeat the version. | PR download checks SHA-256; devcontainer builds a versioned Go module. `cargo xtask check workflows` runs contract tests and actionlint. |
| cargo-deny and cargo-audit | `mise.toml`, devcontainer Cargo installs and workflow utility installers repeat versions. | Devcontainer uses `cargo install --locked`; workflow installer actions have SHA pins. `cargo xtask check dependencies` owns policy and exceptions. |
| mise itself | Optional contributor-installed executable; not repository-pinned. | Backend/platform locks are not supplied by the repository; tool resolution alone does not prove installation reproducibility. |
| SDK generators | `sdk/package.json`/lock own Buf; TypeScript manifest/lock own ts-proto; Python requirements own grpcio-tools/protobuf; `sdk/scripts/install-codegen-tools` owns Go generator pins. | `sdk/scripts/generate` checks installed generator versions; input/output digests and `xtask check sdk` detect generated drift. Python requirements pin versions but currently have no hash requirement. |
| Rust protobuf compiler | Root Cargo manifest/lock own `protoc-bin-vendored`. | Cargo integrity/locked resolution; Rust API build and tests. |
| Language linters and tests | Desktop/SDK npm manifests and locks, Python development requirements, and Go module files own tools. | Desktop TypeScript/Vitest/Prettier checks; SDK Ruff/mypy/Python tests, Go tests/vet/gofmt and TypeScript checks. |
| Desktop capture and shortcuts | Desktop pins `tauri-plugin-global-shortcut` on Windows; `native/computer` owns Windows Graphics Capture and bounded accessibility helpers. Both Cargo locks retain their native dependency graphs. | Explicit ordinary-user App Shot acceptance verifies the synthetic target and helper cleanup; renderer tests cover preview, discard and draft attachment. Portable builds do not prove capture. |
| Managed worktrees | `colossus-worktrees` and Desktop use vendored libgit2 without network transport or hooks; Cargo manifests and locks own the version. | Service and CLI process tests cover ownership, leases, tracked links and removal guards; Desktop browser tests cover settings and opening. |
| Browser acceptance | Desktop npm lock owns Playwright; `test:browser:install` installs its browser payloads in premerge. | Native browser acceptance jobs and Desktop tests. |
| Helm | `scripts/ci/install-helm.sh` pins Helm 3.19.0 and its Linux amd64 checksum; contributors supply Helm through PATH or `HELM`. Vendored PostgreSQL/common charts are pinned by `Chart.lock` and archive SHA-256. | `cargo xtask check helm`: static lint, render matrix, server configuration validation and offline packaging; no cluster install or upgrade tests. |
| Local Helm deployment | `scripts/helm-local.ps1` and `scripts/helm-local/` own the complete Windows Docker Desktop profile, pinned kind/Gateway/dependency images, private allocation, and process-scoped clients. PowerShell 7.2+/.NET handles certificates and key material; Node is required for Hosted API and connector checks. | `pwsh ./scripts/helm-local/test-contract.ps1` checks offline ownership/profile/cleanup behavior on Windows. `up` and `test` verify an actual owned deployment; these live checks are separate from the static Helm gate. See [local deployment](local-helm.md). |
| Compose deployment | `deploy/compose/` owns PowerShell 7.2+ one-time initialization, Docker Compose 2.24+ configuration and private material. A pinned Node setup container provisions the first Chat project; ordinary operation uses Compose directly. | `test-init.ps1` checks defaults, TLS and retained secrets on Windows/Linux. `compose-smoke.yml` tests HTTPS, login, Echo/SSE and retained Chat after restart against published preview.5 images, with direct Compose diagnostics. Node 22+ is needed on the test host only for `smoke.cjs`. See [Compose operation](../cloud/deployment/compose.md). |
| Documentation | `scripts/docs-site` owns the Zensical image version/digest; `deploy/documentation/Dockerfile` mirrors that generator and pins the Nginx static runtime. Docs jobs and developers invoke Docker. | Digest-pinned build, strict site checks and Rust documentation examples via `cargo xtask check docs`; on-prem image HTTP checks use `deploy/documentation/smoke.mjs`. |
| ORAS and OPA | `release-image.yml` owns ORAS downloads; `premerge.yml` owns OPA download. | Explicit checksums; OCI roundtrip and live OPA acceptance. |
| Containers and devcontainer features | Workflow service/acceptance images, devcontainer Docker stages and `devcontainer-lock.json` own immutable references. `oci-proxy.Dockerfile` uses `scratch`. | Preserve image digests and feature lock; live PostgreSQL, OCI and OPA suites. |
| Native build tools and container engines | Runner images and OS package installation supply Clang, CMake, pkg-config, Tauri libraries, Docker/Podman and platform packaging/signing tools. | Native CI/release acceptance; these are not supplied by `mise install`. |
| Compiler caches and Actions | Workflow SHA pins own sccache/setup/cache actions; matching explicit sccache binary versions keep cache writers and consumers compatible. `scripts/cargo-sccache` optionally selects the local compiler wrapper while preserving inherited settings. | Plain Cargo must work without sccache. Refill trusted-main caches after cache-key changes. Keep local incremental defaults; compare measured editing performance before changing them. See [local cache guidance](setup-testing.md#optional-local-compiler-cache). |
| Git hooks and task dispatch | `.githooks/` and `scripts/install-git-hooks.sh` own hooks; dependency-free `cargo xtask` owns focused checks, gate selection, execution and timings. | `quick` requires package selection; `pr` includes the complete Rust gate when selected. Commit-message and pre-commit checks remain independent of hosted gates. |

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
