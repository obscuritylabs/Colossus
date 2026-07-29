# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

## [0.10.2-preview.4] - 2026-07-29

### Fixed

- Made Developer Preview Desktop packaging credential-free end to end: macOS is ad-hoc
  signed without notarization, Windows is unsigned, and neither preview package requires
  or emits Tauri updater signatures.
- Passed the Windows Tauri release override through a temporary JSON file so PowerShell
  and the `tauri.cmd` shim cannot strip the JSON property-name quotes.
- Applied the root workspace's stripped release profile to the standalone Tauri workspace
  so the Windows PE remains inside the sealed-manifest executable-size bound.
- Moved Windows pre-merge acceptance and preview packaging to the configured
  `windows-latest-8-cores` larger runner.
- Disabled automatic Desktop updates for unsigned previews while preserving the signed,
  fail-closed updater contract for future stable releases.

### Upgrade Notes

- `v0.10.2-preview.4` supersedes the unpublished `v0.10.2-preview.3` attempt, whose
  standalone Tauri workspace retained symbols and produced a Windows PE larger than the
  sealed-manifest guard permits. It also supersedes `v0.10.2-preview.2`, whose inline
  JSON override lost its quotes in the PowerShell command shim, and
  `v0.10.2-preview.1`, whose unsigned path still required an updater-signing key. None of
  the failed attempts produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.1] - 2026-07-29

### Fixed

- Migrated the Unix and Windows release bundle-install smoke configurations to schema
  version 2, keeping provider connections separate from model profiles so all six CLI
  artifact jobs can complete packaging verification.
- Added release-contract coverage that rejects schema version 1 in either platform
  packaging script.

### Upgrade Notes

- `v0.10.2-preview.1` supersedes the unpublished `v0.10.1` release attempt, whose
  artifact workflow stopped during packaging verification. No `v0.10.1` GitHub Release
  assets were published.
- This release remains prerelease software. Its macOS Desktop build is ad-hoc signed,
  is not Apple-notarized, and uses the archive name
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.2-preview.1-aarch64-apple-darwin.zip`.
  Its Windows Desktop installer is unsigned and explicitly includes `UNSIGNED` in its
  filename.

## [0.10.1] - 2026-07-28

### Added

- Added stable Apple-silicon Desktop packaging with Developer ID signing, notarization,
  stapling, Gatekeeper assessment, signed update metadata, and an explicit user-driven
  update flow.
- Added a native Windows x64 Managed Local implementation with authenticated named
  pipes, Job Object lifecycle containment, ConPTY terminals, private credential storage,
  and a separately labeled unsigned Developer Preview installer path.
- Added workspace file previews, released artifact and attachment flows, richer run
  history, capability-driven Desktop controls, and delegated workflow execution across
  the public API and SDKs.
- Added operator-supplied CA bundles across supported outbound clients without exposing
  certificate contents or private storage paths to the renderer.

### Changed

- Split provider connection profiles from model profiles in configuration schema version
  2 so model identifiers, context limits, capabilities, and logical role routing are
  explicit and independently validated.
- Refined Desktop work, settings, diagnostics, files, artifacts, compact timelines, and
  responsive keyboard and accessibility behavior for production use.
- Made interactive `colossus run` print only the released assistant response while
  preserving the structured JSON output contract for automation.
- Added a locked cross-language development container and consolidated repository checks
  behind the change-selected `cargo xtask` workflow.

### Fixed

- Hardened provider streaming, OpenAI-compatible tool calls, approval notices, partial
  failures, and terminal reconstruction so released output remains ordered, bounded, and
  useful across CLI, TUI, worker, and Desktop clients.
- Corrected Windows workspace, file-handle, private-storage, settings-migration, process,
  and release-contract behavior across supported validation runners.
- Bound context summarization prompts and repaired Desktop, release, Chroma, and
  cross-platform acceptance paths exposed during production-readiness review.

### Security

- Sanitized provider-controlled failure details before public release and excluded common
  nested Docker, Kubernetes, cloud, and application-default credential stores from
  Desktop file previews.
- Bound Desktop updater metadata, release channels, immutable artifact URLs, updater
  signatures, sealed bundle manifests, code identities, and nested executable hashes so
  stable and Developer Preview packages cannot be confused or silently substituted.
- Revalidated Windows file handles and storage ownership at use time, kept native
  credentials and CA material outside renderer authority, and preserved fail-closed
  behavior for missing stable signing or notarization configuration.

### Upgrade Notes

- Configuration schema version 1 is no longer accepted. Generate a fresh schema version
  2 file with `colossus --config PATH config init`, then deliberately transfer reviewed
  provider, model, policy, storage, and integration settings. Canonical Rust YAML and
  redb state remain authoritative; no legacy Python or SQLite state is imported.
- The stable macOS archive is
  `Colossus-Desktop-v0.10.1-aarch64-apple-darwin.zip`; verify its adjacent SHA-256
  sidecar before installation. Existing preview-era workspace bindings may require one
  explicit folder reselection before Managed Local starts.
- Windows Desktop remains an unsigned Developer Preview and is not part of the stable
  release channel. Windows CLI archives remain stable release artifacts.

## [0.10.1-preview.2] - 2026-07-22

### Fixed

- Bound the checkout-free draft-release job directly to its GitHub repository so the
  validated artifacts can be assembled into the human-approved Developer Preview draft.

### Upgrade Notes

- `v0.10.1-preview.2` supersedes `v0.10.1-preview.1` as the current Developer Preview
  candidate. It remains prerelease software; its Desktop build is ad-hoc signed, is not
  Apple-notarized, and uses the archive name
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.1-preview.2-aarch64-apple-darwin.zip`.

## [0.10.1-preview.1] - 2026-07-22

### Added

- Added an explicitly labeled macOS Developer Preview channel so Managed Local can be
  tested before Apple Developer ID and notarization credentials are available.
- Added a persistent in-app **Developer Preview** warning sourced from the native
  compile-time release channel rather than renderer configuration.

### Security

- Developer Preview packaging remains ad-hoc signed, preserves the sealed manifest,
  exact nested-binary hashes, fixed code identifiers, and strict pre-spawn verification,
  but makes no Apple publisher-identity or notarization claim. The stable release channel
  still requires a canonical Apple Team ID, Developer ID signing, and notarization.

### Upgrade Notes

- `v0.10.1-preview.1` is a GitHub prerelease for testing, not the stable `0.10.1`
  release. Its Desktop archive is named
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.1-preview.1-aarch64-apple-darwin.zip`.
- Verify the adjacent SHA-256 sidecar before opening the archive. Because this preview is
  not Apple-notarized, macOS requires an explicit Control-click **Open** or
  **System Settings → Privacy & Security → Open Anyway** confirmation. Do not disable
  Gatekeeper or strip quarantine metadata.

## [0.10.0] - 2026-07-22

### Added

- Added a versioned public gRPC API with scoped application credentials, pinned TLS
  identity, durable run and prompt operations, compatibility fixtures, and generated
  TypeScript, Python, and Go SDKs alongside the native Rust SDK.
- Added Colossus Operations Studio, a dark blue Tauri workspace for individual work,
  multi-target fleet health, released artifacts, activity, settings, and sanitized
  Markdown run transcripts.
- Added Managed Local as the folder-first Desktop path: a supervised macOS sidecar,
  native provider enrollment into the system keychain, workspace-scoped lifecycle,
  bounded restart recovery, and authenticated parity with external daemon targets.
- Added opt-in local shell tabs and a one-click Colossus TUI that attaches to the existing
  managed worker, plus a unified access-profile model for consistent CLI, TUI, worker,
  public API, and Desktop authority.
- Added the workspace development shell with explicit filesystem and network policy,
  native sandbox initialization, and protected-shell handling on Linux and macOS.

### Changed

- Split hosted validation into fail-closed, path-classified PR and pre-merge tiers while
  retaining stable aggregate gates; release automation now builds six native CLI targets
  and a separately signed and notarized Apple-silicon Desktop artifact.
- Refactored runtime, worker, CLI, TUI, workflow, presentation, sandbox, pack, and service
  crate roots into focused composition modules, and migrated the documentation site to
  Zensical with redesigned product and architecture guidance.
- Made durable queue insertion wake workers immediately so newly queued and resumed work
  does not wait for the polling interval.

### Fixed

- Made Rust public-API code generation use an exact cross-platform vendored `protoc` so
  clean developer, CI, and release runners do not depend on an ambient compiler install.
- Shortened overlong macOS worker socket paths through an owner-private, deterministic
  lease endpoint while preserving direct legacy endpoints wherever the operating system
  accepts them.
- Bound workspace ownership to platform process identity, capped aggregate skill and pack
  discovery resources, and preserved graceful run draining, transport force-close, and
  checkpoint shutdown across managed-runtime exits.

### Security

- Managed Local verifies the bundle manifest and macOS code identity immediately before
  no-shell process creation, transfers one-use credentials and provider secrets only over
  a bounded inherited channel, and fails closed on identity, TLS, API, grant, workspace,
  or writer-lease mismatch.
- Provider secrets are resolved from native memory only after policy authorization,
  zeroized after use, and excluded from renderer DTOs, configuration, environment,
  command arguments, logs, terminal sessions, telemetry, and run context.
- Desktop terminals accept only native-selected shell or TUI sessions for opaque
  workspace handles, isolate capabilities in a dedicated local WebView, bound all I/O,
  disable escape-sequence clipboard writes and automatic navigation, and guarantee
  process-tree cleanup.
- Public API and SDK clients authenticate every connection with a certificate pin,
  instance identity, API compatibility check, scoped bearer, exact role/tool grants, and
  bounded watch limits; administrative and delegated authority remain excluded.

### Upgrade Notes

- Existing daemon installations and external Desktop targets remain supported without
  migration. New Desktop installations default to Managed Local and keep app-owned state
  outside the selected repository.
- Managed Local and signed Desktop distribution are macOS-first in this release. Other
  platforms continue to use the CLI, TUI, daemon, and public SDK surfaces until their
  native terminal and packaging backends land.
- The public API is still `v1alpha1`, and the TypeScript and Python SDK packages remain
  alpha packages; package publication is separate from this source and binary release.

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
