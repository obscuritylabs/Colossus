# Colossus public API

This directory is the source of truth for the public, language-neutral Colossus
application API. The initial package is deliberately unstable:
`colossus.api.v1alpha1`.

The contract is separate from the local worker protocol. Public clients must not
depend on `WorkerOperation`, submit actor identities, pass server-local paths, or
receive unreleased effect output. Authentication, caller attribution, scope checks,
policy, permits, quarantine, and audit are server responsibilities outside the
Protobuf messages.

Compatibility rules:

- Add fields and RPCs; do not reuse field numbers or enum values.
- Reserve removed names and numbers.
- Keep every enum zero value as `*_UNSPECIFIED`.
- Treat IDs, page tokens, etags, idempotency keys, and interaction tokens as opaque.
- Bound every scalar, collection, message, and stream at the server boundary.
- Put `ColossusErrorDetail` in the standard gRPC status-details envelope; never put
  secrets, quarantined bytes, hidden reasoning, or raw internal errors in status
  messages or details.
- `WatchRun.after_sequence` is an exclusive cursor. Delivery is at-least-once and
  clients deduplicate by `(run_id, sequence)`.

Run `buf lint` and `buf build` from this directory. CI runs a local, pinned Buf
breaking check against the pull-request, merge-queue, or push base once that base
contains the API module. The first change that establishes the API is the baseline;
later changes cannot silently skip compatibility validation. After installing the
pinned SDK tools, run a local check from the repository root:

```console
./sdk/scripts/check-breaking main
```
