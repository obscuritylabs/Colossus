# Colossus Agent Guide

This file is the short map. Keep deeper details in `docs/`.

- Read `docs/ARCHITECTURE.md` before changing boundaries.
- Read `docs/SECURITY.md` before changing tools, subprocess execution, policy, audit,
  or bundle handling.
- Keep `domain` dependency-free.
- Keep CLI, REPL, and TUI as interfaces only; no model, tool, policy, or state logic
  should live there.
- Add or update tests for every behavior change.
- Use `uv run pytest`, `uv run ruff check .`, and `uv run mypy src/colossus` before
  declaring Python implementation complete.
- Keep the Rust workspace under `rust/` until the P0+P1 cutover. Use Rust 1.96 and
  edition 2024; do not share configuration or state with the Python implementation.
- Use `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` from `rust/` before declaring Rust implementation complete.
