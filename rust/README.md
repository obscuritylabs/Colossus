# Colossus Rust runtime

This directory contains the event-sourced Colossus reconstruction. It intentionally uses
fresh YAML configuration and fresh state; the frozen Python implementation is available
at the `python-v0.5.0` tag and `python-legacy` branch.

The initial alpha implements the contracts, encrypted redb journal, exclusive writer
lease, restartable redb projections and projected repositories, policy gateway, durable
workflow core, and permit-bound filesystem/process/HTTP adapters. macOS and Linux use a
one-shot authenticated Seatbelt/Landlock helper; Windows requires the OCI backend until
native filesystem and network isolation are complete.

Inspect sandbox readiness or run an explicitly configured exact executable:

```sh
cargo run -p colossus-cli --bin colossus-rs -- sandbox doctor
cargo run -p colossus-cli --bin colossus-rs -- process run /bin/echo --cwd . -- hello
```

Process and network actions are deny-by-default. Add the exact action, executable,
filesystem grants, environment names, and canonical HTTP(S) origins to the fresh YAML
configuration before invoking them.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
