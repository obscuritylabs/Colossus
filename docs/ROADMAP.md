# Roadmap To 0.8.0

Colossus 0.8.0 is the operational-completeness release after the Rust cutover and the
0.7 terminal release. This page is the current remaining-work index. The normative
behavioral requirements remain in the [Feature Inventory](FEATURE_INVENTORY.md), while
the [Acceptance Matrix](RUST_ACCEPTANCE_MATRIX.md) maps requirements to executable
evidence.

Work is complete only when its implementation, restart/recovery behavior, user and
developer documentation, local tests, and applicable hosted platform gates all pass.
An implemented happy path without those boundaries remains open.

## Current Baseline

- **Complete:** the repository root contains only the Rust runtime, Cargo workspaces,
  Rust YAML configuration, and encrypted redb state contract. Python 0.5 remains a
  historical tag and branch, not an active package or compatibility path.
- **Complete:** P0/P1 agent, policy, sandbox, provider, TUI, session, context, work,
  workflow, goal, subagent, memory, research, skill, integration, pack, bundle,
  telemetry, and distribution acceptance is green through the 0.7.0 release.
- **Complete in this source tree:** searchable Rust-native documentation and GitHub
  Pages deployment, with complete navigation and executable link/build contracts.

## 0.8 Completion Tracks

### 1. Retire Stale Interface And Documentation State

- Publish the mdBook user/developer site from the Markdown source on `main`.
- Keep every active page in `docs/SUMMARY.md`; fail tests for missing navigation or
  broken local links.
- Remove stale active-version, command, path, and release-evidence claims.
- Replace internal names that still describe the removed REPL where they are not a
  required durable schema identity. Required schema changes need an explicit Rust-state
  migration and restart tests; they must not silently import Python state.
- Keep the root cutover verifier rejecting a tracked Python package or Python source.

Exit evidence: a clean mdBook build and Pages deployment, documentation contract tests,
repository-wide stale-identity audit, state migration/restart tests where applicable,
and the ordinary Rust formatting, Clippy, and workspace test gates.

### 2. Durable Workflow Triggers And Subscriptions

- **Complete:** persisted schedule definitions with bounded fixed cadence,
  enable/disable state, skip/fire-once misfire behavior, deterministic next-fire and run
  identity, atomic schedule/run queueing, worker/embedded routing, and process-kill
  recovery.
- **Complete:** authenticated HMAC-SHA256 webhook bindings with late credential
  resolution, bounded body/header/replay validation, exact-delivery idempotency,
  definition-hash trust, ordinary effect/policy/audit routing, atomic delivery/run
  queueing, worker/embedded parity, and a loopback-only HTTP adapter.
- **Complete:** exact domain-event subscriptions with optional stream-prefix scope,
  durable global checkpoints, at-least-once source replay, deterministic event/run
  idempotency, definition/input trust, ordinary policy routing, atomic
  checkpoint/delivery/run queueing, worker/embedded parity, and process-kill recovery.
- **Complete:** every trigger-created run routes through the existing hash-pinned queue,
  worker coordination lock, policy, approvals, and recovery rules.

Exit evidence is present in shared repository conformance, clock-controlled schedule
tests, webhook authentication/replay/size rejection tests, subscription restart and
forced duplicate-delivery, filter, schema/trust blocking, and deferred-dispatch isolation
tests, worker/embedded parity, and separate-process redb process-kill recovery without a
duplicate run.

### 3. Replaceable Durable Storage And Audit Export

- Implement a PostgreSQL event-journal adapter behind the existing journal port without
  weakening optimistic streams, the global hash chain, encrypted payloads, atomic
  outboxes, writer ownership, checkpoints, anchors, or recovery mode.
- Run the shared journal, repository, projection, and crash/reopen conformance suites
  against PostgreSQL.
- Add a remote append-only/WORM audit exporter behind the existing exporter port with
  the same ciphertext-free evidence, permit, retry, unknown-outcome, and acknowledgment
  contract as the directory exporter.

Exit evidence: opt-in live PostgreSQL and WORM-adapter CI, concurrent-writer and outage
tests, kill-point recovery, idempotent replay, credential non-disclosure, and operator
diagnostics that distinguish unavailable, lagging, blocked, and recovery-only states.

### 4. Distribution And Extension Operations

- Complete signed offline collections for multi-pack/skill distribution with publisher
  trust, deterministic manifests, dependency closure, tamper rejection, and no-clobber
  installation.
- Add remote pack registry pull, push, and authentication workflows without exposing
  credential values to schemas, model context, telemetry, or audit payloads.
- Preserve local/offline operation when registry adapters are absent or denied.

Exit evidence: reproducible collection fixtures, registry loopback-live acceptance,
  tamper/traversal/symlink tests, credential echo redaction, offline fallback, signed
  release assets, and clean-prefix install/use verification.

### 5. 0.8 Release Proof

- All tracks above have authoritative docs and executable acceptance evidence.
- `cargo fmt --all -- --check`, warnings-denied workspace Clippy, workspace tests,
  fuzz workspace checks, dependency policy, and vulnerability audits pass.
- The explicit release workflow passes native macOS, static Linux, and Windows arm64/x64
  runtime, sandbox, and packaging jobs from the exact release revision.
- The published release contains six native archives, checksums, SPDX inventory,
  publisher identity, signed offline bundle/collections, and fresh install/audit evidence.
- The deployed documentation site identifies 0.8.0 as current and links to the exact
  release and validation evidence.

## Explicit Non-Goals

The remote multi-user control plane, graphical desktop/browser client, unbounded or
self-replicating agent trees, raw chain-of-thought storage, and automatic execution of
skill-directory scripts remain explicit product non-goals. They are not hidden 0.8 work.
Changing one requires an intentional product and security-contract decision first.
