# Rust Foundational Acceptance Matrix

This matrix tracks the new reconstruction requirements separately from the full P0-P2
product checklist. `Passing` means an executable test exists in the current Rust alpha;
`Partial` means the contract/foundation exists but later adapters or platform suites are
still required.

| Requirement | Status | Current evidence | Remaining acceptance |
| --- | --- | --- | --- |
| AUDIT-01 | Passing foundation | redb encryption/round-trip, optimistic conflict, concurrent append, rotation, tampering, signed checkpoint, secure-anchor truncation, projection-outbox, recovery-mode, and unknown-effect tests | Crash fault injection, external/WORM exporter, OS key-provider matrix, fuzzing |
| AUTHZ-01 | Passing foundation | compile-fail permit forgery test; deny-before-adapter, approval re-evaluation, hard-redaction, oversized-input, and post-effect non-release tests | Real filesystem/network/provider/memory adapters, sandbox IPC authentication, OPA live/mTLS integration suite |
| STORE-01 | Partial | split ports, in-memory journal fixture, redb shared conformance test, atomic outbox, canonical memory repository | Other aggregate repositories, projection worker, Chroma and all adapter conformance suites |
| MEM-01 | Passing offline foundation | canonical create/archive/supersede replay plus Tantivy event-id idempotency, candidate search, removal, status, and rebuild tests | Chroma, embedding profiles, queued lag/retry, gateway transport, and canonical policy re-filtering |
| FLOW-01 | Passing foundation | strict YAML, exact hash invalidation, condition grammar, direct cycle rejection, durable run reconstruction, wait/input resume, bounded parallel journal writes, interruption rules | Cross-workflow cycle/depth enforcement, compensation, worker IPC, queued triggers, crash fault injection |

The relevant tests live in:

- `rust/crates/colossus-journal-redb/src/lib.rs`
- `rust/crates/colossus-memory/src/lib.rs`
- `rust/crates/colossus-policy/src/lib.rs`
- `rust/crates/colossus-runtime/src/lib.rs`
- `rust/crates/colossus-workflow/src/lib.rs`
- `rust/crates/colossus-testkit/src/lib.rs`
