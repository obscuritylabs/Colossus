# Rust Reconstruction Status

The Rust reconstruction lives under `rust/` until the P0+P1 cutover. It uses version
`0.6.0-alpha.1`, Rust 1.96, edition 2024, fresh strict YAML configuration, and fresh
redb state. It does not read or migrate Python configuration or SQLite state.

The passing Python `0.5.0` baseline is frozen at the `python-v0.5.0` tag and on the
`python-legacy` branch. The active branch continues the Rust reconstruction without the
obsolete Go launcher.

## Implemented Foundation

- A dependency-free domain crate plus strict serializable journal, policy, approval,
  permit, memory-port, and workflow contracts.
- Split ports for the journal, signing, projections, aggregate repositories, workflow
  repositories, memory indexes, embeddings, audit export, policy, and approvals.
- An encrypted redb journal using XChaCha20-Poly1305, UUIDv7 event identifiers,
  optimistic stream versions, a global hash chain, atomic projection outbox records,
  Ed25519 checkpoints, and separately protected anchors.
- Platform Keychain/DPAPI/Secret Service keys and an explicit environment key provider.
  Missing historical keys fail; journal payloads never downgrade to plaintext.
- Startup verification of event sequences, stream versions, payload authentication,
  plaintext hashes, record hashes, projection outbox positions, checkpoints, and secure
  anchors. Failure opens the runtime read-only.
- Checkpoints every 100 events or 60 seconds, plus explicit clean-shutdown checkpoints.
- Startup conversion of abandoned `effect.started` records to
  `effect.outcome_unknown`, with no automatic retry.
- One effect gateway with a hard safety kernel, built-in deny-by-default policy, strict
  OPA REST decisions, remote HTTPS/mTLS/pinned trust requirements, hard-secret hashing,
  a 1 MiB fail-closed policy-input cap, approval proof re-evaluation, authenticated
  short-lived one-use permits, bounded quarantine, and optional post-effect release
  authorization.
- Strict, hash-pinned YAML workflow definitions; non-executable conditions; all planned
  typed step schemas; bounded step and concurrency budgets; direct-cycle rejection;
  durable run reconstruction; wait/input, resume, cancellation, interruption, `foreach`,
  and bounded parallel execution. Interrupted non-idempotent effects are not retried.
- Canonical event-sourced memory create/archive/supersede reconstruction plus a
  disposable Tantivy lexical index with event-id idempotency, candidate-id search,
  removal, status, and rebuild behavior.
- A composition root, strict YAML config, credential-free/network-free echo provider,
  Reedline REPL, audit and policy diagnostics, workflow CLI surface, and one-shot worker
  recovery/drain entry point.
- A locked Cargo dependency graph and CI jobs for formatting, Clippy with warnings
  denied, and workspace tests while the frozen Python job remains green.

## Current Command Surface

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- config init
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- echo hello
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- audit verify
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- policy doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow validate .colossus/workflows/offline-echo.yaml
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow register .colossus/workflows/offline-echo.yaml
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow run offline-echo 1.0.0 --inputs '{}'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- repl
```

`config init` creates a unique platform credential-store identity for that fresh state
file. It neither asks for an application credential nor performs a network request.

## Remaining Delivery Milestones

This alpha is the audit/storage, authorization, and workflow foundation, not the P0+P1
cutover. The following planned work remains:

- Projection processors and concrete session, work, research, and extension
  repositories with their shared conformance suites.
- Chroma semantic candidates, embedding providers, queued index lag operations, and
  application-layer policy re-filtering of canonical memory records.
- Birdcage and Windows sandbox helpers, OCI fallback, the allowlist network proxy,
  broker downgrade prompts, resource enforcement, and platform escape suites.
- OpenAI Responses and OpenAI-compatible providers, the core tool catalog, provider
  diagnostics/routing, sessions, streaming, and context compaction.
- A long-running single-writer worker lease and authenticated Unix-socket/named-pipe IPC.
- Goals, durable subagents, research/citations, skills/resources, telemetry, packs,
  integrations, offline bundles, and the rest of P1/P2.
- Fuzzing, dependency/license/vulnerability policy, cross-platform sandbox tests, and
  six-target release smoke tests.

Rust is promoted to the repository root only after those P0+P1 acceptance checks pass.
