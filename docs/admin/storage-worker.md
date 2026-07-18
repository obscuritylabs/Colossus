---
title: Storage and worker
description: Operate the canonical journal, projections, and authenticated local worker.
audience: operator
type: concept
---

# Storage and worker

The encrypted event journal is canonical. Session transcripts, work records, workflow
runs, approvals, and effect evidence are reconstructed from append-only events.
Projections, memory indexes, and exports are disposable consumers, not alternate write
models.

## Storage adapters

When `storage.adapter` is omitted, Colossus uses embedded redb. It is the simplest
single-host choice and holds the local writer lease. A multi-process deployment may
select PostgreSQL:

```yaml
storage:
  path: .colossus/instance.redb
  adapter: postgres
  postgres:
    connectionVariable: COLOSSUS_DATABASE_URL
    schema: colossus_production
    tls:
      kind: webpki_roots
    statementTimeoutMs: 30000
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
```

`storage.path` remains the local instance and worker-IPC identity. Changing the adapter
does not import, merge, or delete another journal. Provision and verify a new target as
a separate reviewed operation.

Journal and signing keys are independent 32-byte values managed by the platform
credential service or injected environment variables. There is no plaintext fallback.
The secure anchor is protected separately from the journal.

## Worker ownership

Only one redb writer lease is allowed. A long-running worker can own it and expose an
authenticated local application protocol to CLI and TUI clients:

```bash
colossus --config .colossus/config.yaml worker
colossus --config .colossus/config.yaml worker --status
colossus --config .colossus/config.yaml worker --once
colossus --config .colossus/config.yaml worker --shutdown
```

The worker drains queued workflows and child jobs, evaluates triggers, coordinates
projection and index maintenance, and serves the same application contracts used by
embedded operation. It does not move policy, provider, or repository logic into IPC.

If no endpoint exists, a one-shot command may safely use the embedded runtime. A wrong,
stale, replayed, malformed, or incorrectly permissioned endpoint fails authentication;
it never authorizes a second embedded writer.

## Projection operations

Use status and drain during normal operation. Rebuild only a named disposable
projection after preserving and verifying canonical state:

```bash
colossus --config .colossus/config.yaml projection status
colossus --config .colossus/config.yaml state doctor
```

See [State and recovery](../develop/state-recovery.md) for the implementation contract
and [Audit, telemetry, and recovery](audit-telemetry-recovery.md) for incident handling.
