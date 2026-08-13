---
title: State and recovery
description: Canonical journal, projections, indexes, checkpoints, worker lease, and unknown-outcome recovery invariants.
audience: developer
type: concept
---

# State and recovery

The journal is authoritative. Application aggregates reconstruct from immutable
events. Protected storage encrypts event payloads; keyless storage keeps canonical
plaintext while retaining payload hashes and the record chain. Projections, indexes,
and exported evidence can improve discovery or operations, but they never replace
canonical history.

## Canonical append

Each event has stream identity/version, global sequence, actor and lineage, payload
protection metadata, plaintext hash, previous chain hash, and current chain hash. Appends
optimistically validate stream versions. redb commits events, chain head, outbox work,
and local metadata atomically. PostgreSQL locks a singleton chain-head row and commits
the equivalent records in one transaction.

Protected journals use authenticated encryption, signed checkpoints, and a separately
protected secure anchor to detect record mutation and consistent tail truncation.
Keyless journals store plaintext payloads and disable signed checkpoints and the secure
anchor while retaining payload and record-chain verification. With protected storage,
startup defaults to incremental verification: a version-two anchor attests a previously
verified prefix, startup authenticates the checkpoint boundary, and then verifies only
the contiguous tail. A legacy, absent, quarantined, or incompatible anchor triggers one
complete bootstrap replay before a new attestation is trusted. Keyless incremental
startup instead checks bounded local head, hash, index, outbox, and projection
relationships without replaying historical payloads. Full startup mode and
`audit verify` replay every event. Corruption in a protected journal's anchored prefix is
detected when that record is decoded and verified and by every explicit full audit;
deterministic failures quarantine the anchor and make the runtime read-only. Startup
repairs only narrowly defined interrupted checkpoint metadata when the complete journal
still proves the advanced anchor safe.

## Derived state

- Session discovery, work lists, and other projections are rebuildable read models.
- Canonical session messages are reconstructed from their streams.
- Tantivy and optional Chroma store memory candidate projections, not lifecycle truth.
- Audit exporters consume a durable outbox and expose ciphertext-free evidence.
- Each external-work consumer has an independent optimistic position and retry state.
- `effects-recovery-v1` is a disposable operational view of effects with a durable
  start and no terminal outcome. Its cursor or record set is not recovery authority.
  Startup instead enumerates the journal's indexed `effect:` streams, derives pending
  lifecycles from their canonical tails, and recovers at most 1,024 effects. It never
  scans unrelated global history to infer uncertain effects.

Rebuild replays canonical events. It never imports an unrelated store or turns a
projection into authority.

## Process ownership

redb permits one writer lease. The worker may own that lease and coordinate projection,
index, trigger, workflow, and child-job drains. Embedded operation is available only
when no valid worker endpoint owns the instance.

## Recovery states

| Condition | Required behavior |
| --- | --- |
| Verification succeeds | Normal reads and effects |
| Chain, checkpoint, anchor, or decryption failure | Read-only recovery; effects blocked |
| Projection behind | Drain from canonical outbox |
| Disposable projection corrupt | Preserve journal, then explicit reset/rebuild |
| Consumer known retryable failure | Durable bounded backoff |
| Consumer unknown delivery | Block implicit retry pending operator reconciliation |
| Effect started without terminal event | Record unknown outcome |
| Workflow interrupted before effect | Reconstruct and resume from durable step state |
| Workflow uncertain after effect | Resume only with exact idempotency or explicit recovery |
| Running child found at startup | Mark interrupted; never silently rerun |

Recovery code must distinguish a known failure from uncertainty. “Try again” is not a
safe default after an external effect may have escaped.

## Test expectation

Port compatibility is executable. In-memory, redb, and PostgreSQL implementations run
shared conformance suites for optimistic append, replay, reopening, outbox order, and
projection behavior. Fault tests terminate operations around commit and acknowledgement
boundaries to prove rollback, durable visibility, or explicit uncertainty.
