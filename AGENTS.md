# Colossus Agent Guide

This file is the short map. Keep deeper details in `docs/`.

- Read `docs/ARCHITECTURE.md` before changing boundaries.
- Read `docs/CRATE_STRUCTURE.md` before adding modules or growing a crate root.
- Read `docs/SECURITY.md` before changing tools, subprocess execution, policy, audit,
  or bundle handling.
- Keep `domain` dependency-free.
- Keep CLI and TUI as interfaces only; no model, tool, policy, or state logic
  should live there.
- Keep crate roots (`lib.rs` and `main.rs`) as thin public API or composition
  surfaces. Split nontrivial configuration, metadata, resolution/service logic,
  adapters, and tests into focused modules instead of accumulating unrelated
  responsibilities in one file. Run `./scripts/check_crate_roots.sh` after structural
  changes.
- Add or update tests for every behavior change.
- Rust is the active root implementation. Python 0.5 is retained only on
  `python-v0.5.0` and `python-legacy`; do not reintroduce its package or state.
- Use Rust 1.96 and edition 2024. Configuration and canonical state use the Rust YAML
  and redb formats; never silently import the legacy Python state.
- Use the smallest relevant test tier while iterating:
  - focused: `cargo test -p <changed-crate> --lib` plus directly affected test targets;
  - fast workspace: `cargo test-fast` (all workspace library tests);
  - full acceptance: `cargo test-full` (the complete workspace suite).
  The focused and fast tiers shorten feedback loops but never replace the full completion
  gate.
- For cold or cross-worktree builds, any Cargo command may be run through
  `./scripts/cargo-sccache`. Keep this opt-in: ordinary `cargo` must continue to work
  when `sccache` is unavailable.
- Use `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` from the repository root before declaring implementation
  complete.
