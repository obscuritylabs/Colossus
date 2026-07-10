# Rust Foundational Acceptance Matrix

This matrix tracks the new reconstruction requirements separately from the full P0-P2
product checklist. `Passing` means an executable test exists in the current Rust alpha;
`Partial` means the contract/foundation exists but later adapters or platform suites are
still required.

| Requirement | Status | Current evidence | Remaining acceptance |
| --- | --- | --- | --- |
| AUDIT-01 | Passing foundation | redb encryption/round-trip, optimistic conflict, concurrent append, rotation, tampering, signed checkpoint, secure-anchor truncation, projection-outbox, recovery-mode, and unknown-effect tests | Crash fault injection, external/WORM exporter, OS key-provider matrix, fuzzing |
| AUTHZ-01 | Passing native foundation | compile-fail permit forgery; deny-before-adapter; approval re-evaluation; hard-redaction; oversized-input; post-effect non-release; permit-bound filesystem/process/HTTP adapters; authenticated/expiring helper jobs; Seatbelt/Landlock isolation; environment clearing; symlink/traversal, process-tree, timeout, output, exact-origin proxy, and denied-content tests; hardened OCI construction | Provider/memory adapters, live OCI and Windows isolation matrices, OPA live/mTLS integration suite |
| STORE-01 | Passing embedded foundation | split ports; in-memory and redb journal/projection conformance; atomic outbox; exclusive writer lease; restart catch-up; optimistic positions; deterministic rebuild; startup position validation; session/work/memory/workflow reducers; canonical memory repository | Research/extension repositories, Chroma, queued external index/export work, worker IPC, and additional adapter conformance suites |
| MEM-01 | Passing offline foundation | canonical create/archive/supersede replay plus Tantivy event-id idempotency, candidate search, removal, status, and rebuild tests | Chroma, embedding profiles, queued lag/retry, gateway transport, and canonical policy re-filtering |
| FLOW-01 | Passing foundation | strict YAML, exact hash invalidation, condition grammar, direct cycle rejection, durable run reconstruction, wait/input resume, bounded parallel journal writes, interruption rules | Cross-workflow cycle/depth enforcement, compensation, worker IPC, queued triggers, crash fault injection |

The relevant tests live in:

- `rust/crates/colossus-journal-redb/src/lib.rs`
- `rust/crates/colossus-memory/src/lib.rs`
- `rust/crates/colossus-policy/src/lib.rs`
- `rust/crates/colossus-projection/src/lib.rs`
- `rust/crates/colossus-runtime/src/lib.rs`
- `rust/crates/colossus-sandbox/src/lib.rs`
- `rust/crates/colossus-cli/tests/native_sandbox.rs`
- `rust/crates/colossus-workflow/src/lib.rs`
- `rust/crates/colossus-testkit/src/lib.rs`
