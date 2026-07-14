# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

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
