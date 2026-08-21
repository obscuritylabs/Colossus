# Colossus Agent Guide

This file is the short map. Keep deeper details in `docs/`.

- Read `docs/develop/architecture.md` before changing boundaries.
- Read `docs/develop/security-architecture.md` before changing tools, subprocess
  execution, policy, audit, or bundle handling.
- Follow `docs/develop/rust-practices.md` for nontrivial Rust changes and
  `docs/develop/testing.md` when adding, moving, or removing tests.
- Keep `domain` dependency-free.
- Keep CLI and TUI as interfaces only; no model, tool, policy, or state logic
  should live there.
- Keep crate roots (`lib.rs` and `main.rs`) as thin public API or composition
  surfaces. Split nontrivial configuration, metadata, resolution/service logic,
  adapters, and tests into focused modules instead of accumulating unrelated
  responsibilities in one file. Run `./scripts/check_crate_roots.sh` after structural
  changes.
- Tests should protect behavior and contracts that remain supported. When removing a
  feature, remove its feature-specific tests. Add rejection, migration, or tombstone
  tests only when the post-removal behavior is itself an intentional compatibility or
  security contract. Add or update tests for every other behavior change.
- Before merging, inspect unresolved human and automated review threads plus required
  checks. Address actionable findings in code and tests; see `docs/develop/contributing.md`.
- Rust is the active root implementation. Python 0.5 is retained only on
  `python-v0.5.0` and `python-legacy`; do not reintroduce its package or state.
- Use Rust 1.96 and edition 2024. Configuration and canonical state use the Rust YAML
  and redb formats; never silently import the legacy Python state.
- Use the smallest relevant test tier while iterating:
  - focused: `cargo test -p <changed-crate> --lib` plus directly affected test targets;
  - fast workspace: `cargo xtask dev` (cheap checks plus all workspace library tests);
  - Rust completion: `cargo xtask check rust`;
  - pre-PR: `cargo xtask pr --base origin/main` (change-selected Rust, SDK, desktop,
    documentation, dependency, and workflow checks).
  The focused and fast tiers shorten feedback loops but never replace the full completion
  gate.
- For cold or cross-worktree builds, any Cargo command may be run through
  `./scripts/cargo-sccache`. Keep this opt-in: ordinary `cargo` must continue to work
  when `sccache` is unavailable.
- Use `cargo xtask check rust` from the repository root before declaring implementation
  complete. It owns the formatting, structure, locked metadata, Clippy, workspace-test,
  and fuzz-harness gates used by PR validation. For now, run this command with permission
  to bind local loopback sockets: several integration tests start temporary local servers,
  and sandboxed runs otherwise fail with `Operation not permitted` after substantial work.
