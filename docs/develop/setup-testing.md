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
