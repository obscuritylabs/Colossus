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
and local metadata atomically. PostgreSQL locks the chain-head row for one runtime namespace and commits
the equivalent records in one transaction. Every event, stream, outbox, projection and
metadata key carries that namespace; the default empty namespace preserves existing
standalone journals. A hosted execution fence checks its lease generation inside each
write transaction, so an expired or superseded owner cannot commit canonical state.

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

Shared hosted runtimes receive explicit SQL storage, stable namespace identity and an
execution fence before opening. They derive journal/signing keys from separate external
masters and retain secure anchors in an independently administered HTTPS authority with
its own encrypted PostgreSQL database. Runtime SQL rollback cannot roll that authority
back. A stale owner cannot release effect output; unresolved started effects remain
unknown after takeover. The new owner rebuilds its local lexical cache from the durable
projection queue. Native local runtime composition retains its existing protected
storage and local ownership contract.

## Derived state

- Session discovery, work lists, and other projections are rebuildable read models.
- Session discovery reads ordered active and archived summaries from
  `session-catalog-v1`. The same projection maps message sequences to positions in
  canonical session streams. A transcript page reads a bounded range of positions
  and decrypts the messages it needs, including one lookahead at the page boundary.
- Tantivy stores disposable memory candidate projections, not lifecycle truth.
- Audit exporters consume a durable outbox and expose ciphertext-free evidence.
- Each external-work consumer has an independent optimistic position and retry state.
- `effects-recovery-v1` is a disposable operational view of effects with a durable
  start and no terminal outcome. Its cursor or record set is not recovery authority.
  Startup instead enumerates the journal's indexed `effect:` streams, derives pending
  lifecycles from their canonical tails, and recovers at most 1,024 effects. It never
  scans unrelated global history to infer uncertain effects.

Rebuild replays canonical events. It never imports an unrelated store or turns a
projection into authority.

Existing journals build the catalog from their durable outbox during background
maintenance. Until it is current, discovery reads pages of the existing session
summary projection and checks canonical lifecycle tails for archive membership.
It keeps only the requested newest summaries in memory. This compatibility path
uses a human preview when the ordered catalog has processed every message for
that session. Older summary rows lack the provenance needed to use their preview.
Reads can advance the events committed since open. Writes also make bounded
progress on small carried-over tails in a populated catalog. Background
maintenance owns the older catalog backlog. Opening a known conversation can
use small reverse stream reads while the catalog catches up. An older cursor
may require reading past newer messages during this transition.

Once the catalog is current, recent, archived, and latest queries seek directly
to bounded summary rows, and older pages seek directly to their cursor. Catalog
records carry session titles and bounded scalar human-message previews.
Multipart prompt text is available from the canonical transcript.

### Unfinished provider exchanges

Native Messages replay is required state for an unfinished tool exchange. After
provider output passes post-effect release, its private candidate and the pending
tool-turn marker commit together before tool effects begin. The complete assistant
and tool-observation messages, marker completion, and replay settlement then commit
in one journal batch. A failed provider request or cancellation preserves replay
required by committed history.

The private checkpoint retains the prepared request with image artifact references,
never resolved image bytes. Same-session continuation keeps that exact prefix and
appends human guidance without retrieving a new memory prefix or archiving its tool
observations. Route, tool, canonical-history and snapshot bindings remain checked.
Protected journals retain both admitted and settled candidates across restart;
keyless custody remains bounded process memory. When a keyless runtime has lost its
checkpoint, an ordinary Execute continuation with a valid committed transcript can
prepare a fresh response from saved evidence. Tool calls and results appear as
untrusted text in the recovery request. The request offers no tools, and recovery
context compaction is transient. Ordinary Execute can also use this path when exact
replay passes its binding checks but exceeds a hard context budget. The canonical
transcript and any active snapshot retain their original authority. A pending tool
turn still requires reconciliation from effect evidence before provider dispatch.

Normal loads respect replay retirement. Recovery of an unfinished legacy exchange
may inspect the immediately preceding protected settled record and restore it only
after canonical and provider bindings validate. It never fabricates private blocks
or repeats tool execution.

## Process ownership

File-backed redb permits one writer lease. The worker may own that lease and coordinate
projection, index, trigger, workflow, and child-job drains. Embedded operation is
available only when no valid worker endpoint owns the instance. Ephemeral redb has no
writer lease and intentionally provides no recovery across process exit; it is for
fresh one-shot runs, not durable workers or retryable effects.

### Managed shell sessions

`process-session:<id>` streams record safe ownership and lifecycle metadata. The latest
`process-session-catalog` event indexes at most 256 retained sessions; startup reads
that bounded index and individual canonical tails. Running or starting sessions become
`interrupted` on recovery, with no successful exit code, PID adoption, or automatic
command replay. Released logs are retained only in a bounded runtime buffer; the
session read API reports a gap after restart. Quarantined output never enters this
read model.

A runtime task owns the supervisor, isolated directory, and workspace lease through
cleanup. Run completion stops run-owned and unlaunched sessions; explicitly launched
workspace sessions remain supervised until their original deadline or runtime shutdown.

## Human terminals

Human PTYs are process-local, independently supervised resources, not durable agent
runs or managed `shell.run` sessions. Browser/connector disconnects detach their readers;
bounded host leases retain the same process. Output frames and resize events have
monotonic cursors and a bounded replay window. A fresh emulator cannot reconstruct a
screen from an arbitrary retained suffix; missing history is explicit. An input receipt
lost after dispatch is uncertain and must never trigger automatic command replay.
Worker restart ends sessions and changes the incarnation. Colossus never adopts a
stored PID. [ADR 0011](adr/0011-fleet-human-terminals.md) owns this contract.

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

Port compatibility is executable. In-memory, redb, PostgreSQL, SQLite, and embedded
Turso implementations run shared conformance suites for optimistic append, replay,
outbox order, and projection behavior. File-backed tests reopen journals and interrupt
operations around commit or acknowledgement to establish rollback, durable visibility,
or explicit uncertainty.

## Embedded SQL adapter evaluation

`colossus-journal-sql` opens a file-backed journal and projection store with bundled
SQLite or the local Turso engine. Both use one writer lease, WAL mode, and full
synchronization. The caller supplies an owner-private directory and journal-specific
keys and anchor custody. The engine and protection mode are fixed at initialization.

For protected stores, the signed checkpoint commits in SQL before the independent
anchor advances. If the process stops between these writes, startup verifies the
earlier anchor, the intervening journal tail, and the durable checkpoint before
advancing the anchor. A failure to confirm the anchor update after committing a
checkpoint enters read-only recovery.

From the repository root, run the example once per engine against an existing scratch
directory with several GB free. Substitute `redb` or `turso` for `sqlite` to compare.

```bash
cargo run --release --locked -p colossus-journal-sql --example journal_benchmark -- sqlite /path/to/scratch
```

The default workload seeds 900,000,000 bytes of synthetic canonical JSON across
1,024 streams with protected payloads and batches of 128 events. It reports separate
measurements for ingest, steady-state single appends, indexed tail reads, global and
projection replay, incremental reopen, full audit, and disk size. Each run creates
and removes its own database under the scratch directory. Run engines sequentially
in rotated order and keep the scratch volume and build profile fixed.

### Session catalog read budget

The same acceptance workload exercises redb, SQLite, and embedded Turso against
independent encrypted journals reopened with current projections. It checks recent
ordering, archive filtering, and latest-session selection through the session
repository. All three queries must perform zero session stream reads and decrypt
zero message payloads. Newest and older transcript pages must use bounded journal
reads and decrypt at most one more message than the requested page size.

```bash
cargo xtask quick test -p colossus-journal-sql -- --test session_catalog -- --nocapture --test-threads=3
```

### Time to the terminal prompt

Build the CLI and benchmark example from the same source and release profile. Pass
the path to the built CLI and an existing private scratch directory with several GB
of free space. Add `.exe` to the CLI path on Windows.

```bash
cargo build --release --locked -p colossus-cli --features journal-startup-benchmark --bin colossus --example journal_startup_benchmark
cargo run --release --locked -p colossus-cli --features journal-startup-benchmark --example journal_startup_benchmark -- measure target/release/colossus /path/to/scratch
```

The benchmark writes 900,000,000 canonical JSON bytes in 1,024 saved conversations
per engine. It catches up the runtime projections, signs each journal's checkpoint,
and verifies the stored corpus before timing. It starts a new embedded `colossus tui`
process for each sample and times the interval from launch until the terminal renders
the composer. The `--resume` sample also waits for the latest saved transcript. Each
launch checks that the composer accepts a draft, then exits without submitting it.

The JSON Lines output records one warm-up and nine measured trials per engine and
launch mode by default. It includes a fresh prompt with the welcome animation, a
fresh prompt with that animation disabled, and a prompt after `--resume`. The
benchmark rotates engine order and leaves operating-system caches in place. Use
`--bytes`, `--sessions`, and `--trials` for shorter runs. The opt-in feature makes
the two SQL adapters available to this source benchmark on private scratch state.
