---
status: archived
replacement:
  - /get-started/upgrade-compatibility/
  - /develop/architecture/
  - https://github.com/obscuritylabs/Colossus/blob/main/CHANGELOG.md
---

# Rust Runtime Status

Rust 0.10.8 is the active repository-root stable core release line. The latest published
Desktop proof remains the separate 0.10.2-preview.10 Developer Preview. The Rust runtime
uses Rust 1.96, edition 2024, strict schema-version-2 YAML configuration, and
hash-chained redb state with optional platform- or environment-backed encryption. It
never imports Python configuration or SQLite state. Python 0.5 remains
frozen at `python-v0.5.0` and on the `python-legacy` branch.

## Implemented Baseline

- The hash-chained event journal is authoritative. Explicit platform or environment keys
  add encryption and signed checkpoints; keyless state remains plaintext and emits a
  posture warning. The journal provides optimistic streams, projections, protected
  anchors, recovery mode, unknown-effect recovery, audit views, and durable export work.
- Every external or sensitive effect crosses the safety kernel and built-in or OPA
  policy before a one-use permit reaches a private adapter. File, process, HTTP,
  provider, MCP, integration, memory-index, workflow, and extension results remain
  quarantined until any required release decision succeeds.
- Native macOS/Linux isolation, Windows AppContainer plus Job Objects, and OCI fallback
  share exact filesystem, process, environment, resource, and network obligations.
- Role-routed echo, OpenAI Responses, and OpenAI-compatible providers feed one bounded
  agent loop. Streaming items are normalized, individually released, and journaled
  before an interface observes them.
- `risk-auto` reviews eligible approval-required shell, read-only network, and exact
  configured top-level MCP requests through the policy-bound `risk_evaluator` role with
  tools disabled. Only a strict `low + allow` result creates an automatic proof; every
  other result or evaluator failure requires a prompt.
- Sessions, context snapshots, tasks, decisions, plans, goals, subagents, memories,
  research, skills, integrations, packs, bundles, presentation preferences, and
  telemetry are durable application services rather than CLI state.
- Tantivy is the offline memory index. Optional Chroma and embedding adapters are
  disposable projections; canonical records are always reloaded and rechecked.
- Hash-pinned YAML workflows support durable queueing, bounded control flow, waits,
  idempotent retries, explicit compensation, subworkflows, cancellation, and restart
  recovery. Persisted fixed-cadence schedules add deterministic skip/fire-once misfire
  handling, atomic queued-run creation, explicit enable/disable, and process-kill-safe
  reconstruction.
- The authenticated worker and embedded runtime expose the same application API. The
  Ratatui surface owns only editing/layout and renders released typed documents from an
  `InteractiveHost`; protocol-v4 prompts and cancellation preserve the same boundaries.

The detailed behavioral contract and acceptance evidence live in
[Feature Inventory](feature-inventory.md) and
[Rust Acceptance Matrix](rust-acceptance-matrix.md). Current security invariants live in
[Security architecture](../../docs/develop/security-architecture.md); this status page
intentionally does not duplicate them.

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

See [Five-minute quickstart](../../docs/get-started/quickstart.md),
[Use Colossus](../../docs/use/index.md), and
[Configuration fields](../../docs/reference/configuration.md) for the supported
configuration surface.

## Release And Remaining Scope

The repository-root cutover is complete. The 0.10 line adds the authenticated public API
and language SDKs, Operations Studio Desktop, supervised Managed Local runtime, stable
signed and notarized Apple-silicon packaging, and a separately labeled unsigned Windows
Developer Preview path. Ordinary pull requests use classified validation; the full
platform, security, CLI, and channel-specific Desktop artifact matrix must pass the
explicit release gates before a version is published.

The latest published Developer Preview is
[v0.10.2-preview.10](https://github.com/obscuritylabs/Colossus/releases/tag/v0.10.2-preview.10).
Its release record carries the native runtime, sandbox, policy, dependency, fuzz,
package, checksum, and explicitly unsigned or ad-hoc-signed Desktop evidence produced
by the tag workflow.

PostgreSQL event storage, HTTPS WORM audit export, persisted schedules, authenticated
webhooks, and durable repository-event subscriptions are included in the 0.8 source
tree. Existing redb state remains authoritative unless an operator explicitly selects
and provisions PostgreSQL; no automatic storage migration occurs.

The 0.9.0 source also supports provider-neutral SearXNG/SerpAPI search routing,
reproducible signed multi-pack/data-only-skill collections, no-clobber verified
installation, and permit-bound authenticated registry pull/push using the same signed
collection format. Search and registry use are opt-in; local pack, OCI, collection, and
signed release-bundle verification retain credential-free offline paths. The
[Feature Inventory](feature-inventory.md#22-delivery-status) is the archived product-level
backlog; the acceptance matrix records executable evidence.
