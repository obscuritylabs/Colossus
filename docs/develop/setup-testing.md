---
title: Source setup and test tiers
description: Build Colossus from source and choose focused, fast, and full verification.
audience: developer
type: tutorial
---

# Source setup and test tiers

## Goal

Build the workspace with the supported Rust toolchain and establish a fast, trustworthy
test loop.

## Prerequisites

- Rust `1.96` with edition `2024` support.
- Git and the native build dependencies required by your platform.
- A source checkout at the repository root.

The tracked development container is the supported ready-to-build Linux environment.
It uses digest-pinned official Debian Bookworm base images and a locked Rust feature,
selects Clang for native dependencies, pins the Rust, Node.js, Python, and Go versions
used by CI, includes the Tauri system libraries, installs the pinned `actionlint`,
`cargo-deny`, and `cargo-audit` tools used by the local PR gate, and provides an isolated
Docker daemon for documentation builds. In Codespaces or VS Code, rebuild the container
after changing `.devcontainer/`.

Rust API contract builds use the exact cross-platform `protoc-bin-vendored` workspace
dependency, so contributors and release runners do not need an ambient `protoc` binary.
Language SDK generation remains separate and uses its own pinned local generator
toolchain under `sdk/`.

## Steps

1. Confirm the toolchain and build the workspace:

    ```bash
    rustc --version
    cargo build --workspace
    ```

2. Run one focused crate test while iterating:

    ```bash
    cargo test -p colossus-policy --lib
    ```

    Add directly affected integration targets where appropriate:

    ```bash
    cargo test -p colossus-cli --test config_security
    ```

    Maintainers with a dedicated Splunk test endpoint can also run the ignored native
    Streamable HTTP smoke test:

    ```bash
    COLOSSUS_LIVE_SPLUNK_MCP_URL=https://splunk.example.test/services/mcp \
    SPLUNK_MCP_TOKEN=... \
      cargo test -p colossus-mcp --features live-splunk \
        live_splunk_streamable_http_discovery -- --ignored
    ```

3. Run the fast development tier. It checks the diff, formatting, crate roots, and all
   workspace library tests:

    ```bash
    cargo xtask dev
    ```

4. Run the complete Rust gate when the change is ready:

    ```bash
    cargo xtask check rust
    ```

5. Before opening or updating a pull request, run change-selected validation against
   the target branch:

    ```bash
    cargo xtask pr --base origin/main
    ```

`cargo xtask pr` always checks workflow contracts and then uses the repository's
fail-closed path classifier to select Rust, public SDK, Desktop, documentation, and
dependency-policy components. Component checks are also directly available as
`cargo xtask check rust`, `sdk`, `desktop`, `docs`, `dependencies`, `sidecar`, and
`workflows`. The task runner orchestrates repository-owned checks; hosted CI still owns
trusted-base decisions, runner provisioning, AppArmor installation, artifact upload,
and platform acceptance.

Every pull-request update receives selected Linux/documentation validation, while
reviewed final heads receive the representative macOS, Windows, and live-security tier
only when a writer applies `ci:full`. Complete x64/ARM64 coverage is reserved for release
tags. See [Tiered CI/CD](ci-cd.md).

For cold builds or work across multiple worktrees, opt into the local compilation cache:

```bash
./scripts/cargo-sccache check -p colossus-runtime
./scripts/cargo-sccache xtask dev
sccache --show-stats
```

Ordinary `cargo` remains supported when `sccache` is unavailable.

To run an isolated development TUI:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

The launcher creates development-only configuration, independent environment key
material, state, and secure anchor under `.colossus`. It compiles before loading keys
and then executes the binary directly.

To run Colossus Desktop with its debug Managed Local sidecar and bundled CLI:

```bash
./scripts/desktop-dev
```

The launcher installs the locked renderer dependencies, builds and stages both native
executables for the host target, and opens the Tauri development app. An External
daemon `connection.local.json` is optional. If the file exists, its instance identity
and certificate pin must be valid; remove it to test Managed Local only.

Debug Desktop uses a keyless plaintext journal in a separate
`development-plaintext/` Managed Local state partition, so local iteration does not
prompt for journal keys in the platform keychain. This mode retains the journal hash
chain but has no payload confidentiality, signed checkpoints, or external rollback
anchor. Release builds continue to use platform-protected journals and never reuse the
debug partition.

The pruned release compilation path requires an explicit non-runnable validation
channel and sentinel:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_RELEASE_CHANNEL=validation_only \
COLOSSUS_DESKTOP_TEAM_ID=ADHOC \
  npm run tauri:build
```

A stable, sealed macOS application requires both the signing identity and the exact
10-character Apple Team ID embedded into the native runtime:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_SIGNING_IDENTITY='Developer ID Application: Example (TEAMID)' \
COLOSSUS_DESKTOP_TEAM_ID='TEAMID1234' \
COLOSSUS_DESKTOP_RELEASE_CHANNEL=stable \
  npm run tauri:bundle:macos
```

Set `COLOSSUS_DESKTOP_NOTARY_PROFILE` to a `notarytool` keychain profile to submit,
staple, assess, and archive the signed app without placing Apple credentials in argv or
environment variables.
When the profile lives outside the default search list, also set
`COLOSSUS_DESKTOP_NOTARY_KEYCHAIN` to its absolute keychain path.

For an explicitly labeled, runnable Developer Preview, use only the ad-hoc identity and
preview channel. This historical 0.10.1 preview command illustrates the contract; never
attach a notary profile to this build:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_RELEASE_VERSION=0.10.1-preview.2 \
COLOSSUS_DESKTOP_RELEASE_CHANNEL=developer_preview \
COLOSSUS_DESKTOP_TEAM_ID=ADHOC \
COLOSSUS_DESKTOP_SIGNING_IDENTITY=- \
  npm run tauri:bundle:macos
```

The Developer Preview retains strict signature, fixed identifier, sealed-manifest, and
nested-binary hash verification, but ad-hoc signing does not establish Apple publisher
identity and the app is not notarized. The native release channel keeps a persistent
warning visible in the application. The separate `validation_only` channel also requires
the `ADHOC` sentinel and identity `-`, but its runtime intentionally rejects Managed Local
startup. Stable packaging rejects both ad-hoc channels, and stable release publication
for the production Desktop track still requires Developer ID plus notarization.

Stable core `vX.Y.Z` tags skip every Desktop job. They publish CLI and SDK candidates
without any Apple signing, notarization, or Tauri updater credential. Production Desktop
credentials belong to a separate future release track and must not be added to the core
release environment.

A canonical `vX.Y.Z-preview.N` Developer Preview tag is the credential-free runnable
Desktop path: it uses the ad-hoc preview channel, reads no Apple signing secret, and
creates clearly named unnotarized macOS and unsigned Windows assets. Manual dispatch of
a preview version remains validation-only, embeds the rejected `ADHOC` sentinel, labels
its artifacts `VALIDATION-ONLY-ADHOC`, and cannot create a runnable app or draft release.
Manual dispatch of a stable version instead validates the immutable SDK candidate and
requires all Desktop jobs to be skipped. See [Core release operations](releasing.md) for
registry bootstrap, stable tag, approval, and recovery steps.

## Expected result

The workspace builds, focused tests provide a short feedback loop, and the local
completion gates finish without formatting drift, warnings, or test failures. Hosted
pre-merge acceptance remains a separate final-PR requirement.

## Verification

Confirm that `git status --short` contains only intentional source, test, and
documentation changes. Run the smallest command a reviewer can use to reproduce the
behavior and include it in the handoff.

## Failure path

Use the first compiler, Clippy, or test failure as the diagnostic source. Do not bypass
the required toolchain, deny-warnings policy, dependency rules, or platform acceptance
tests. A fast tier is useful feedback but never substitutes for the completion gates.

## Next step

Read [Architecture overview](architecture.md) before moving code across crates.
