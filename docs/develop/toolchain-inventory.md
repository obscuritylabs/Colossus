---
title: Toolchain inventory
description: Current toolchain declarations, consumers, verification and supported development environments.
audience: developer
type: reference
---

# Toolchain inventory

This reference maps the current toolchain declarations to their consumers and checks.
Exact pins remain in the named files; this page is not an executable version manifest.
The root `mise.toml` owns contributor, PR and premerge tool versions;
`rust-toolchain.toml` owns Rust.
PR and premerge jobs consume selected tools through `.github/actions/setup-toolchain`.
The devcontainer installs the contributor set from the same lock. Retained release
and Rust bootstrap image declarations are checked against this inventory.

## Provisioning and check owners

| Surface | Current declarations and consumers | Verification and check owner |
| --- | --- | --- |
| Stable Rust | `rust-toolchain.toml` owns channel, minimal profile, Clippy and rustfmt. Mise provisioning reads this file. The devcontainer adds editor components separately; retained release workflows and the digest-pinned Rust bootstrap image have checked values. | rustup distribution verification; locked Cargo graphs; `cargo xtask check rust`. Host environment overrides must be checked explicitly. |
| Fuzz Rust and cargo-fuzz | `mise.toml` variables own the nightly and cargo-fuzz versions; `mise.fuzz.toml` selects them only under `MISE_ENV=fuzz`, with `mise.fuzz.lock`. Neither is in the default PR/bootstrap installation. | Explicit `cargo +nightly-…`, locked Cargo source backend; explicit nightly invocation and bounded hosted fuzz run. |
| Node, Python and Go | `mise.toml` owns versions using core backends; PR/premerge jobs install subsets; the devcontainer installs all three. Retained release workflow and Linux Desktop packaging image pins must agree. | `mise.lock` records archive URLs and SHA-256 for the supported provisioning platforms. Python uses python-build-standalone archives. Retained setup actions and image digests remain pinned. `xtask check sdk` and `desktop` own behavior checks. |
| npm | `mise.toml` owns the release npm pin; release SDK preparation and `publish-sdk.yml` retain checked explicit installation; other consumers use the Node distribution's npm. | SDK publication checks its required minimum; npm lockfiles own package bytes. The publisher minimum is distinct from the provisioned version. |
| actionlint | `mise.toml` selects the explicit aqua backend; PR provisioning consumes the lock. Premerge workflow jobs and the devcontainer consume the same lock. | The locked Linux SHA-256 preserves the previous download checksum; aqua also verifies upstream provenance. Container archives retain checksum/provenance verification. `cargo xtask check workflows` runs contract tests and actionlint. |
| cargo-deny and cargo-audit | `mise.toml` selects explicit aqua backends and verified upstream archives for Linux/Windows PRs and premerge jobs. The Bookworm container selects a locked cargo-audit source build through `mise.devcontainer.toml`. Retained release installers have checked versions. | Archive locks preserve verification; macOS and Bookworm cargo-audit retain locked native source compilation. Retained workflow installer actions have SHA pins. `cargo xtask check dependencies` owns policy and exceptions. |
| mise itself | `scripts/ci/mise-bootstrap.sh` owns the exact release and Linux x64/ARM64, macOS ARM64 and Windows x64 executable checksums; the shared action is SHA-pinned. | The action and container verify the executable before provisioning; unsupported platforms fail before download. Locked installation requires recorded URLs/checksums; rustup remains responsible for compiler verification. |
| SDK generators | `sdk/package.json`/lock own Buf; TypeScript manifest/lock own ts-proto; Python requirements own grpcio-tools/protobuf; `sdk/scripts/install-codegen-tools` owns Go generator pins. | `sdk/scripts/generate` checks installed generator versions; input/output digests and `xtask check sdk` detect generated drift. Python requirements pin versions but currently have no hash requirement. |
| Rust protobuf compiler | Root Cargo manifest/lock own `protoc-bin-vendored`. | Cargo integrity/locked resolution; Rust API build and tests. |
| Language linters and tests | Desktop/SDK npm manifests and locks, Python development requirements, and Go module files own tools. | Desktop TypeScript/Vitest/Prettier checks; SDK Ruff/mypy/Python tests, Go tests/vet/gofmt and TypeScript checks. |
| Browser acceptance | Desktop npm lock owns Playwright; `test:browser:install` installs its browser payloads in premerge. | Native browser acceptance jobs and Desktop tests. |
| Documentation | `scripts/docs-site` owns the Zensical image version/digest; `deploy/documentation/Dockerfile` mirrors that generator and pins the Nginx static runtime. Docs jobs and developers invoke Docker. | Digest-pinned build, strict site checks and Rust documentation examples via `cargo xtask check docs`; on-prem image HTTP checks use `deploy/documentation/smoke.mjs`. |
| ORAS and OPA | `release-image.yml` owns ORAS downloads; `premerge.yml` owns OPA download. | Explicit checksums; OCI roundtrip and live OPA acceptance. |
| Containers and devcontainer features | Workflow service/acceptance images, devcontainer base/Rust bootstrap images and `devcontainer-lock.json` own immutable references. `oci-proxy.Dockerfile` uses `scratch`. | Preserve image digests and feature lock; live PostgreSQL, Chroma, OCI and OPA suites. |
| Native build tools and container engines | Runner images and OS package installation supply Clang, CMake, pkg-config, Tauri libraries, Docker/Podman and platform packaging/signing tools. | Native CI/release acceptance; these are not supplied by `mise install`. |
| Compiler caches and Actions | Workflow SHA pins own sccache/setup/cache actions; `scripts/cargo-sccache` is an optional local wrapper. | Plain Cargo must work without sccache; preserve cold/warm jobs and cache behavior. |
| Git hooks and task dispatch | `.githooks/` and `scripts/install-git-hooks.sh` currently own hooks; `cargo xtask` owns check selection/execution. | Commit-message and pre-commit checks remain independent of hosted gates. |

## Change selection contract

Root mise configuration variants and environment lockfiles, `scripts/ci/mise-bootstrap.sh`,
`.mise/`, `mise-tasks/`, shared `.github/actions/`, `.devcontainer/` and Rust toolchain
files select Rust, SDK,
Desktop, documentation and dependency checks. Shared setup can affect all consumers;
the classifier deliberately errs toward exercising them. `CLAUDE.md` selects docs
alongside its canonical `AGENTS.md`. Individual path tests cover these selectors in
`scripts/ci/test-contracts.sh`; unrelated changes retain their existing classification.

The PR workflow loads classification from its trusted base revision. Changes to the
classifier must land before subsequent PRs rely on new selectors. `scripts/ci/check-toolchain.mjs` enforces equality between retained version declarations,
required Rust components and platform locks. The workflow gate runs this guard and
negative tests that introduce conflicting pins, missing checksums and unlocked setup.

## Development environments

| Environment | Requirements |
| --- | --- |
| macOS linked worktree | Native build dependencies, configured tools and a relative guide symlink resolving to `AGENTS.md`. |
| Linux contributor/devcontainer | Clang/Tauri dependencies, required Rust components, and Docker for the pinned documentation build. The devcontainer additionally includes editor components. |
| Windows linked worktree | Git symlink support, Git Bash for shell scripts, configured tools and native build dependencies. A text file containing the link target is not an equivalent guide. |
| Premerge and release runners | Representative Linux/macOS/Windows premerge tiers; six release targets: x64/ARM64 macOS, Linux musl and Windows MSVC. See [Tiered CI/CD](ci-cd.md). |
| Offline contributor setup | Already-installed tools, package caches, native libraries and container images. `mise install` needs network access on an empty machine; the runtime's offline support is separate. |

## Locked provisioning and updates

The supported lock targets are Linux x64/ARM64, macOS ARM64 and Windows x64. Lock metadata
is not platform execution evidence; native premerge acceptance remains required.
Cargo-audit uses verified prebuilt archives on Linux/Windows and native
`cargo install --locked` on macOS, because upstream supplies no ARM64 macOS binary.
Bookworm containers set `MISE_ENV=devcontainer` and use the locked source backend too:
the Linux x64 archive requires newer glibc than Bookworm supplies. `mise.devcontainer.lock`
records that source-build contract. Tool versions still come from the root variables.
The source backend disables binstall and relies on Cargo registry integrity plus the
crate's published dependency lock. Mise still records unused upstream archive metadata
when cross-platform locking; the tool's OS restriction controls installation. Rust compiler archives use rustup verification rather than
mise archive checksums; the Rust lock records the channel, profile and components.
Other release architectures continue using their existing setup until separately verified.

PR setup installs only the job's requested tools: Node/actionlint/Python for workflow
contracts (including Python source-archive tests), Rust for the separate formatting,
lint and unit-test jobs, Rust/Node/Python/Go for SDKs, Rust/Node for Desktop,
Rust for docs, and Rust/deny/audit for dependency policy. Mise caches installed tools;
separate npm, pip and Go caches retain package reuse. Existing sccache and Cargo registry
caches remain independent. Direct bin paths avoid auto-installing unselected tools
through mise shims. Initial installs require network access even with a lockfile.

Premerge reuses the setup on Linux x64, macOS ARM64 and Windows x64. Desktop jobs
retain npm download caches, compiler caches and native acceptance recipes. Windows
runtime renderer checks still run before Rust provisioning. Only the fuzz job sets
`MISE_ENV=fuzz`; its explicit `cargo +"$nightly"` selection is resolved from that
inventory. The Linux Desktop acceptance lane keeps its digest-pinned packaging container; the
guard checks its Rust and Node image versions against the inventory. The Linux OCI
proxy's musl target remains installed explicitly with rustup.
The container installs locked tools during image construction; editor-only Rust
components and the locked Docker feature remain separate from the build inventory.

To update a pin, edit its owner, refresh the lock with the mise version declared in
`scripts/ci/mise-bootstrap.sh`, and update retained consumer declarations:

```bash
mise lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64
mise --env fuzz lock --platform linux-x64
mise --env devcontainer lock --platform linux-x64,linux-arm64
mise install --locked
cargo xtask check workflows
```

Review archive URLs, checksums and backend changes. Updating mise itself also requires
reviewing its executable checksum and the action SHA. The guard intentionally treats
package engine minimums, image digests and native system packages as separate facts;
the root Dockerfile's Rust minor-series tag is checked against the canonical minor.
A rollback restores the setup action, inventory, environment locks and container
configuration together, then reruns the selected checks. Plain Cargo/script workflows continue to work without mise.
