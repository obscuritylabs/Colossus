# Rust Foundational Acceptance Matrix

This matrix tracks the new reconstruction requirements separately from the full P0-P2
product checklist. `Passing` means an executable test exists in the current Rust alpha;
`Partial` means the contract/foundation exists but later adapters or platform suites are
still required.

| Requirement | Status | Current evidence | Remaining acceptance |
| --- | --- | --- | --- |
| AUDIT-01 | Passing foundation | redb encryption/round-trip, optimistic conflict, concurrent append, rotation, tampering, signed checkpoint, secure-anchor truncation, projection-outbox, recovery-mode, and unknown-effect tests | Crash fault injection, external/WORM exporter, OS key-provider matrix, fuzzing |
| AUTHZ-01 | Passing native and Docker OCI foundation | compile-fail permit forgery; deny-before-adapter; approval re-evaluation; hard-redaction; oversized-input; post-effect non-release; permit-bound filesystem/process/HTTP/provider/embedding/Chroma adapters; provider credential resolution only after allow; authenticated/expiring helper jobs; Seatbelt/Landlock isolation; native and OCI environment clearing; symlink/traversal, process-tree, timeout, output, exact-origin proxy with HTTP Host/TLS SNI checks, DNS/address pinning, denied content, immutable workload/proxy images, read-only roots, network-none and proxy-only networking, direct-egress denial, cancellation cleanup, and unknown-outcome tests against Docker; the same live suite is wired for Podman in Linux CI; real OPA allow/deny/approval/post-release/invalid/outage/readiness plus pinned-CA mTLS | Podman proxy-path revalidation and live Windows isolation matrix |
| TOOL-01 | Passing P0 catalog and presentation foundation | strict configured catalog; schema-before-policy validation; complete offline filesystem, Git, shell, user prompt, work, plan/goal, patch, repository, subagent, discovery, trace, context, skill/resource, and echo tools; configured MCP calls; exact-origin `web.fetch`/`docs.fetch`; permit-bound repository/patch/context/export effects; metadata-only trace views; interactive-only `user.ask`; event-sourced policy-authorized REPL preferences; correlated durable run/tool events; compact/verbose/off semantic file, shell, Git, work, context, repo, skill, web, MCP, trace, integration, pack, and generic rendering; prompt-safe TTY activity refresh with escape-free redirected output; cached embedded/worker session, provider, context, work, approval, preference, and last-run prompt status; recoverability and elapsed phase/action output with worker parity; traversal, control-state, symlink, binary, ambiguity, scope, approval, and post-release tests | Persistent history, full theme palettes, editor-buffer counters, and broader live endpoint/tool-use acceptance |
| STORE-01 | Passing embedded, worker, and memory-index foundation | split ports; in-memory and redb journal/projection conformance; atomic outbox; exclusive writer lease; restart catch-up; optimistic positions; deterministic rebuild; startup position validation; authenticated local worker IPC v2 with pre-disclosure handshake, replay protection, correlated streamed run envelopes, typed routing for every top-level CLI runtime operation, worker/embedded implemented REPL-command parity with scripted-stdin acceptance, serialized projection/index/subagent maintenance, clean shutdown, and embedded fallback; session/work/memory/workflow reducers; canonical session/message and memory repositories; Tantivy and permit-bound Chroma projection conformance | Live Windows named-pipe acceptance, research/extension repositories, queued external index/export work, and additional adapter conformance suites |
| MEM-01 | Passing lexical and semantic foundation | canonical create/archive/supersede replay; Tantivy event-id idempotency, search, removal, status, rebuild, and degraded fallback; selectable Chroma v2 candidate projection; durable local Chroma replay position and unknown-outcome retry block; explicit rebuild recovery; deterministic local and strict OpenAI-compatible embedding profiles; exact-origin gateway transport; deny-before-network tests; canonical scope/status/expiry re-filtering | Durable external-work queue across independent lexical and semantic projections, live Chroma compatibility matrix, and backoff/readiness telemetry |
| FLOW-01 | Passing durable foundation | strict YAML and exact hash invalidation; condition grammar; direct and indirect call-cycle rejection; 16-level call-depth enforcement; journal-native queued/claimed/drained runs; pinned and policy-authorized linked child runs with parent/step/depth projections, visible waiting-child identity, duplicate-free resume, intent-to-queue crash repair, terminal propagation, and cancellation cascade; durable output and attempt-budget reconstruction; root, nested, and iteration-scoped wait/input resume; scoped effect, retry, idempotency, parallel-branch, and child-call identities; bounded parallel journal writes; explicit idempotent retry; separately dispatched compensation effects; abandoned-attempt `outcome_unknown` recovery with non-idempotent retry refusal; authenticated worker validate/register/start/status/resume/input/cancel/drain routing | Broader process-kill fault injection |
| PROV-01 | Passing P0 provider foundation | strict echo/Responses/compatible profiles; role routing; one-shot CLI and REPL path; incremental Responses and compatible SSE; per-item quarantine and post-effect release; durable partial streams; normalized usage telemetry; model catalog/doctor; full logical request disclosure; reference-only credential policy input; permit-bound late credential resolution; safe reasoning summaries; durable multi-turn continuation; call-ID-correlated tool results; strict pre-policy schemas; two-attempt malformed-argument recovery; explicit max-turn exhaustion; complete required P0 tool catalog | Broader compatible-endpoint acceptance matrix |
| STATE-01 | Passing session, context, and work-state foundation | canonical session creation and append-only messages; optimistic versions; reconstructed summaries/history; newest/latest/exact resume; session id in run/effect provenance and results; provider history restoration; separate-process CLI restart acceptance; numbered REPL picker; automatic/manual context compaction, immutable snapshots, restore, runtime-injected session scope, policy/approval enforcement, and provenance tests; bounded session work-state aggregation for tasks, active decisions, actionable plans, current goals, and nonterminal subagents through embedded CLI/REPL and authenticated worker | Migration/cutover UX |

The relevant tests live in:

- `rust/crates/colossus-journal-redb/src/lib.rs`
- `rust/crates/colossus-agent/src/lib.rs`
- `rust/crates/colossus-memory/src/lib.rs`
- `rust/crates/colossus-memory-chroma/src/lib.rs`
- `rust/crates/colossus-policy/src/lib.rs`
- `rust/crates/colossus-policy/tests/opa_live.rs`
- `rust/crates/colossus-provider/src/lib.rs`
- `rust/crates/colossus-tools/src/lib.rs`
- `rust/crates/colossus-projection/src/lib.rs`
- `rust/crates/colossus-presentation/src/lib.rs`
- `rust/crates/colossus-runtime/src/lib.rs`
- `rust/crates/colossus-sandbox/src/lib.rs`
- `rust/crates/colossus-session/src/lib.rs`
- `rust/crates/colossus-cli/tests/agent_smoke.rs`
- `rust/crates/colossus-cli/tests/native_sandbox.rs`
- `rust/crates/colossus-cli/tests/oci_sandbox.rs`
- `rust/crates/colossus-workflow/src/lib.rs`
- `rust/crates/colossus-testkit/src/lib.rs`
