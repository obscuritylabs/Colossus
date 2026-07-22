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

3. Run all workspace library tests:

    ```bash
    cargo test-fast
    ```

4. Run the complete suite when the change is ready:

    ```bash
    cargo test-full
    ```

5. Before handoff, run the authoritative gates:

    ```bash
    ./scripts/check_crate_roots.sh
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    ```

These local completion gates are distinct from hosted CI tiers. Every pull-request update
receives selected Linux/documentation validation, while reviewed final heads receive the
representative macOS, Windows, and live-security tier only when a writer applies
`ci:full`. Complete x64/ARM64 coverage is reserved for release tags. See
[Tiered CI/CD](ci-cd.md).

For cold builds or work across multiple worktrees, opt into the local compilation cache:

```bash
./scripts/cargo-sccache check -p colossus-runtime
./scripts/cargo-sccache test-fast
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

The pruned release compilation path requires an explicit non-runnable validation
sentinel:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_TEAM_ID=ADHOC npm run tauri:build
```

A sealed, runnable macOS application requires both the signing identity and the exact
10-character Apple Team ID embedded into the native runtime:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_SIGNING_IDENTITY='Developer ID Application: Example (TEAMID)' \
COLOSSUS_DESKTOP_TEAM_ID='TEAMID1234' \
  npm run tauri:bundle:macos
```

Set `COLOSSUS_DESKTOP_NOTARY_PROFILE` to a `notarytool` keychain profile to submit,
staple, assess, and archive the signed app without placing Apple credentials in argv or
environment variables. Use signing identity `-` only for local/CI structural validation;
it must be paired with `COLOSSUS_DESKTOP_TEAM_ID=ADHOC`. That explicit sentinel produces
a validation-only bundle whose release runtime rejects Managed Local startup; it is not a
runnable or distributable application.
When the profile lives outside the default search list, also set
`COLOSSUS_DESKTOP_NOTARY_KEYCHAIN` to its absolute keychain path.

Tag-triggered GitHub releases require the public Actions repository variable
`MACOS_TEAM_ID`. The credential-free build job embeds that canonical 10-character Team
ID without receiving any Actions Secrets context. Configure the same value as the
`MACOS_TEAM_ID` repository secret so the isolated signing job can fail closed if its
credential set does not match the unsigned app. That signing job uses an ephemeral
runner keychain and also requires these repository secrets:

- `MACOS_DEVELOPER_ID_P12_BASE64`: base64-encoded Developer ID Application certificate
  and private key in PKCS#12 form;
- `MACOS_DEVELOPER_ID_P12_PASSWORD`: the PKCS#12 export password;
- `MACOS_NOTARY_API_KEY_BASE64`: base64-encoded App Store Connect API private key (`.p8`);
- `MACOS_NOTARY_KEY_ID`: the App Store Connect API key identifier;
- `MACOS_NOTARY_ISSUER_ID`: the Team API issuer UUID;
- `MACOS_TEAM_ID`: a protected copy of the public repository variable, cross-checked
  against the app and the imported Developer ID identity.

The release jobs validate the variable and every secret, compare the imported
certificate's Team ID, grant key access only to the macOS signing tools, store the notary
profile in that ephemeral keychain, and delete the decoded files and keychain in an
`always()` cleanup step. Missing or inconsistent configuration fails a tag release
closed. Manual `workflow_dispatch` remains validation-only, uses ad-hoc signing,
embeds the rejected `ADHOC` sentinel, labels its artifacts `VALIDATION-ONLY-ADHOC`, and
cannot create a runnable app or draft release.

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
