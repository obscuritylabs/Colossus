# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

### Added

- Added the ChatGPT subscription-backed Codex provider to Desktop Managed Local,
  including native official-CLI sign-in/sign-out status and per-model reasoning effort.
- Added the shared owner-private Colossus home, opaque per-workspace CLI/Desktop state
  partitions, `COLOSSUS_HOME`, and `storage.location: home_workspace`.
- Added automatic bounded home and repository `AGENTS.md` instruction snapshots for
  top-level user runs, Goal iterations, and delegated subagents.

### Changed

- `config init` now creates the user-level configuration, while `config init --local`
  creates a complete repository replacement. Resolution is explicit, repository, then
  home without merging.
- Desktop Managed Local now starts fresh under the workspace's Desktop home partition;
  earlier application-support data is preserved but ignored.
- Codex CLI account commands now report completion only after Colossus verifies their
  runtime credential postcondition; unsafe stores are rejected before the official CLI
  is invoked, and logout must leave no usable credential behind.

### Security

- Kept Codex tokens and account identifiers out of the Desktop WebView, settings, and
  generated runtime YAML; the native host passes only the validated owner-private auth
  file path over inherited sidecar bootstrap IPC.
- Open Codex auth files with no-follow, nonblocking Unix semantics so special files such
  as FIFOs fail promptly instead of stalling credential readiness checks.
- Required no-follow owner-private Colossus homes and confined home-workspace storage;
  direct installers create only the empty root, and privileged Unix installs defer it
  until an end-user launch.
- Kept `AGENTS.md` outside authorization: it cannot widen tools, policy, approvals,
  sandbox roots, network origins, or immutable runtime instructions.

## [0.10.7] - 2026-08-11

### Fixed

- Accepted GitHub's compact versioned-API JSON in the Unix bootstrap installer while
  retaining bounded metadata, release identity, channel, and asset-name validation.

## [0.10.6] - 2026-08-10

### Added

- Added install-aware `colossus update` replacement for validated direct installations,
  with exact-version selection, downgrade refusal, the reviewed fixed-origin bootstrap
  embedded in the binary, and a detached Windows handoff for locked executables.
- Added unauthenticated macOS, Linux, and Windows installation verification after every
  published stable release.
- Added a locked four-platform Nix flake plus a reviewed two-architecture Homebrew
  formula and checksum-driven formula generator. Package-manager wrappers advertise
  upgrade guidance without gaining direct self-update authority.

### Changed

- Enabled npm provenance for trusted SDK publication now that the source repository is
  public.

### Security

- Disabled ambient proxy use explicitly in the Unix bootstrap metadata and asset
  downloads while retaining exact HTTPS origin and redirect checks.
- Updated Ratatui's cache dependency to patched `lru` 0.18.2 and documented a narrow,
  reachability-reviewed `RUSTSEC-2026-0253` exception for Tantivy 0.26.1 until its
  already-merged `lru` 0.18.2 update is published.

## [0.10.5] - 2026-08-08

### Added

- Added explicitly acknowledged direct-execution sandbox modes, including a dangerous
  full-access profile that can use ambient executables, permitted process resources,
  host working directories, and child-process networking after acknowledgement.
- Added keyless plaintext redb and PostgreSQL storage for deployments that intentionally
  rely on host storage controls, with bounded startup and worker compatibility checks.
- Added dense, searchable session and theme browsers, generated command help, and an
  explicit Direct-or-Goal decision surface for approved plan execution.

### Changed

- Docked effect approval review above the composer with focused summary, exact-request,
  and protection views that preserve complete sanitized scope.
- Reduced storage commit amplification during startup while retaining typed schema
  validation on the read-only fast path.
- Expanded provider connection documentation across supported hosted, compatible, and
  local model backends.

### Fixed

- Kept completion, session, theme, help, and plan-selection chrome off durable inline
  terminal scrollback by rendering focus-taking command surfaces transiently.
- Added Splunk Streamable HTTP compatibility for empty one-way acknowledgements while
  keeping credential-header exemptions scoped to the configured credential map.
- Paged resumed-session previews past tool-only records so recent user and assistant
  context remains visible in tool-heavy sessions.

### Security

- Kept dangerous full-access execution behind explicit acknowledgement while preserving
  policy, approval, audit, process-limit, and resource enforcement.
- Protected plaintext worker startup and Windows worker authentication material, and
  rejected incompatible live workers through the versioned IPC contract.

## [0.10.4] - 2026-08-04

### Fixed

- Ensured the stable draft-release job runs after the intentionally skipped Desktop
  jobs once the complete core release gate succeeds.
- Wrote Windows CLI checksum sidecars with portable LF endings so the Linux draft
  verifier and Unix consumers can validate them with `sha256sum --check`.

## [0.10.3] - 2026-08-04

### Fixed

- Allowed the exact deterministic Python source-distribution normalizer through stable
  release readiness while continuing to reject the retired root Python package and any
  other unapproved Python source.

## [0.10.2] - 2026-08-03

### Added

- Added a coordinated stable core release channel that publishes the six native CLI
  archives plus version-aligned npm, PyPI, and Go SDK releases from one source commit.
- Added immutable SDK candidate manifests and checksums, protected OIDC registry
  publishing, exact-byte recovery checks, and an independently tagged Go submodule.

### Changed

- Decoupled stable CLI and SDK releases from Apple signing, notarization, updater keys,
  and Desktop packaging. Developer Preview tags retain their explicitly unsigned
  Desktop artifacts; production Desktop distribution remains a separate release track.
- Marked every internal Rust package as non-publishable so the coordinated release does
  not accidentally expose workspace crates on crates.io.

### Release Notes

- The first stable core release requires registry trusted-publisher setup and a
  protected `sdk-production` GitHub environment before the approved draft is published.
- The Python distribution is `obscuritylabs-colossus-sdk` because the normalized
  `colossus-sdk` PyPI name belongs to an unrelated project. The import remains
  `colossus_sdk`.
- npm provenance is disabled while this repository is private because npm does not
  support provenance statements for public packages built from private repositories.

## [0.10.2-preview.10] - 2026-08-03

### Changed

- Kept YAML fenced blocks on the safe plain-text fallback so the syntax-highlighting
  dependency graph remains compatible with the supported static musl release targets.

## [0.10.2-preview.9] - 2026-08-03

### Added

- Added ChatGPT/Codex subscription authentication through the official Codex credential
  store, including browser and device-code login, status and logout commands, bounded
  token refresh, streamed Responses turns, tool calls, and model-specific reasoning
  effort levels.
- Added terminal-native transcript scrollback as the default TUI mode. Finalized user,
  assistant, tool, and system entries move into ordinary terminal history for mouse
  selection, copy, search, and wheel navigation while the composer and status remain
  sticky.
- Added CommonMark parsing and syntax-highlighted fenced code blocks to terminal
  presentation, with tables, task lists, nested emphasis, blockquotes, links, image
  fallbacks, bounded wrapping, and safe large-block fallbacks.
- Added the complete interactive Plan workflow to full-screen TUI and scripted line
  mode, with process-local Execute/Plan state, same-session Draft selection and
  refinement, approval, discard, and Direct or bounded Goal execution in embedded and
  worker-backed runtimes.
- Added `/goal resume GOAL_ID` for continuing the remaining budget of an Active goal
  after cancellation or bounded failure.
- Added typed `PlanWritten` run events and canonical plan evidence on completed and
  cancelled Plan Mode runs. Public API, protobuf, and SDK completed results and
  cancellations now expose the optional canonical `plan_id`.
- Added authenticated worker protocol v6 `RunInteractive` variants for Execute and Plan
  turns, plan lifecycle operations, Direct and Goal execution, Goal resume, notices,
  prompts, released events, and cancellation.

### Changed

- Made `--alt-screen` the explicit opt-in for the application-owned full-screen
  transcript. `--no-alt-screen` remains a compatibility alias for the default inline
  native-scrollback viewport.
- Made Ctrl-C exit an idle TUI, request cooperative cancellation for an active run, and
  exit on a second press during cancellation. `/exit` and Ctrl-D on an empty idle
  composer remain available.
- Preserved intermediate assistant output and tool boundaries as finalized transcript
  entries so native scrollback and resumed session history retain the complete visible
  run rather than only the final answer.
- Added optimistic plan revisions: legacy records default to revision 0, new plans start
  at 1, and refinement or any lifecycle transition increments the revision and rejects
  stale requests.
- Made every completed Plan Mode turn perform exactly one successful `plan.create` or
  runtime-bound `plan.update`. A missing write receives one corrective turn before
  failing closed, while duplicate writes are blocked before dispatch.
- Made approved-plan consumption atomic. Cancel or failure before consumption preserves
  terminal Plan mode and selection; after consumption, Direct and Goal outcomes retain
  canonical plan plus completion, cancellation, or bounded-failure evidence.

### Security

- Kept ChatGPT access, refresh, and account credentials out of configuration, model
  input, diagnostics, and durable run history. Codex provider access remains bound to
  the exact ChatGPT and OpenAI authentication origins through the normal effect gateway.
- Bound `plan.update` to the server-selected Draft id and revision, kept
  `plan.discard` operator-only, and routed update, discard, approval, Direct execution,
  and Goal handoff through the normal effect gateway.
- Corrected the Plan Mode inspection allowlist to `context.show` and
  `context.snapshots` while continuing to exclude filesystem writes, patch application,
  command execution, approval, networking, delegation, and plan consumption.

### Upgrade Notes

- Worker protocol v6 is intentionally incompatible with stale resident workers. Restart
  the worker and client with the same Colossus version after upgrading. Interrupted
  interactive operations are not automatically retried; inspect `/plans` and linked run
  or Goal evidence first.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.8] - 2026-07-30

### Fixed

- Replaced the Windows executable detachment's backup-oriented `File.Replace` call
  with the same-volume overwrite form of `File.Move`. This keeps the verified
  single-link replacement atomic without passing an empty backup path to .NET on
  the `windows-latest-l` runner image.

### Upgrade Notes

- `v0.10.2-preview.8` supersedes the unpublished `v0.10.2-preview.7` attempt.
  Preview.7 passed release readiness and compiled the Windows Desktop application,
  but Windows packaging stopped when `File.Replace` rejected its null backup-path
  argument. That attempt produced no GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.7] - 2026-07-30

### Fixed

- Detached Cargo's hard-linked top-level Windows Desktop executable with a verified,
  same-volume atomic replacement before sealing its manifest binding. The manifest
  patcher still rejects symbolic links, additional hard links, noncanonical paths, and
  oversized files.
- Kept x64 Windows acceptance, CLI packaging, and unsigned Desktop packaging on the
  exact `windows-latest-l` larger-runner label.

### Upgrade Notes

- `v0.10.2-preview.7` supersedes the unpublished `v0.10.2-preview.6` attempt. Its
  AppArmor readiness check, all six CLI targets, and macOS Desktop package passed, but
  Windows Desktop packaging correctly rejected Cargo's multiply linked PE before
  manifest binding. No earlier preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.6] - 2026-07-29

### Fixed

- Staged CI AppArmor attachment binaries in a run-unique, root-level directory so the
  exact-path profile has no runner-controlled `/usr`, `/usr/local`, or `/opt` ancestor.
  The executable and its only replaceable parent remain root-owned and non-writable by
  unprivileged users.
- Made AppArmor path and parser validation run against a harmless root-owned stub before
  compiling Colossus, so incompatible runners fail in seconds without starting release
  artifact jobs.
- Kept x64 Windows acceptance and unsigned preview packaging on the provisioned
  `windows-latest-l` larger runner.

### Upgrade Notes

- `v0.10.2-preview.6` supersedes the unpublished `v0.10.2-preview.5` attempt. Its
  ARM64 release-readiness runner exposed `/usr` as runner-controlled, so the hardened
  AppArmor installer rejected the attachment before any artifact jobs started. No
  earlier preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.5] - 2026-07-29

### Fixed

- Expanded the sealed Desktop manifest patcher's bounded executable allowance to 1 GiB
  so the stripped Windows Developer Preview PE can be bound without weakening its
  regular-file, canonical-path, or link-count checks.
- Staged AppArmor acceptance binaries below root-controlled `/usr/lib` so both x64 and
  ARM64 GitHub-hosted Linux runners satisfy the exact-path attachment policy.
- Kept x64 Windows acceptance and preview packaging on the provisioned
  `windows-latest-l` larger runner.

### Upgrade Notes

- `v0.10.2-preview.5` supersedes the unpublished `v0.10.2-preview.4` attempt. Its
  Windows Desktop executable remained above the previous 512 MiB patching ceiling, and
  its ARM64 Linux runner exposed a replaceable `/usr/local/libexec` ancestor. No earlier
  preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

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
  `windows-latest-l` larger runner.
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
