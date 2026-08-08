---
title: Storage and worker
description: Operate the canonical journal, projections, and authenticated local worker.
audience: operator
type: concept
---

# Storage and worker

The event journal is canonical. Session transcripts, work records, workflow
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
  startupVerification: incremental
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

`storage.keys.kind: none` stores hash-chained plaintext payloads without checkpoints or
anchors. `platform` and `environment` enable the complete protected tier using
independent 32-byte journal and signing keys plus a separately protected secure anchor.
The mode is recorded in each redb file or PostgreSQL schema and cannot change in place.

Startup verification defaults to `incremental`. A legacy or missing versioned anchor
causes a one-time complete bootstrap audit and writes the version-two attestation; this
can take tens of seconds for a large existing journal. Later clean starts verify one
checkpoint boundary plus any uncheckpointed tail instead of decrypting all history.
In keyless mode, incremental startup validates local head and index invariants without
replaying historical payloads. Those checks stay bounded: redb reads its constant-time
table lengths, and PostgreSQL reads only the first and last indexed sequence of each
canonical table rather than counting rows, so startup cost does not grow with journal
size. A PostgreSQL journal with an interior deletion is therefore detected by
`startupVerification: full` rather than at every start.
Set `startupVerification: full` when policy requires complete replay before every
writable start. `state doctor` reports the configured mode, actual path, verified
sequence range, event count, and anchor version.

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

Ordinary workers create or load an independent versioned secret at
`<storage.path>.worker-auth`. The file must be a current-user, owner-only, single-link
regular file; clients only load it. Managed Desktop continues to deliver its independent
worker key through inherited native bootstrap memory.

## Public application API

On Unix, the installed worker can host the versioned application gRPC API alongside
private CLI/TUI IPC:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api"
```

The argument is an absolute discovery directory, not a secret directory supplied by an
application. Colossus creates the final component with mode `0700` when absent, then
canonicalizes, owner-checks, locks, and rechecks it. Existing links, another owner,
group/other permissions, or an active directory lease fail closed. The worker binds
only `127.0.0.1:0` and atomically publishes:

- `endpoint.json`: API/schema version, stable instance ID, loopback HTTPS endpoint,
  PID, and certificate SHA-256;
- `certificate.pem`: the pinned public leaf certificate; and
- `.public-api.lock`: non-secret same-directory ownership coordination.

All are owner-only. No file contains a bearer, authentication root, TLS private key,
or instance seed.

For each canonical directory, Colossus derives a dedicated keyring service namespace
from its SHA-256 and loads or creates three separate 32-byte keyring entries:
`authentication-root-v1`, `tls-seed-v1`, and `instance-identity-seed-v1`. They are
stable across worker restarts and independent of the journal encryption key,
checkpoint signing key, permit MAC, worker IPC key, provider credentials, and each
other. They have no environment-variable, command-line, or file fallback.

### First application enrollment

Enrollment and revocation are offline administration operations. Stop the worker first;
the command refuses to proceed when the configured worker endpoint or journal writer
is owned.

```bash
colossus --config .colossus/config.yaml worker --shutdown

colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:example-desktop \
  --scope runs:execute \
  --scope runs:read \
  --scope runs:control \
  --scope prompts:respond \
  --role primary \
  --credential-keyring-service com.example.desktop \
  --credential-keyring-account colossus-public-api
```

Use the application's own OS-keyring service/account. The command stores the one-time
bearer directly at that destination and prints only the application ID, non-secret
credential ID, stable instance ID, certificate SHA-256, exact grant, and destination
identifiers. It never accepts or prints a bearer. Provision the instance ID and pin
into signed application configuration or application-owned protected storage; do not
derive the expected pin from the mutable discovery directory. If destination storage
fails after pending issuance, Colossus revokes the new credential before returning an
error. A newly issued credential cannot authenticate until keyring delivery succeeds
and a separate durable activation event is recorded.

The generic keyring destination is an OS-user credential store. Its service/account
values identify an entry but do not authenticate a process. Platform behavior differs:
an unlocked Secret Service collection can be visible to other processes for the same
user, while macOS may prompt or deny a separately signed application reading an item
created by this CLI. Use a platform-specific application-bound credential provider
when same-user process isolation is required, and test enrollment/readback in the
packaged application.

At least one exact scope and role is required. The recognized scopes are
`runs:execute`, `runs:read`, `runs:control`, `prompts:respond`, and
`approvals:respond`. Add only the scopes the application needs. Repeat
`--tool EXACT_TOOL_NAME` for every permitted tool; no `--tool` flags means deny all
tools. `agent.delegate` is rejected until application authority is safely propagated
to child runs. Granting `approvals:respond` lets that application satisfy effect
approval obligations and should receive separate review.

Enrollment refuses to overwrite an existing destination keyring entry. During an
intentional rotation, add `--replace-credential`. Colossus first authenticates the
existing entry under this API root and confirms the same application ID. A malformed,
revoked, foreign-root, or other-application token fails before issuance. It then
issues the new credential as pending, stores it, durably activates it, and durably
revokes the old credential in that order; output contains both non-secret IDs. If
activation fails, the new credential is revoked and the prior keyring value is restored
when possible. If old-credential revocation cannot be confirmed after the new
credential is active, Colossus preserves the active new credential at the destination
instead of risking a rollback to an already-revoked token. The sanitized error includes
both non-secret credential IDs and instructs the administrator to reconcile and
explicitly revoke the old credential; it never includes either bearer.

Use explicit revocation for credentials held under other keyring entries:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --revoke-credential 018f0000-0000-7000-8000-000000000001
```

Revocation is durable and idempotent.

## Projection operations

Use status and drain during normal operation. Rebuild only a named disposable
projection after preserving and verifying canonical state:

```bash
colossus --config .colossus/config.yaml projection status
colossus --config .colossus/config.yaml state doctor
```

See [Storage configuration](../reference/configuration/storage.md) for the exact adapter,
key-provider, secure-anchor, and PostgreSQL TLS fields. See
[State and recovery](../develop/state-recovery.md) for the implementation contract and
[Audit, telemetry, and recovery](audit-telemetry-recovery.md) for incident handling.
