---
title: State and recovery
description: Canonical journal, projections, indexes, checkpoints, worker lease, and unknown-outcome recovery invariants.
audience: developer
type: concept
---

# State and recovery

The journal is authoritative. Application aggregates reconstruct from immutable,
encrypted events. Projections, indexes, and exported evidence can improve discovery or
operations, but they never replace canonical history.

## Canonical append

Each event has stream identity/version, global sequence, actor and lineage, encrypted
payload metadata, plaintext hash, previous chain hash, and current chain hash. Appends
optimistically validate stream versions. redb commits events, chain head, outbox work,
and local metadata atomically. PostgreSQL locks a singleton chain-head row and commits
the equivalent records in one transaction.

Journal payloads use authenticated encryption. Signed checkpoints and a separately
protected secure anchor detect record mutation and consistent tail truncation. Startup
verifies the chain and repairs only narrowly defined interrupted checkpoint metadata
when the anchored journal head proves it safe.

## Derived state

- Session discovery, work lists, and other projections are rebuildable read models.
- Canonical session messages are reconstructed from their streams.
- Tantivy and optional Chroma store memory candidate projections, not lifecycle truth.
- Audit exporters consume a durable outbox and expose ciphertext-free evidence.
- Each external-work consumer has an independent optimistic position and retry state.

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
