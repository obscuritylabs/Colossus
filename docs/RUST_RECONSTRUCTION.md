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
- An exclusive cross-process redb writer lease shared by embedded CLI/REPL and worker
  startup. A second writer fails immediately instead of racing the canonical journal.
- A long-running single-writer worker with authenticated, bounded Unix-socket and Windows
  named-pipe framing, a pre-disclosure server handshake, connection-bound replay
  protection, streamed model events, session/workflow application operations, periodic
  safe draining, durable task/decision/plan/goal/subagent/memory operations, readiness,
  research/skill/pack/integration/MCP/process/network routing, clean shutdown, and
  automatic embedded fallback.
- Restartable projection workers with optimistic per-projection positions, atomic redb
  record/position commits, deterministic rebuilds, lag/readiness diagnostics, and
  default session, work, memory, and workflow reducers. Session and work repository
  ports have concrete projected adapters.
- Checkpoints every 100 events or 60 seconds, plus explicit clean-shutdown checkpoints.
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
- Live Docker acceptance for bind mounts, immutable/preloaded workload and proxy images,
  environment clearing, read-only roots, proxy-only networking, raw-egress denial,
  timeouts, cancellation cleanup, and audited unknown outcomes. The same suite is wired
  into Linux CI for Podman revalidation. Live OPA acceptance covers readiness,
  allow/deny, approval proof
  re-evaluation, post-effect denial, invalid decisions, outages, decision-log warnings,
  pinned CA trust, and mutual TLS client identity.
- Strict, hash-pinned YAML workflow definitions; non-executable conditions; all planned
  typed step schemas; bounded step and concurrency budgets; direct-cycle rejection;
  durable run reconstruction; wait/input, resume, cancellation, interruption, `foreach`,
  and bounded parallel execution. Interrupted non-idempotent effects are not retried.
- Policy-bound canonical memory create/update/archive/supersede/read/list/search operations;
  a disposable Tantivy lexical index with event-id idempotency, durable replay position,
  retryable lag, candidate-id search, status, and rebuild; canonical scope/status/expiry
  re-filtering; degraded index fallback; and post-effect-authorized context injection
  after decisions and before snapshots. Strict model tools derive repository/session
  scope from trusted runtime context, reject cross-scope targets, attribute writes to the
  model actor, and make a memory created on one turn available as non-instructional
  context on a later turn. A selectable Chroma v2 projection stores only candidate ids,
  caller-generated embeddings, bounded text, and bounded metadata. Chroma and
  OpenAI-compatible embedding HTTP calls each cross the effect gateway; a deterministic
  local feature-hashing embedding profile remains available offline. Unknown Chroma
  mutation outcomes are durably marked and block automatic retry until an independently
  authorized rebuild resets and reconstructs the disposable projection.
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
  max-turn exhaustion, and model-visible `echo`, `filesystem.list`, `filesystem.read`,
  `filesystem.search`, `filesystem.write`, `filesystem.replace`, `git.status`,
  `git.diff`, `git.show`, `shell.run`, and `network.http` tools. Effectful tools execute
  through the existing gateway; only `echo` is active by default. Workspace
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
  denied, and workspace tests while the frozen Python job remains green.
- Configured stdio MCP integration using the official Rust SDK protocol models, exact
  sandbox executable identities, environment-only credential references, deterministic
  paginated discovery, strict server/tool allowlists, live JSON Schema validation,
  approval-obligated invocation, bounded quarantine, post-effect release, echoed-secret
  redaction, CLI/REPL surfaces, and MCP-backed research collection. Servers remain hidden
  from the model catalog until configuration is present.

## Current Command Surface

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- config init
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- echo hello
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- run 'Reply with exactly: ok'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask run 'Create note.txt with filesystem.write'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- provider profiles
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- provider doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- provider models
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- models routes
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- tools list
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- sessions list
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- context status SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- context compact SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- tasks list --session SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- decisions list --session SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- plans list --session SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- goals run 'Finish the scoped task' --session SESSION_ID --max-iterations 5
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- agents status
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- memories search 'query' --session SESSION_ID
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- memories index status
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- research run 'Summarize the audit architecture' --depth quick --source repo
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- research list
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- telemetry runs
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- telemetry metrics
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- skills list
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- run --skill coding 'Implement the scoped change'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask skills scaffold my-skill 'My data-only skill'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- skills validate path/to/local-skill --local
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask skills install path/to/local-skill
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask integrations import-openapi demo openapi.json --base-url https://api.example.test --credential-reference env:DEMO_API_TOKEN
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- integrations list
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask integrations connect github --credential-reference env:GITHUB_TOKEN
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask integrations connect searxng --base-url http://127.0.0.1:8888 --auth-type none
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask integrations connect opensearch --base-url http://127.0.0.1:9200 --auth-type none
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- mcp servers
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- mcp tools --server local
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- --approval-mode ask mcp call local search '{"query":"audit"}'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- run --resume 'Continue'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- audit verify
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- policy doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- state doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- sandbox doctor
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- process run /bin/echo --cwd . -- hello
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- projection status
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- projection rebuild
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow validate .colossus/workflows/offline-echo.yaml
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow register .colossus/workflows/offline-echo.yaml
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- workflow run offline-echo 1.0.0 --inputs '{}'
cargo run --manifest-path rust/Cargo.toml -p colossus-cli --bin colossus-rs -- repl
```

`config init` creates a unique platform credential-store identity for that fresh state
file. It neither asks for an application credential nor performs a network request.
`process run` remains deny-by-default: the example requires `process.spawn`, the exact
executable, its execute grant, a working-directory grant, and any environment/network
obligations to be present in the YAML configuration.

## Remaining Delivery Milestones

This alpha is the audit/storage, authorization, and workflow foundation, not the P0+P1
cutover. The following planned work remains:

- Extension repositories plus their shared conformance suites. Canonical research runs,
  sources, claims, citations, phase progress, deterministic/model fallbacks, restart
  interruption, repository collection, and configured SearXNG collection are implemented.
- A durable external-work queue that can advance Tantivy and Chroma independently,
  live Chroma version compatibility, and richer retry/backoff telemetry. Selectable
  Chroma semantic candidates plus local/OpenAI-compatible embedding profiles are
  implemented; Tantivy remains the offline default and canonical re-filtering remains
  mandatory.
- Podman revalidation of the new proxy-only network path and Windows filesystem/network
  runtime acceptance. Native macOS/Linux isolation, live Docker execution/recovery, OCI
  command and allowlist-proxy hardening, the native allowlist proxy, authenticated
  helper, explicit broker downgrade rules, resource supervision, and native/OCI escape
  tests are implemented. CI compiles every
  Rust target on macOS and Windows while unsupported Windows execution remains fail-closed.
- Durable Plan Mode, single-use approval, plan-to-goal handoff, bounded Goal Mode, and
  durable subagents are implemented. The durable memory, task/decision, and context
  budget/snapshot boundaries, durable multi-turn loop, bounded malformed-tool recovery,
  strict catalog validation, pure echo,
  permit-bound file list/read/search/write/replace, Git inspection, structured shell
  execution, and permit-bound HTTP GET are implemented.
- Full worker-backed REPL slash-command parity and live Windows named-pipe acceptance.
- Packs, offline bundles, additional MCP transports/conformance coverage, and the rest of P1/P2. Native GitHub,
  SearXNG, and OpenSearch tools plus event-sourced OpenAPI imports, strict dynamic schemas,
  pending-auth hiding, environment credential brokering, exact-origin HTTP execution,
  permit/approval enforcement, bounded quarantine, and post-effect release are implemented.
  Declarative discovery, deterministic composition, active-scoped resources, optimistic
  authoring, workspace-local validation, and approval-gated skill installation are
  implemented; skill resources remain non-executable. Configured stdio MCP discovery,
  invocation, and research collection are implemented through the normal sandbox and
  gateway boundaries.
- Fuzzing, dependency/license/vulnerability policy, the full Windows/Linux sandbox
  runtime matrix, and
  six-target release smoke tests.

Rust is promoted to the repository root only after those P0+P1 acceptance checks pass.
