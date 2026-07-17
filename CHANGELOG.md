# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

## [0.9.0] - 2026-07-17

### Added

- Added provider-neutral `web.search` with explicit SearXNG or SerpAPI profiles, exact
  `agent` and `research` role routing, normalized bounded results, operator diagnostics,
  and shared agent/Deep Research execution.
- Added reproducible signed collections for packs and data-only skills, including
  independent publisher verification, complete dependency-closure validation,
  no-clobber batch installation, and CLI, TUI, worker, and embedded runtime operations.
- Added authenticated registry pull and create-only push for signed collections using
  deterministic bounded archives and optional environment-backed credentials.

### Changed

- Consolidated remaining product scope into the feature inventory and removed the
  superseded standalone roadmap.
- Made the default provider-neutral search user agent identify the 0.9 release line.

### Security

- `web.search` now crosses the ordinary effect gateway with an exact origin, DNS pinning,
  redirects and ambient proxies disabled, late credential resolution, mandatory
  post-effect authorization, and credential removal before normalization or release.
- Collection and registry operations reject traversal, links, special or undeclared
  entries, signature or hash mismatches, incomplete pack dependency closure, destination
  clobbering, and unapproved network or filesystem access.

### Upgrade Notes

- Existing configurations remain valid. The deprecated 0.8 `research.search` SearXNG
  form is accepted only when top-level `search` is absent; new `web.search` use requires
  an explicit role mapping and never falls back to another provider.
- Existing state and installed packs are not migrated or replaced automatically.
  Collections and registry transport are opt-in, and installation refuses existing
  destinations.

## [0.8.1] - 2026-07-16

### Added

- Added `scripts/colossus-dev`, an isolated source-development launcher that reuses the
  debug build, loads owner-only environment keys only after compilation, and keeps its
  configuration, redb state, and secure anchor separate from operator state.

### Changed

- Rotated the official Ed25519 offline-bundle publisher identity and added `.env` to
  ignored local files so an operator-held signing seed cannot be accidentally staged.

### Security

- The `v0.8.0` source tag remains immutable, but no signed GitHub release was published
  from it after the previous private seed became unavailable. Version 0.8.1 establishes
  a new exact publisher/key binding and is the first signed distribution of the 0.8
  release line.
- Operators who trusted the earlier publisher identity must verify the new
  `release/bundle-publisher.json` through an independent channel and explicitly replace
  the old trust binding before installing the 0.8.1 signed bundle.

## [0.8.0] - 2026-07-16

### Added

- Added durable fixed-cadence workflow schedules with deterministic skip or fire-once
  misfire handling, hash-pinned definitions, atomic queued-run creation, and
  process-kill-safe recovery.
- Added HMAC-SHA256 authenticated workflow webhooks with bounded replay protection,
  exact raw-body verification, policy and audit routing, atomic delivery receipts, and
  loopback HTTP, CLI, TUI, and worker surfaces.
- Added exact repository-domain-event workflow subscriptions with optional stream
  scoping, durable checkpoints, deterministic run identities, duplicate-delivery
  suppression, and deferred policy-refusal handling.
- Added a transactional PostgreSQL journal and projection adapter with encrypted
  envelopes, global-chain serialization, shared repository conformance, TLS policy,
  credential-reference-only configuration, and concurrent-process recovery tests.
- Added permit-bound HTTPS WORM audit export with deterministic create-only objects,
  exact-origin networking, credential redaction, idempotent replay, and durable unknown
  outcome handling.
- Added a searchable Rust-native mdBook documentation site deployed from `main`, with
  executable navigation, link, and Pages-permission contracts.

### Changed

- Parallelized the pull-request and release validation matrices behind a fast formatting
  and locked-metadata preflight, added shared compiler caching, removed redundant native
  workspace compilation, and replaced source-built supply-chain tools with pinned,
  checksum-verified release binaries.
- Expanded the storage configuration to select redb or PostgreSQL explicitly while
  preserving redb as the local default and canonical configuration behavior.
- Updated workflow, storage, security, operator, and reconstruction documentation for
  the new durable trigger and external-storage boundaries.

### Fixed

- Hardened Windows worker named-pipe backlog, replacement-listener, routing, shutdown,
  and contention behavior so a busy authenticated worker is never mistaken for an
  absent worker that permits a second embedded writer.
- Granted the documentation workflow the Pages permission required by its pinned setup
  action and added regression coverage for that deployment boundary.
- Scoped `sccache` environment variables to jobs that install the wrapper, preventing
  dependency-policy jobs from attempting to invoke a missing compiler wrapper.
- Increased the bounded OCI control-command startup allowance so cold rootless Podman
  initialization does not spuriously fail the same security acceptance suite that
  Docker completes.
- Reused the production RustSec database for the fuzz lockfile audit, removing a
  redundant network fetch from the fail-closed supply-chain job.
- Injected both standard Unix proxy-variable spellings for native and OCI sandboxes so
  clients such as curl use the authenticated allowlist proxy on every release platform.

### Security

- Workflow triggers revalidate definition hashes, call graphs, input schemas, replay
  state, and policy immediately before atomic dispatch; trigger creation grants no
  downstream effect authority.
- PostgreSQL and WORM credentials remain late-resolved environment references, verified
  TLS is required outside explicit loopback acceptance, and diagnostics never release
  connection strings, bearer values, ciphertext, or plaintext audit payloads.

### Upgrade Notes

- Existing redb installations remain redb installations. Selecting PostgreSQL is an
  explicit configuration change and does not silently import, copy, or replace local
  state; operators must provision and validate the target independently.
- Schedule, webhook, and subscription records are additive. Existing workflow
  definitions and runs retain their current hashes and behavior until an operator
  explicitly creates and enables a trigger.
- Signed multi-pack collections and authenticated remote pack-registry pull/push
  operations are intentionally deferred to the 0.9.0 roadmap. Local verified packs,
  OCI layouts, and signed offline release bundles remain supported in 0.8.0.

## [0.7.0] - 2026-07-15

### Added

- Added a Ratatui terminal with a durable paged and reflowing transcript, pinned composer
  and footer, modal overlays, queued input, encrypted history, and terminal restoration
  across resize, cancellation, panic, and normal exit paths.
- Added width-aware semantic tables, status and result blocks, Markdown, source previews
  with line numbers, styled diffs, separated process streams, and compact borderless
  rendering for large resource and tool listings.
- Added visible slash-command and discovered `@skill` completion menus, fish-style ghost
  text, a guided theme picker, complete semantic theme previews, and the tested custom
  Ocean theme.
- Added authenticated worker protocol v4 frames for approval decisions, user input, and
  cooperative cancellation with embedded and worker-host parity.
- Added `--output auto|human|json`; terminals default to human output while pipes retain
  stable JSON for automation.

### Changed

- Made the Ratatui interface the sole interactive terminal owner for `colossus` and
  `colossus tui`, while preserving non-TTY line mode and explicit JSON automation.
- Replaced raw structured dumps with intentional list/detail views, explicit empty
  states, brighter readable theme text, and color-separated prompts, answers, reasoning,
  tools, warnings, and errors.
- Routed session resume, approvals, and `user.ask` through stable foreground TUI flows
  with explicit answer and cancellation guidance.
- Woke the bounded scheduler immediately for model-created subagents so a parent can
  receive the completed child result in the same turn, and rendered queued/running
  results as pending instead of failed.
- Replaced nested theme JSON with an active-state table and readable custom-theme search
  locations; selection now saves immediately and updates the complete TUI palette.

### Removed

- Removed the superseded public `repl` subcommand and its competing terminal ownership
  path. Durable journal compatibility terminology remains internal only.

## [0.6.0] - 2026-07-14

### Added

- Auditable Rust agent and workflow runtime with an encrypted event journal, policy
  gateway, durable sessions/workflows, memory indexes, sandboxed effects, and native
  distribution tooling.
- OpenAI Responses, OpenAI-compatible, and credential-free echo providers with CLI,
  REPL, worker, and embedded runtime surfaces.
- Model-assisted `risk-auto` review for approval-required shell requests. Only a strict
  low-risk allow result can create an automatic approval proof; all other results and
  evaluator failures return control to the user.
- Release-readiness documentation for installation, configuration, offline and
  airgapped operation, bundle format, release process, and security policy.
- Continuous integration covering formatting, linting, tests, fuzzing, supply-chain
  policy, live security adapters, and six native release targets.

### Changed

- Promoted the Rust workspace to the repository root and made Rust 1.96/edition 2024 the
  active build contract.
- Replaced the Python-dependent commit checker, development container, Docker image, and
  CI layout with Rust-root equivalents.
- Renamed the transitional `colossus-rs` executable to the canonical `colossus` command
  used by installed, container, and release artifacts.
- Added a reproducible host-side cutover verifier that pins Rust and supply-chain tools,
  rejects reintroduced Python source, and checks both production and fuzz dependency
  graphs.
- Split hosted validation into an inexpensive post-merge Ubuntu gate, a fail-closed pull
  request test/security gate, and an explicit six-target release gate.

### Fixed

- Preserved the exact persisted event representation during journal hash verification
  and authenticated decryption so additive context fields do not invalidate older Rust
  journal records.
- Cached platform credential material per service/account for the process lifetime so
  journal replay and concurrent runtime setup do not repeatedly reopen the same Keychain,
  DPAPI, or Secret Service entry; failed credential reads remain uncached.
- Hardened authenticated worker IPC and Windows named-pipe retries so canonical response
  payloads remain authenticated under contention without weakening timeout behavior on
  other platforms.
- Parsed protocol-skill frontmatter identically with LF or CRLF line endings so native
  Windows archive and signed-bundle installations load the bundled skill library.

### Removed

- Removed the Python 0.5 runtime, tests, packaging, and SQLite state contract from
  `main`; the frozen implementation remains at `python-v0.5.0` and on `python-legacy`.

## [0.1.0] - 2026-06-08

### Added

- Initial secure layered CLI harness with CLI, REPL, and TUI interfaces.
- Ports-and-adapters architecture with dependency-inward boundaries.
- Deterministic echo provider and OpenAI-compatible provider adapters.
- Brokered built-in tool metadata, policy decisions, local state, and audit logging.
- Bundled skills and offline bundle manifest verification.
