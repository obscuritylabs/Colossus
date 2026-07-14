# Rust Runtime Status

Rust 0.6.0 is the active repository-root implementation. It uses Rust 1.96, edition
2024, fresh strict YAML configuration, and fresh redb state. It does not read or migrate
Python configuration or SQLite state.

The passing Python `0.5.0` baseline is frozen at the `python-v0.5.0` tag and on the
`python-legacy` branch. The active branch contains only the Rust runtime and supporting
release tooling.

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
- An exclusive cross-process redb writer lease shared by embedded CLI/REPL and worker
  startup. A second writer fails immediately instead of racing the canonical journal.
- A long-running single-writer worker with authenticated, bounded Unix-socket and Windows
  named-pipe framing, a pre-disclosure server handshake, connection-bound replay
  protection, streamed model events, session/workflow application operations, periodic
  safe draining, durable task/decision/plan/goal/subagent/memory operations, readiness,
  research/skill/pack/integration/MCP/process/network routing, clean shutdown, and
  automatic embedded fallback. Worker and embedded REPLs share the implemented
  slash-command surface and support interactive or scripted line-oriented input.
- The complete worker acceptance suite is cross-platform and wired to native Windows x64
  and arm64 CI. It proves named-pipe readiness, parallel authenticated clients, wrong-key
  rejection before operation disclosure, single-writer ownership, every implemented CLI
  route, clean shutdown, and embedded fallback.
- Restartable projection workers with optimistic per-projection positions, atomic redb
  record/position commits, deterministic rebuilds, lag/readiness diagnostics, and
  default session, work, memory, and workflow reducers. Session and work repository
  ports have concrete projected adapters.
- Checkpoints every 100 events or 60 seconds, plus explicit clean-shutdown checkpoints.
- A replaceable async audit-export port and durable outbox consumer with independent
  position, bounded exponential retry state, explicit unknown-outcome recovery, and
  recursion suppression. The initial directory adapter writes strict ciphertext-free
  evidence through `audit.export.write`; CLI and worker expose status, drain, and reset.
- Deterministic process-termination fault injection immediately before and after journal
  commit proves uncommitted rollback and atomic durable journal/head/stream/outbox state.
  A separate post-export/pre-ack crash proves deterministic evidence replay without
  duplicate files before the consumer advances. Verified startup repairs a periodic
  checkpoint interrupted after its event commit, and batches cannot skip the 100-event
  signing interval. Secure anchors are persisted before checkpoint metadata, with a
  process-kill test proving a missing signed checkpoint is recreated from the verified
  anchored head.
- A separate `cargo-fuzz` workspace drives nightly libFuzzer targets for strict
  journal/audit/effect/policy JSON contracts, workflow YAML/schema validation, and the
  non-executable condition grammar. The same committed seed corpora run on stable in the
  ordinary workspace test gate. Condition byte, token, and recursive nesting ceilings
  prevent parser and evaluator resource exhaustion.
- Startup conversion of abandoned `effect.started` records to
  `effect.outcome_unknown`, with no automatic retry.
- One effect gateway with a hard safety kernel, built-in deny-by-default policy, strict
  OPA REST decisions, remote HTTPS/mTLS/pinned trust requirements, hard-secret hashing,
  a 1 MiB fail-closed policy-input cap, approval proof re-evaluation, authenticated
  short-lived one-use permits, bounded quarantine, and optional post-effect release
  authorization.
- Permit-bound filesystem, subprocess, and HTTP adapters; an authenticated one-shot
  sandbox helper; Seatbelt/Landlock native isolation; cross-platform process-tree
  ownership; exact-origin network policy; native loopback and OCI sidecar allowlist
  proxies; bounded output and resources; post-effect HTTP/file quarantine; and hardened
  OCI command construction. Broker downgrades require an explicit configuration and
  policy grant.
- A required native sandbox matrix on macOS and Linux arm64/x64. Acceptance no longer
  silently skips when Seatbelt or Landlock is unavailable and covers symlink and parent
  traversal, environment clearing/rejection, child cleanup after timeout and normal exit,
  process-count and memory limits, exact proxy egress, and raw proxy-bypass attempts.
  Windows process isolation uses a per-job AppContainer plus an atomically attached Job
  Object; its native x64/arm64 suite covers filesystem/traversal, environment, process-tree,
  timeout, process-count, memory, authenticated exact-origin forwarding, missing/wrong
  proxy credentials, unlisted origins, raw-loopback bypass, WFP cleanup, and credential
  redaction. Networked effects fail closed if the package-scoped WFP or loopback setup is
  unavailable.
- Live Docker and Podman acceptance for bind mounts, immutable/preloaded workload and
  proxy images,
  environment clearing, read-only roots, proxy-only networking, raw-egress denial,
  timeouts, cancellation cleanup, and audited unknown outcomes. The structured-result
  suite passed locally on Podman 5.8.5 arm64 and remains wired into Linux CI. Live OPA
  acceptance covers readiness,
  allow/deny, approval proof
  re-evaluation, post-effect denial, invalid decisions, outages, decision-log warnings,
  pinned CA trust, and mutual TLS client identity.
- Strict, hash-pinned YAML workflow definitions; non-executable conditions; all planned
  typed step schemas; bounded step, concurrency, and 16-level call-depth budgets; direct
  and indirect cycle rejection; journal-native queue/claim/drain; durable output and
  attempt-budget reconstruction; root/nested wait input, resume, cancellation,
  interruption, `foreach`, and bounded parallel execution. Known failures retry only
  with explicit idempotency, compensation effects are separately gateway-dispatched,
  and interrupted non-idempotent effects are never retried. Subworkflow steps launch
  separately pinned, policy-authorized child runs with projected lineage, visible waiting
  identity, duplicate-free resume, crash repair, terminal propagation, and cancellation
  cascade. Repeated and parallel steps use durable scoped execution identities, so
  per-item inputs, effects, idempotency keys, retries, and child links reconstruct without
  cross-iteration reuse.
- Durable redb process-kill acceptance terminates a separate test process after a simulated
  external primary or compensation effect has been synced but before the workflow records
  a terminal result. Startup records one exact execution/attempt as `outcome_unknown`,
  refuses non-idempotent primary and all uncertain compensation replay, permits only the
  explicitly idempotent primary retry with the same key, and remains stable after another
  reopen. A separate kill immediately after durable step completion proves resume advances
  without repeating the completed step. Parallel-branch recovery preserves scoped
  idempotency and does not append a second completion for an already durable sibling.
  Linked-child intent survives a kill before child queueing, while a kill inside an
  idempotent child requires child-first recovery and then completes the parent without
  relaunching or duplicating the child.
- Shared factory-based conformance suites reopen the event-sourced research and extension
  repositories against the same journal. Research acceptance covers provenance, sequential
  source labels, citation integrity, terminal immutability, session filtering, and source/
  claim reconstruction. Extension acceptance covers integration reconnect/disconnect,
  immutable connection identity, bounded lists, pack install/disable/uninstall/reinstall,
  publisher trust, aggregate compatibility, and restart reconstruction.
- A bounded session work-state snapshot composes tasks, active decisions, actionable
  plans, current goals, and nonterminal subagents for `work`, embedded `/work`, and the
  authenticated worker REPL without duplicating repository logic in interfaces.
- Strict versioned presentation contracts, a `PresentationRepository` port, and an
  encrypted event-sourced adapter for theme, multiline composition, streaming, event
  detail, reasoning-summary visibility, and transcript density. Mutations cross the
  effect gateway and authenticated worker IPC; embedded and worker REPLs share the same
  commands and semantic work/context/provider-event renderer. Correlated `RunEventEnvelope`
  streaming adds durable run start/completion, tool start/result, recoverability, phase,
  and elapsed-time events. Semantic renderers distinguish file, shell, Git, work,
  context, repository, skill, web, MCP, trace, integration, pack, and generic result
  families. Interactive terminals refresh phase/tool elapsed activity in place while
  redirected output remains escape-free. Embedded and worker prompts render cached
  session, routed model, context, work, approval, preference, and last-run status through
  bounded application operations. Raw provider frames and hidden reasoning never enter
  this presentation path.
- Strict versioned JSON/TOML custom themes load from bounded config-adjacent and
  platform user libraries. Theme selection journals an immutable resolved palette and
  source hash, embedded and authenticated-worker REPLs share list/preview/select
  behavior, and the data-only legacy Python schema is strictly mapped for cutover.
- Policy-bound canonical memory create/update/archive/supersede/read/list/search operations;
  an atomic journal external-work outbox with independent durable consumer checkpoints;
  a disposable Tantivy lexical index with event-id idempotency, retryable lag,
  candidate-id search, status, and rebuild; canonical scope/status/expiry
  re-filtering; degraded index fallback; and post-effect-authorized context injection
  after decisions and before snapshots. Strict model tools derive repository/session
  scope from trusted runtime context, reject cross-scope targets, attribute writes to the
  model actor, and make a memory created on one turn available as non-instructional
  context on a later turn. An optional Chroma v2 projection advances independently beside
  Tantivy and stores only candidate ids,
  caller-generated embeddings, bounded text, and bounded metadata. Chroma and
  OpenAI-compatible embedding HTTP calls each cross the effect gateway; a deterministic
  local feature-hashing embedding profile remains available offline. Unknown Chroma
  mutation outcomes are durably marked and block automatic retry until an independently
  authorized rebuild resets and reconstructs the disposable projection. Per-consumer
  retry state, bounded exponential backoff, stable redacted errors, and readiness details
  survive restart. CI exercises the permit-bound v2 lifecycle against pinned current and
  previous Chroma releases.
- A composition root and strict YAML config with role-routed echo, OpenAI Responses, and
  OpenAI-compatible provider profiles. Provider generation and model-catalog calls use
  permit-bound adapters, disclose credential references (never values) to policy, resolve
  credentials only after authorization, incrementally decode Responses and compatible
  SSE, quarantine and optionally post-authorize every normalized item, discard raw hidden
  reasoning, durably append safe typed model events before observation, and aggregate
  normalized provider usage in telemetry.
- One-shot `run`, provider profile/doctor/model diagnostics, role-route inspection, a
  Reedline REPL using the same primary-role application path, audit and policy diagnostics,
  workflow CLI surface, and one-shot worker recovery/drain entry point.
- A reusable `colossus-agent` application loop and `colossus-tools` catalog with a
  configurable 1..=100 turn bound (24 by default), strict pre-policy JSON Schema
  validation, assistant/tool call-ID continuation for Responses and compatible chat,
  bounded two-attempt malformed-argument recovery, correlated tool results, explicit
  max-turn exhaustion, and the complete required offline catalog: filesystem, Git,
  structured shell, interactive user question, durable work, plan/goal, exact patch,
  repository context, subagent, discovery, trace, context, skill/resource, and echo
  tools. Configured MCP plus `web.fetch`, `docs.fetch`, and `network.http` use the same
  strict catalog. Effectful tools execute through the existing gateway; only `echo` is
  active by default, while `user.ask` is injected only for a real interactive embedded
  REPL. Workspace
  listing/search returns relative paths, does not follow links,
  excludes Colossus/Git control state, searches bounded UTF-8 files, and releases results
  only after post-effect policy authorization. Text create/overwrite/append/replace is
  atomic, approval-obligated by configuration, and returns a bounded diff plus changed
  line range after a separate release decision.
- Strict model-visible `task.create/update/list`, `decision.create/update/list/archive/
  supersede`, and `memory.create/update/list/search/archive/supersede` tools. Task and
  decision sessions are implicit, memory repository/session identities are runtime
  derived, every result is post-effect gated, and canonical target checks prevent model,
  workflow, or subagent callers from crossing their current scope. Agent-authored key
  decisions bind the next model turn; relevant memories are injected separately as
  explicitly non-instructional background.
- Git inspection and structured command execution share the authenticated sandbox helper
  but retain distinct action/capability identities. Git pathspec traversal, revision
  option injection, external diff/text-conversion helpers, and generic shell wrappers are
  rejected. Executables are exact configuration grants; argv is never shell-parsed;
  caller timeout/output requests may only narrow policy limits; and stdout/stderr remains
  quarantined until a post-effect decision. A nonzero exit is a known completed process
  result, while timeout, resource failure, and lost cleanup certainty retain their
  failed/unknown effect semantics.
- Terminal approval modes are composed into the same runtime gateway: one-shot commands
  default to `deny`, the REPL defaults to `ask`, `full-access` supplies proofs only for
  policy decisions that already require approval, and `risk-auto` safely falls back to
  an explicit prompt with `risk.status: unavailable` until the risk evaluator lands.
  Approval denial/error/grant events are durable, and caller-supplied post-effect phases
  cannot bypass the initial approval decision.
- Canonical event-sourced sessions with stable UUIDv7 ids, bounded titles and previews,
  append-only user/assistant/tool messages, optimistic stream versions, newest-first
  discovery, exact and latest resume, provider-history restoration, and restart
  acceptance across separate CLI processes. The REPL maintains one active session and
  exposes `/sessions`, exact resume, new session, and a numbered `/resume` picker.
- Durable Rust context preparation before every provider turn, with complete-request
  token estimates, 32,768-token fallback windows, 70/45 percent threshold and target,
  eight-message tail preservation, encrypted immutable snapshots, explicit activation
  and restoration events, optional policy-bound `context_summarizer` calls,
  deterministic fallback, and `context.prepared.v1` audit records. CLI and REPL expose
  status, list, compact, and restore operations without deleting canonical messages.
- Typed event-sourced tasks and key decisions with canonical reconstruction, bounded
  session/status queries, immutable identity/provenance, archival, atomic supersession,
  restart-safe CLI operations, and disposable `work-v1` projections. Active decisions
  are injected as binding context ahead of snapshots; archived and superseded records
  remain auditable without steering future turns.
- Durable session-scoped plans with ordered typed steps, immutable post-draft content,
  append-only draft/approve/execute/discard transitions, single-run execution guards,
  restart reconstruction, `work-v1` projection compatibility, strict model-visible
  `plan.create/show/approve_request` tools, canonical cross-session checks, and normal
  approval-proof re-evaluation. CLI list/show/create/approve and `/plans` inspect the
  same repository.
- Bounded durable Goal Mode using normal agent runs and the same session, context,
  provider, tool, policy, approval, sandbox, and audit paths. Goals reconstruct active,
  complete, or blocked state; enforce 1..=50 iteration budgets; preserve per-iteration
  run ids/output/timing; expose runtime-injected `goal.show/update` only on goal turns;
  stop on terminal state or errors; and report budget exhaustion without claiming
  completion. Approved plan consumption and linked goal creation are atomic and
  single-use. CLI `goals run/list/show` and REPL `/goal`/`/goals` use the same service.
- Durable queued subagents with parent run/call lineage, isolated child sessions,
  configurable positive concurrency (10 by default), normal policy/provider/tool/context
  execution, post-gated bounded results, cross-session confinement, explicit
  queued/running/completed/failed/cancelled/interrupted transitions, startup interruption
  recovery, and requeue restrictions. Child catalogs remove nested delegation and the
  executor independently denies it. CLI `agents queue/list/show/status/drain/cancel/
  requeue`, REPL `/agents`, one-shot parent-run draining, and worker draining share the
  same canonical queue.
- A locked Cargo dependency graph and CI jobs for formatting, Clippy with warnings
  denied, workspace tests, and pinned `cargo-deny`/`cargo-audit` policy for both the
  production and independent fuzz lockfiles while the frozen Python job remains green.
- A six-target native release-artifact matrix for macOS, static Linux, and Windows on
  arm64 and x64. Every job executes version/config parsing, a credential-free echo turn,
  and encrypted audit verification before packaging a user-facing `colossus` binary,
  platform installer, license, README, and SHA-256 sidecar. Each completed archive is
  extracted, installed into a clean prefix, and reruns the echo/audit smoke without
  Cargo, Python, provider credentials, or network. Installers reject linked package
  binaries and linked destination bin directories. Artifacts are uploaded independently
  so one platform cannot hide another platform's failure.
- Configured stdio MCP integration using the official Rust SDK protocol models, exact
  sandbox executable identities, environment-only credential references, deterministic
  paginated discovery, strict server/tool allowlists, live JSON Schema validation,
  approval-obligated invocation, bounded quarantine, post-effect release, echoed-secret
  redaction, CLI/REPL surfaces, and MCP-backed research collection. Servers remain hidden
  from the model catalog until configuration is present.

## Current Command Surface

From the repository root:

```bash
cargo run -p colossus-cli --bin colossus -- config init
cargo run -p colossus-cli --bin colossus -- echo hello
cargo run -p colossus-cli --bin colossus -- run 'Reply with exactly: ok'
cargo run -p colossus-cli --bin colossus -- --approval-mode ask run 'Create note.txt with filesystem.write'
cargo run -p colossus-cli --bin colossus -- provider profiles
cargo run -p colossus-cli --bin colossus -- provider doctor
cargo run -p colossus-cli --bin colossus -- provider models
cargo run -p colossus-cli --bin colossus -- models routes
cargo run -p colossus-cli --bin colossus -- tools list
cargo run -p colossus-cli --bin colossus -- sessions list
cargo run -p colossus-cli --bin colossus -- context status SESSION_ID
cargo run -p colossus-cli --bin colossus -- context compact SESSION_ID
cargo run -p colossus-cli --bin colossus -- tasks list --session SESSION_ID
cargo run -p colossus-cli --bin colossus -- decisions list --session SESSION_ID
cargo run -p colossus-cli --bin colossus -- plans list --session SESSION_ID
cargo run -p colossus-cli --bin colossus -- goals run 'Finish the scoped task' --session SESSION_ID --max-iterations 5
cargo run -p colossus-cli --bin colossus -- agents status
cargo run -p colossus-cli --bin colossus -- memories search 'query' --session SESSION_ID
cargo run -p colossus-cli --bin colossus -- memories index status
cargo run -p colossus-cli --bin colossus -- research run 'Summarize the audit architecture' --depth quick --source repo
cargo run -p colossus-cli --bin colossus -- research list
cargo run -p colossus-cli --bin colossus -- telemetry runs
cargo run -p colossus-cli --bin colossus -- telemetry metrics
cargo run -p colossus-cli --bin colossus -- skills list
cargo run -p colossus-cli --bin colossus -- run --skill coding 'Implement the scoped change'
cargo run -p colossus-cli --bin colossus -- --approval-mode ask skills scaffold my-skill 'My data-only skill'
cargo run -p colossus-cli --bin colossus -- skills validate path/to/local-skill --local
cargo run -p colossus-cli --bin colossus -- --approval-mode ask skills install path/to/local-skill
cargo run -p colossus-cli --bin colossus -- --approval-mode ask integrations import-openapi demo openapi.json --base-url https://api.example.test --credential-reference env:DEMO_API_TOKEN
cargo run -p colossus-cli --bin colossus -- integrations list
cargo run -p colossus-cli --bin colossus -- --approval-mode ask integrations connect github --credential-reference env:GITHUB_TOKEN
cargo run -p colossus-cli --bin colossus -- --approval-mode ask integrations connect searxng --base-url http://127.0.0.1:8888 --auth-type none
cargo run -p colossus-cli --bin colossus -- --approval-mode ask integrations connect opensearch --base-url http://127.0.0.1:9200 --auth-type none
cargo run -p colossus-cli --bin colossus -- mcp servers
cargo run -p colossus-cli --bin colossus -- mcp tools --server local
cargo run -p colossus-cli --bin colossus -- --approval-mode ask mcp call local search '{"query":"audit"}'
cargo run -p colossus-cli --bin colossus -- run --resume 'Continue'
cargo run -p colossus-cli --bin colossus -- audit verify
cargo run -p colossus-cli --bin colossus -- policy doctor
cargo run -p colossus-cli --bin colossus -- state doctor
cargo run -p colossus-cli --bin colossus -- sandbox doctor
cargo run -p colossus-cli --bin colossus -- process run /bin/echo --cwd . -- hello
cargo run -p colossus-cli --bin colossus -- projection status
cargo run -p colossus-cli --bin colossus -- projection rebuild
cargo run -p colossus-cli --bin colossus -- workflow validate .colossus/workflows/offline-echo.yaml
cargo run -p colossus-cli --bin colossus -- workflow register .colossus/workflows/offline-echo.yaml
cargo run -p colossus-cli --bin colossus -- workflow run offline-echo 1.0.0 --inputs '{}'
cargo run -p colossus-cli --bin colossus -- workflow run offline-echo 1.0.0 --inputs '{}' --queued
cargo run -p colossus-cli --bin colossus -- work --session SESSION_ID
cargo run -p colossus-cli --bin colossus -- preferences show
cargo run -p colossus-cli --bin colossus -- preferences history
cargo run -p colossus-cli --bin colossus -- repl
```

`config init` creates a unique platform credential-store identity for that fresh state
file. It neither asks for an application credential nor performs a network request.
`process run` remains deny-by-default: the example requires `process.spawn`, the exact
executable, its execute grant, a working-directory grant, and any environment/network
obligations to be present in the YAML configuration.

## Cutover Status

The Rust workspace has been promoted to the repository root, the Python runtime/package
has been removed from `main`, and the active package version is 0.6.0. The canonical
Cargo, local, installed, container, and release executable is `colossus`; the
transitional `colossus-rs` binary name is no longer produced. The local locked format,
Clippy, workspace-test, fuzz-workspace, offline echo, and audit gates are the immediate
cutover authority. `release/verify-local-cutover.sh` reproduces the complete host-side
gate, including pinned production and fuzz supply-chain policy, and rejects any
reintroduced Python package or tracked Python source.

The 2026-07-13 macOS arm64 cutover audit also reran the opt-in live suites without
Actions credits: OPA 1.16.2 passed decision, approval, release, readiness, outage,
pinned-trust, and mTLS acceptance; Chroma 1.5.8 and 1.5.9 each passed the complete v2
candidate-index lifecycle; and Docker passed the digest-pinned OCI escape and cleanup
suite. These host results strengthen local evidence but do not substitute for the
supported-platform or aggregate remote matrices.

Hosted GitHub Actions evidence is temporarily deferred while Actions credits are
unavailable. The fail-closed `rust-cutover-gate`, native sandbox/runtime matrices,
Windows x64/arm64 checks, live OPA/OCI/Chroma suites, and six-target artifact jobs remain
configured and must be rerun through an explicit release-validation workflow dispatch
before creating the final `v0.6.0` release tag. Pull requests retain the full test and
security matrices through `rust-pr-gate`, ordinary `main` pushes run only the inexpensive
Ubuntu compile gate, and release packaging is not duplicated after every merge. No
skipped or billing-blocked remote job is represented as passing.

P2 schedules, webhooks, repository events, event subscriptions, PostgreSQL storage,
external WORM audit anchors, and additional adapters remain post-0.6 work.
