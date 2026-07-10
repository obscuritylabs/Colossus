# Rust Foundational Acceptance Matrix

This matrix tracks the new reconstruction requirements separately from the full P0-P2
product checklist. `Passing` means an executable test exists in the current Rust alpha;
`Partial` means the contract/foundation exists but later adapters or platform suites are
still required.

| Requirement | Status | Current evidence | Remaining acceptance |
| --- | --- | --- | --- |
| AUDIT-01 | Passing foundation | redb encryption/round-trip, optimistic conflict, concurrent append, rotation, tampering, signed checkpoint, secure-anchor truncation, projection-outbox, recovery-mode, and unknown-effect tests | Crash fault injection, external/WORM exporter, OS key-provider matrix, fuzzing |
| AUTHZ-01 | Passing native and Docker OCI foundation | compile-fail permit forgery; deny-before-adapter; approval re-evaluation; hard-redaction; oversized-input; post-effect non-release; permit-bound filesystem/process/HTTP/provider adapters; provider credential resolution only after allow; authenticated/expiring helper jobs; Seatbelt/Landlock isolation; native and OCI environment clearing; symlink/traversal, process-tree, timeout, output, exact-origin proxy with HTTP Host/TLS SNI checks, DNS/address pinning, denied content, immutable workload/proxy images, read-only roots, network-none and proxy-only networking, direct-egress denial, cancellation cleanup, and unknown-outcome tests against Docker; the same live suite is wired for Podman in Linux CI; real OPA allow/deny/approval/post-release/invalid/outage/readiness plus pinned-CA mTLS | Memory and embedding adapters, Podman proxy-path revalidation, live Windows isolation matrix |
| STORE-01 | Passing embedded foundation | split ports; in-memory and redb journal/projection conformance; atomic outbox; exclusive writer lease; restart catch-up; optimistic positions; deterministic rebuild; startup position validation; session/work/memory/workflow reducers; canonical memory repository | Research/extension repositories, Chroma, queued external index/export work, worker IPC, and additional adapter conformance suites |
| MEM-01 | Passing offline foundation | canonical create/archive/supersede replay plus Tantivy event-id idempotency, candidate search, removal, status, and rebuild tests | Chroma, embedding profiles, queued lag/retry, gateway transport, and canonical policy re-filtering |
| FLOW-01 | Passing foundation | strict YAML, exact hash invalidation, condition grammar, direct cycle rejection, durable run reconstruction, wait/input resume, bounded parallel journal writes, interruption rules | Cross-workflow cycle/depth enforcement, compensation, worker IPC, queued triggers, crash fault injection |
| PROV-01 | Partial P0 slice | strict echo/Responses/compatible profiles; role routing; one-shot CLI and REPL path; model catalog/doctor; full logical request disclosure; reference-only credential policy input; permit-bound late credential resolution; bounded quarantined normalization; strict tool-argument parsing; safe reasoning summaries; audited typed events | Incremental streaming, multi-turn tool execution/recovery, usage accounting, provider retry classification, broader compatible-endpoint acceptance |

The relevant tests live in:

- `rust/crates/colossus-journal-redb/src/lib.rs`
- `rust/crates/colossus-memory/src/lib.rs`
- `rust/crates/colossus-policy/src/lib.rs`
- `rust/crates/colossus-policy/tests/opa_live.rs`
- `rust/crates/colossus-provider/src/lib.rs`
- `rust/crates/colossus-projection/src/lib.rs`
- `rust/crates/colossus-runtime/src/lib.rs`
- `rust/crates/colossus-sandbox/src/lib.rs`
- `rust/crates/colossus-cli/tests/native_sandbox.rs`
- `rust/crates/colossus-cli/tests/oci_sandbox.rs`
- `rust/crates/colossus-workflow/src/lib.rs`
- `rust/crates/colossus-testkit/src/lib.rs`
