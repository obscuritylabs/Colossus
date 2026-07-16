# Rust Runtime Status

Rust 0.7.0 is the active repository-root implementation. It uses Rust 1.96, edition
2024, strict YAML configuration, and encrypted redb state. It never imports Python
configuration or SQLite state. Python 0.5 remains frozen at `python-v0.5.0` and on the
`python-legacy` branch.

## Implemented Baseline

- The encrypted, hash-chained event journal is authoritative. It provides optimistic
  streams, projections, signed checkpoints, protected anchors, recovery mode, unknown
  effect recovery, audit views, and durable export work.
- Every external or sensitive effect crosses the safety kernel and built-in or OPA
  policy before a one-use permit reaches a private adapter. File, process, HTTP,
  provider, MCP, integration, memory-index, workflow, and extension results remain
  quarantined until any required release decision succeeds.
- Native macOS/Linux isolation, Windows AppContainer plus Job Objects, and OCI fallback
  share exact filesystem, process, environment, resource, and network obligations.
- Role-routed echo, OpenAI Responses, and OpenAI-compatible providers feed one bounded
  agent loop. Streaming items are normalized, individually released, and journaled
  before an interface observes them.
- `risk-auto` reviews approval-required `shell.run` requests through the policy-bound
  `risk_evaluator` role with tools disabled. Only a strict `low + allow` result creates
  an automatic proof; every other result or evaluator failure requires a prompt.
- Sessions, context snapshots, tasks, decisions, plans, goals, subagents, memories,
  research, skills, integrations, packs, bundles, presentation preferences, and
  telemetry are durable application services rather than CLI state.
- Tantivy is the offline memory index. Optional Chroma and embedding adapters are
  disposable projections; canonical records are always reloaded and rechecked.
- Hash-pinned YAML workflows support durable queueing, bounded control flow, waits,
  idempotent retries, explicit compensation, subworkflows, cancellation, and restart
  recovery.
- The authenticated worker and embedded runtime expose the same application API. The
  Ratatui surface owns only editing/layout and renders released typed documents from an
  `InteractiveHost`; protocol-v4 prompts and cancellation preserve the same boundaries.

The detailed behavioral contract and acceptance evidence live in
[Feature Inventory](FEATURE_INVENTORY.md) and
[Rust Acceptance Matrix](RUST_ACCEPTANCE_MATRIX.md). Security invariants live in
[Security Model](SECURITY.md); this status page intentionally does not duplicate them.

## Run And Verify

Initialize configuration and start an offline turn:

```bash
cargo run -p colossus-cli --bin colossus -- config init
cargo run -p colossus-cli --bin colossus -- run 'Reply with exactly: ok'
```

Start the interactive surface:

```bash
cargo run -p colossus-cli --bin colossus
```

Run the required local implementation checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [Getting Started](GETTING_STARTED.md), [User Guide](USER_GUIDE.md), and
[Configuration](CONFIGURATION.md) for the supported command and configuration surfaces.

## Release And Remaining Scope

The repository-root cutover is complete. The exact `v0.6.0` source revision passed the
explicit cutover workflow, including build, clean installation, and smoke tests for macOS,
static Linux, and Windows arm64/x64. The published release includes all six archives,
checksums, an SPDX inventory, the publisher identity, verification evidence, and a signed
multi-platform offline bundle. Ordinary `main` pushes continue to run the inexpensive
validation path; the full matrix belongs to pull requests and explicit release validation.

P2 remains intentionally open: schedules, webhooks, repository/event subscriptions,
PostgreSQL event storage, external WORM audit anchors, and additional adapters. These are
new product work, not hidden blockers in the Rust 0.6 baseline.
