# Colossus Rust runtime

This directory contains the event-sourced Colossus reconstruction. It intentionally uses
fresh YAML configuration and fresh state; the frozen Python implementation is available
at the `python-v0.5.0` tag and `python-legacy` branch.

The initial alpha implements the contracts, encrypted redb journal, exclusive writer
lease, restartable redb projections and projected repositories, policy gateway, and
durable workflow core before effectful provider and tool adapters are added.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
