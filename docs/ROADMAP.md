# Post-0.8 Roadmap

Colossus 0.8.0 is the operational workflow and storage release after the Rust cutover
and the 0.7 terminal release. It delivers the durable trigger, PostgreSQL, WORM audit,
documentation, and CI tracks recorded below. Signed multi-pack collections and remote
registry operations are explicitly deferred to 0.9.0.

This page records the delivered 0.8 boundary and the remaining-work index. The
normative behavioral requirements remain in the [Feature Inventory](FEATURE_INVENTORY.md),
while the [Acceptance Matrix](RUST_ACCEPTANCE_MATRIX.md) maps requirements to executable
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
  telemetry, and distribution acceptance is green through the 0.7.0 release, with the
  0.8 workflow-trigger and storage additions covered by the same safety kernel.
- **Complete:** searchable Rust-native documentation and GitHub Pages deployment, with
  complete navigation and executable link/build contracts.

## 0.8 Delivered Tracks

### 1. Retire Stale Interface And Documentation State

- **Complete:** the mdBook user/developer site is published from the Markdown source on
  `main`.
- **Complete:** every active page is present in `docs/SUMMARY.md`, with tests for missing
  navigation and broken local links.
- **Complete:** active version, command, path, and release-evidence claims were refreshed
  for the Rust-only runtime.
- **Complete:** removed-REPL names were audited; only required durable schema identities
  remain, with restart compatibility preserved rather than silently rewriting state.
- **Complete:** the root cutover verifier continues to reject a tracked Python package or
  Python source.

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

- **Complete in this source tree:** a PostgreSQL event-journal adapter sits behind the
  existing journal port without weakening optimistic streams, the global hash chain,
  encrypted payloads, atomic outboxes, writer ownership, checkpoints, anchors, or
  recovery mode.
- **Complete:** shared journal, repository, projection, external-work, and crash/reopen
  conformance suites run against PostgreSQL.
- **Complete:** a remote append-only/WORM audit exporter behind the existing exporter
  port preserves the same ciphertext-free evidence, permit, retry, unknown-outcome, and
  acknowledgment contract as the directory exporter.

Exit evidence is present in pinned live PostgreSQL CI, concurrent-writer/outage recovery,
kill-point recovery, shared conformance, idempotent WORM replay, credential
non-disclosure, and adapter-aware operator diagnostics. A repository-configured HTTPS
WORM endpoint can additionally run its destructive live acceptance during an explicit
workflow dispatch.

### 4. Deferred 0.9 Distribution And Extension Operations

- Complete signed offline collections for multi-pack/skill distribution with publisher
  trust, deterministic manifests, dependency closure, tamper rejection, and no-clobber
  installation.
- Add remote pack registry pull, push, and authentication workflows without exposing
  credential values to schemas, model context, telemetry, or audit payloads.
- Preserve local/offline operation when registry adapters are absent or denied.

These capabilities are not part of the 0.8.0 release contract. Local verified packs,
OCI layouts, and signed multi-platform offline release bundles remain supported. The
0.9 implementation must preserve that offline behavior and satisfy the evidence below.

Exit evidence: reproducible collection fixtures, registry loopback-live acceptance,
tamper/traversal/symlink tests, credential echo redaction, offline fallback, signed
release assets, and clean-prefix install/use verification.

### 5. 0.8 Release Proof

- All delivered 0.8 tracks above have authoritative docs and executable acceptance
  evidence; the explicitly deferred 0.9 track is not represented as complete.
- `cargo fmt --all -- --check`, warnings-denied workspace Clippy, workspace tests,
  fuzz workspace checks, dependency policy, and vulnerability audits pass.
- The explicit release workflow passes native macOS, static Linux, and Windows arm64/x64
  runtime, sandbox, and packaging jobs from the exact release revision.
- The published release contains six native archives, checksums, SPDX inventory,
  publisher identity, a signed offline bundle, and fresh install/audit evidence.
- The deployed documentation site identifies 0.8.0 as current and links to the exact
  release and validation evidence.

## Explicit Non-Goals

The remote multi-user control plane, graphical desktop/browser client, unbounded or
self-replicating agent trees, raw chain-of-thought storage, and automatic execution of
skill-directory scripts remain explicit product non-goals. They are not hidden 0.8 work.
Changing one requires an intentional product and security-contract decision first.
