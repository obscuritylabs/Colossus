---
status: current
replacement:
  - /develop/setup-testing/
  - /reference/
---

# Rust Acceptance Matrix

This is a compact evidence index for the active Rust runtime. The normative product
contract remains [Feature Inventory](feature-inventory.md); current security architecture
is documented in
[Security architecture](../../docs/develop/security-architecture.md). Detailed behavior
belongs in tests, not duplicated prose.

| Requirement | Status | Executable evidence | Remaining scope |
| --- | --- | --- | --- |
| AUDIT-01 | Passing | encrypted redb/PostgreSQL conformance; concurrency, outage, and crash injection; hash/checkpoint/anchor tamper detection; key rotation; unknown-effect recovery; projection and directory/WORM audit-export replay | broader OS key-provider matrix |
| AUTHZ-01 | Passing foundation | deny-before-adapter and unforgeable one-use permit tests across every effect family; two-phase release denial; OPA allow/deny/approval/outage/mTLS; native and OCI escape suites; workflow-scoped development-grant removal; `risk-auto` shell-only low/allow proof plus workflow/failure fallback; shared public-wildcard/private-origin rejection; green 0.8.1 release-platform rerun | future policy adapters |
| STORE-01 | Passing | shared in-memory/redb/PostgreSQL journal, repository, projection, and outbox suites; local writer lease or database transaction ownership; authenticated worker IPC; Tantivy and permit-bound Chroma conformance; audit-export retry/recovery | additional adapters |
| MEM-01 | Passing P1 | canonical lifecycle and scope filtering; Tantivy rebuild/fallback; Chroma candidate projection; local and OpenAI-compatible embeddings; lag, backoff, and unknown-outcome recovery | broader hosted Chroma/version coverage |
| FLOW-01 | Passing P1 + triggers | strict YAML/hash trust; bounded control flow; waits; idempotent retry; compensation; subworkflows; cancellation; fixed-cadence schedule misfire/trust/restart reconstruction; HMAC webhook authentication/replay/size/trust rejection; exact domain-event subscriptions with durable checkpoints and duplicate acknowledgement; atomic deterministic trigger dispatch under process kill; policy and worker/embedded routing | future trigger adapters |
| PROV-01 | Passing P0 | echo, Responses, and compatible adapters; normalized streaming/tool/usage contracts; post-release gating; late credential resolution; malformed-call recovery; CLI/TUI/worker continuation tests | broader hosted provider/version coverage |
| ACCESS-01 | Passing P0 | four metadata-driven profiles; exact and wildcard tool selection; non-overlapping action overrides; OPA ownership; prerequisite diagnostics; future core/integration/pack inheritance; unclassified and removed-field fail-closed coverage; credential-free effective output with canonical workspace and derived/explicit resources | future extension source classes |
| TOOL-01 | Passing P1 | access-resolved catalog; schema-before-policy rejection; filesystem, Git, command/argv shell isolation, workspace/control-state protection, bounded wildcard proxy evidence, work, plan/goal, patch, repo, subagent, context, skill, MCP, integration, and presentation acceptance | broader live Windows/OCI development-shell matrix |
| AGENT-01 | Passing P1 | durable lifecycle and recovery; configured concurrency; recursive-delegation denial; foreground scheduler wake; same-turn parent result acceptance | future distributed schedulers |
| UX-02 | Passing P1 | frozen Python 0.5 parity inventory; bounded presentation documents; terminal Markdown, semantic cards, source/process/diff previews, human list/detail tables, grouped stateful help, slash/theme/`@skill` completion, guided session and theme choices, five-theme visual snapshots, output-mode and embedded/worker acceptance | additional semantic presentation adapters as new features are added |
| UX-03 | Passing P1 | Ratatui terminal ownership; durable paged transcript and semantic-document reflow; pinned composer/footer; reducer and five-size/five-theme `TestBackend` suites; PTY history-preservation, resize, inline, and terminal-restoration regressions; embedded/worker host parity; authenticated worker-v4 approval, input, and cancellation frames; cooperative cancellation gates; green 0.8.1 release-platform rerun | future terminal adapters |
| DIST-01 | Passing P2 | clean-prefix native installers; signed bundle trust/tamper/reproducibility tests; signed collection reproducibility, dependency ordering, no-clobber batch lifecycle, tamper and hostile-archive rejection; authenticated loopback registry push/pull through the effect gateway; installed echo and encrypted audit smoke without Cargo, credentials, or network; green six-target `v0.8.1` release run | hosted registry compatibility coverage |
| CUTOVER-DOC | Passing | Rust-only root/package/CLI/config/state contract; published examples parsed by executable documentation tests | keep examples synchronized |

Primary evidence locations:

- `crates/colossus-policy/src/lib.rs` and `crates/colossus-policy/tests/opa_live.rs`
- `crates/colossus-journal-redb/src/lib.rs`,
  `crates/colossus-journal-postgres/src/lib.rs`, and `crates/colossus-audit/src/lib.rs`
- `crates/colossus-runtime/src/lib.rs`, `crates/colossus-agent/src/lib.rs`, and
  `crates/colossus-workflow/src/lib.rs`
- `crates/colossus-sandbox/src/lib.rs` and `crates/colossus-windows-process/src/lib.rs`
- `crates/colossus-memory/src/lib.rs` and `crates/colossus-memory-chroma/src/lib.rs`
- `crates/colossus-cli/tests/` for terminal, worker, provider, sandbox, distribution,
  and documentation acceptance
- `crates/colossus-tui/src/lib.rs` and `crates/colossus-tui/tests/pty_history.rs` for the
  terminal reducer, visual fixtures, transcript preservation, and terminal restoration
- `internal/documentation/terminal-ux-parity.md` for the archived presentation parity
  narrative
- `crates/colossus-fuzzing/src/lib.rs` for stable corpus regression and fuzz targets

The pull-request gate runs the expensive cross-platform/security suites. Ordinary
`main` pushes intentionally run a smaller validation path. Release completion requires
the separate six-target packaging workflow; a skipped or billing-blocked job is never
counted as evidence.
