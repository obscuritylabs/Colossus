# Cloud PostgreSQL adapter

This adapter implements the cloud-owned asynchronous `CloudStore` port with Diesel
and diesel-async. It never implements `EventJournal` and never opens a runtime's
database. Production cloud composition uses this adapter; `MemoryCloudStore` is a
deterministic test fixture.

The domain has separate tables for projects, memberships, hosts, runtime agents,
workspaces, conversations/messages, tasks, commands/receipts, source-session/run
mappings, admission counters, invitations and certificate renewals. Each table has
ordinary typed domain columns, project-scoped keys and indexes. `CloudStore` accepts
typed domain values; the adapter maps them to columns and reconstructs them on reads.
Private Diesel row models such as `ProjectRow` and `UserRow` own explicit Rust field
conversions and native typed parameter bindings. Core reads decode columns directly;
writes bind row fields directly. They do not serialize whole domain objects into JSON
parameters or rebuild JSON objects in SQL. The shared revision/audit/outbox statement
and bounded query clauses remain parameterized SQL executed through Diesel.
Canonical JSON encoding is separate audit-hash input, and serialization of actual
JSONB payload fields stays inside the adapter. Database row models never enter the
domain or the `CloudStore` contract, so another adapter can choose its own representation.
Accounts, identity bindings, memberships, projects, inventory, conversation metadata,
messages, references, admission state, enrollment and settings have no JSONB document
column. Login display metadata lives in child rows, and bounded sets use native arrays.
JSONB remains for versioned SDK requests/snapshots, command operations/replies, runtime
policy observations, released event payloads and opaque authorization envelopes.
Foreign keys are project-scoped and deferred until transaction commit so related
records can be written together.

Entity revisions use compare-and-swap and row locks. Released feeds use independent
stream heads; there is no cloud-wide journal-head lock. Commands, domain mutations,
received events and source cursors commit with audit and delivery-outbox records in
one PostgreSQL transaction. Changed duplicates and stream gaps fail. An uncertain
connection failure preserves `StoreError::OutcomeUnknown` so application code must
reconcile the immutable request identity before retrying.

Lists fetch records and audit metadata in one bounded query. Creation/update times
have indexed `TIMESTAMPTZ` columns and cursor ordering uses timestamp plus identifier.
Exact domain timestamp strings are also retained so reconstruction and audit verification
preserve their original precision. Search, membership, inventory and pending-command
filters use named columns; an absent command reply is SQL `NULL`.
Conversation task queries filter the task's `thread_id`, while message queries use
their thread parent. Source inventory and released events remain caller authorized.

The asynchronous pool defaults to 16 connections, plus one dedicated notification
listener per replica. Pool acquisition, connection, SQL statements and lock waits
are bounded. LISTEN reconnects with a 500 ms to 30 second backoff. Notifications
carry only project identifiers; consumers always reload durable state from their
cursor, including after notification loss or broadcast lag. A transaction commits
its outbox before publishing its notification. Notifications do not constitute
durable delivery acknowledgements.

One pending outbox hint per project coalesces changing entities and streams. Existing
pending hints use conflict-do-nothing, so ordinary events do not serialize on a
shared project row. The listener batches project wakeups every 100 ms, retaining at
most 1,024 distinct pending project identifiers per interval. Missed or coalesced
wakeups are recovered from the durable source cursor.

`CloudStore::maintain` publishes and marks up to 256 pending hints per default pass,
then removes at most 256 expired rows from each operational collection. Published
outbox hints default to one day of retention. Expired browser sessions and OIDC
flows are removed; flow consumption and maintenance counts are audited. Markers
without an expiry, including maintenance metadata, remain. The host runs bounded passes
every minute. Canonical messages, released events, tasks, receipts, cursors and audit
history are never automatically deleted by this janitor.

Human accounts, issuer/subject bindings and local password credentials have dedicated
tables. Local credentials retain salted Argon2id PHC hashes; browser APIs release only
account metadata. Project roles are database authority after the audited one-time
configuration bootstrap. Project parents describe display nesting and never inherit
membership. Archived projects retain read/management access while removing execution,
control and approval permissions. Structural administration transactions prevent
hierarchy cycles and removal of the last active administrator; ordinary runtime/event
transactions do not take that administration lock.

Browser sessions retain cookie/CSRF hashes, stable account identity and a revocation
epoch. Credential rotation and disable/reenable invalidate earlier epochs across
replicas. OIDC flow values must be encrypted by host composition. Shared login budgets
are checked before creating personal attempt counters, so public login traffic cannot
produce unlimited expired-flow metadata. Connection leases use monotonic fencing
generations; runtime transactions check the generation and expiry under the same
transaction's lease-row lock. A stale replica cannot commit runtime output after a
replacement owner acquires its lease. Tombstones retain consumed entity revisions,
preventing a single-use authorization-flow key from being recreated.

## Transport and migration

`CloudDatabaseConfig` references the URL through an environment-variable name. The
adapter never includes URLs or raw PostgreSQL diagnostics in errors. TLS requires
hostname and CA verification using rustls. Custom CA bundles and configured client
identities are supported. Disabling TLS explicitly is restricted to loopback and
Unix-socket acceptance fixtures.

The initial schema is a fresh relational baseline; there is no cloud journal importer.
Disposable development schemas from earlier baselines must be recreated. Runtime
journals use their own adapters and are unaffected by this cloud schema reset.

Numbered SQL migrations are applied transactionally under a schema-specific advisory
lock. Applied SQL checksums are retained and changed migration files fail startup.
This lock is used only during migration, never during normal cloud mutations.
Schema rollback SQL is intentionally destructive and requires an explicit operator
decision; application startup never runs it. Once a cloud schema is deployed, evolve it
with new migrations rather than editing applied SQL.

## Audit boundary

Retained cloud entities and released feeds have per-resource SHA-256 audit chains.
Reads verify the retained content against audit metadata; malformed or changed
content fails closed. Transactionally committed audit entries describe source cursor
advancement. These checks provide tamper evidence within the retained database and
do **not** reproduce the runtime journal's independent secure-anchor rollback
protection. A database administrator can replace or roll back the database and its
hashes together. Production operation needs independently retained audit checkpoints,
restricted database roles, backups and tested restore procedures. Retention must
preserve the chain/checkpoint boundary when pruning historical audit or outbox rows.

## Focused acceptance

Use the explicit disposable PostgreSQL fixture's connection reference:

```sh
cargo test --locked -p colossus-cloud-postgres --lib
cargo test --locked -p colossus-cloud-postgres --lib \
  postgres_atomic_recovery_and_replica_conformance -- --ignored
```

The ignored test requires `COLOSSUS_CLOUD_TEST_DATABASE_URL` and owns one generated
`cloud_acceptance_*` schema, which it removes on success. It checks transactions,
optimistic contention, released replay/gaps, project isolation, chronological pages,
replica notifications, durable sessions, fencing, one-use consumption and detected
record changes. This is correctness evidence; fleet capacity requires measured load
at representative event rates and history-query concurrency.

## Independently retained audit checkpoints

The source operator helper verifies retained project chains in a repeatable-read
snapshot and signs metadata-only heads with a separately provisioned Ed25519 seed:

```sh
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  export --config database-reference.json --project project-a \
  --audit-key-variable COLOSSUS_CLOUD_AUDIT_SIGNING_SEED --output checkpoint.json
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  verify-file --anchor checkpoint.json --public-key PINNED_PUBLIC_KEY_HEX
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  verify-db --config database-reference.json --anchor checkpoint.json \
  --public-key PINNED_PUBLIC_KEY_HEX
```

`database-reference.json` contains `CloudDatabaseConfig`, with a connection-variable
reference rather than a URL. The audit variable contains a distinct 32-byte seed in
hexadecimal. Never reuse browser authentication, encryption or certificate keys.
Pin the verification key and archive the checkpoint outside this PostgreSQL database;
trusting the public key embedded in an arbitrary checkpoint provides no identity
assurance. Exports contain opaque entity/feed identities, revisions and hashes, never
released conversation text or credentials. `verify-db` rejects missing or changed
anchored entries and state/feed revisions below their retained heads. Export bounds
are 100,000 heads and two million retained audit records per project per pass.

The same helper supports `maintain --config database-reference.json`. Publication
means a project wakeup was committed, independently of runtime command acceptance.

## Reproducible database load measurement

The `fleet_load` example exercises the real `CloudRepository` path, including native
authority checks, fixed task/thread placement, released run and thread feeds,
cursor advancement, audit/outbox writes, concurrent history reads and recovery:

```sh
cargo run --locked -p colossus-cloud-postgres --example fleet_load -- \
  --streams 300 --rate 1000 --seconds 20 --pool 16 \
  --report /tmp/cloud-load.json
```

Set `COLOSSUS_CLOUD_TEST_DATABASE_URL` for the explicit loopback fixture. This example
creates and removes one generated `cloud_load_*` namespace; `--keep` retains it for
operator inspection, with its name included in the report. Its default workload uses
90% 512-byte output deltas, 5% 2-KiB released assistant messages and 5% state updates.
Offered traffic is paced, while each source preserves sequence order. The JSON report
includes achieved throughput, operation/history p50/p95 latency, offered queue lag,
pool wait/timeouts, failures/unattempted events and ten-stream reconnect/backfill,
exact replay and continuation checks. It also enables bounded numeric transaction
profiling after setup: transaction/pool-checkout time, awaited mutation SQL wall
time/count and synchronous JSON/audit preparation time. These are aggregate times
across concurrent workers; SQL wall time includes client scheduling/decoding and
does not mean PostgreSQL server execution time alone. Profiling remains disabled
in ordinary adapter construction. Run on an otherwise quiet host and state the
build profile, fixture resources and payload mix with any reported results.

This is a local database/application measurement. It does not establish capacity for
gRPC connections, SSE browsers, OIDC, Kubernetes replicas or actual runtime execution.
