---
title: Storage configuration
description: Configure canonical journal storage, encryption and signing keys, secure anchors, and PostgreSQL with practical examples.
audience: operator
type: reference
---

# Storage configuration

`storage` is required. It tells Colossus where the canonical event journal lives and
how to protect it. Storage configuration is separate from projections, semantic
indexes, and exports: those are derived data that Colossus can rebuild from the
journal.

Use this mental model when configuring storage:

| Component | Purpose |
| --- | --- |
| Journal adapter | Persists the canonical append-only event history in redb or PostgreSQL |
| Journal key | Encrypts event payloads before either adapter stores them |
| Signing key | Signs checkpoints and supplies domain-separated worker authentication material |
| Secure anchor | Keeps the last trusted journal sequence and hash outside the journal so rollback or truncation can be detected |
| `storage.path` | Selects the local redb file or, with PostgreSQL, the local Colossus instance identity |

Both adapters store encrypted journal payloads. PostgreSQL TLS protects the database
connection; it does not replace journal encryption.

For deployment topology, worker ownership, projections, and the public application API,
see [Storage and worker](../../admin/storage-worker.md).

## Choose a starting point

| Scenario | Adapter | Key provider | Guidance |
| --- | --- | --- | --- |
| Desktop or single-user CLI | `redb` | `platform` | Simplest default; the OS credential store protects keys and the anchor |
| Headless single-host service | `redb` | `environment` | Inject both keys and preserve the file-backed anchor |
| One long-running worker | `redb` | Either | Let the worker own the single redb writer lease |
| Shared or multi-process deployment | `postgres` | Usually `environment` | Put the journal in PostgreSQL and inject identical keys into every runtime |
| Same-host PostgreSQL processes under one OS identity | `postgres` | `platform` can work | Every process must resolve the same service and key accounts |

Start with redb unless multiple processes need direct access to the canonical journal.
Changing `adapter` is not a migration: Colossus does not import, merge, copy, or delete
the other adapter's journal.

## Local redb with platform keys

This is the recommended local configuration:

```yaml
storage:
  path: .colossus/state.redb
  adapter: redb
  startupVerification: incremental
  keys:
    kind: platform
    service: dev.colossus.runtime
    journal_key_id: journal-production
    signing_key_id: checkpoint-production
```

When an entry is absent, Colossus generates a random 32-byte journal or signing key and
stores it through the operating system credential service. The secure anchor is stored
there too. Keyring failure is fatal; Colossus never silently writes these keys to disk.

Choose stable, deployment-specific `service` and key IDs. Reusing generic identifiers
across unrelated deployments makes their credential namespaces collide.

## Top-level storage fields

### `storage.path`

`path` may be workspace-relative or absolute. Relative paths resolve from the canonical
selected workspace, and Colossus creates the parent directory when needed.

Its exact meaning depends on the adapter:

| Adapter | Meaning of `storage.path` |
| --- | --- |
| `redb` | The encrypted canonical redb database file and the basis for its writer lease |
| `postgres` | A local instance identity used to derive worker IPC and adjacent local state paths; it is not the PostgreSQL journal location |

With PostgreSQL, changing `storage.path` changes local instance identity even when the
database connection and schema stay the same. Keep it stable for a deployed instance.

Do not treat copying a redb file, changing this path, or switching adapters as a storage
migration. Provision the destination explicitly, verify its keys and secure anchor, and
follow the operational recovery process.

### `storage.adapter`

| Value | PostgreSQL block | Concurrency model |
| --- | --- | --- |
| Omitted or `redb` | Must be absent | One local writer lease; other clients should use the worker |
| `postgres` | Required | Canonical journal is shared through PostgreSQL |

The canonical spelling is `postgres`. An unknown adapter, a `postgres` block under
redb, or `adapter: postgres` without that block fails configuration validation.

### `storage.startupVerification`

| Value | Startup behavior |
| --- | --- |
| Omitted or `incremental` | Bootstrap the versioned secure anchor when necessary, then verify the signed checkpoint boundary and later journal tail |
| `full` | Replay, decrypt, and cryptographically verify every journal event before every writable start |

Incremental verification is the default. A legacy, absent, quarantined, or incompatible
secure anchor causes one complete bootstrap audit and writes a version-two attestation.
That first start can take tens of seconds for a large existing journal. Later clean
starts inspect one checkpoint boundary plus any uncheckpointed tail instead of
decrypting all history.

Set `startupVerification: full` when policy requires complete replay before every
writable start. The `audit verify` command always performs a complete audit regardless
of this setting. `state doctor` reports the configured mode, actual verification path,
verified sequence range, event count, and secure-anchor version.

## Key providers

The journal encryption key and checkpoint signing key are independent 32-byte secrets.
Do not give them the same value. The identifiers in YAML are identities, not secret
material and not rotation instructions.

### Platform credential store

Use `kind: platform` when every process can access the intended OS Keychain, Windows
Credential Manager, or Secret Service entries:

```yaml
keys:
  kind: platform
  service: dev.colossus.runtime
  journal_key_id: journal-production
  signing_key_id: checkpoint-production
```

| Field | Meaning |
| --- | --- |
| `service` | Namespace in the operating system credential store |
| `journal_key_id` | Stable identity of the active event-encryption key |
| `signing_key_id` | Stable identity of the checkpoint-signing key |

Colossus uses distinct journal, signing, and anchor accounts under the configured
service. It can retrieve a historical journal key when that key's account still exists.

For a shared PostgreSQL deployment, do not let separate hosts independently create
entries with the same IDs: each host would generate different bytes. Provision the
same protected secret material everywhere or use environment keys supplied by a
central secret manager.

### Environment-backed keys

Use `kind: environment` for headless, air-gapped, container, or externally managed
secret injection:

```yaml
storage:
  path: .colossus/state.redb
  adapter: redb
  startupVerification: incremental
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
```

| Field | Rule |
| --- | --- |
| `journal_variable` | Name of the environment variable containing the journal key |
| `journal_key_id` | Stable identity recorded with encrypted journal events |
| `signing_variable` | Name of the environment variable containing the signing key |
| `anchor_path` | Workspace-relative or absolute path for separately persisted integrity state |

Each variable must contain exactly 32 bytes encoded as hexadecimal or base64. YAML
stores only the variable names. There is no plaintext, generated-file, or keyring
fallback.

These variables are read by Colossus itself. They do not need to appear in
`sandbox.environment` unless an authorized child process also needs them—which storage
keys normally should not.

The environment provider exposes only the configured journal key ID. If existing
events reference a different ID, Colossus cannot decrypt them through this provider.
Changing an ID or replacing the bytes under an existing ID is not a safe key rotation.

## Secure anchors

The secure anchor records the last trusted journal sequence and hash outside the
canonical journal. `state doctor` compares it with the journal to detect missing,
rolled-back, or altered journal state.

| Key provider | Anchor location |
| --- | --- |
| `platform` | Protected credential-store account derived from `service` and `journal_key_id` |
| `environment` | JSON file at `anchor_path`, updated with an atomic rename |

Treat the anchor as integrity-critical state. Keep the file-backed anchor separate from
the redb database, preserve it through host replacement, and back it up consistently
with the journal and key identities. Incremental startup replaces a legacy or absent
anchor only after a successful complete bootstrap audit. A malformed or mismatched
version-two anchor fails closed and quarantines incremental verification until a
complete verification succeeds.

The anchor is not a substitute for the journal encryption or signing key, and a copy of
the journal alone is not a complete verified recovery set.

## PostgreSQL configuration

PostgreSQL is the shared-journal adapter. This complete storage block uses environment
keys and the built-in WebPKI roots:

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

The value of `COLOSSUS_DATABASE_URL` may be a libpq-style URL or key/value connection
string. Put only the environment variable name in YAML. Colossus does not include the
resolved connection value in its safe configuration summaries or adapter diagnostics.

PostgreSQL journal payloads remain encrypted with `COLOSSUS_JOURNAL_KEY`. Every runtime
that opens the same schema must resolve compatible journal and signing keys.

### PostgreSQL fields

| Field | Constraint |
| --- | --- |
| `connectionVariable` | Nonempty POSIX-style environment name; its value contains the connection string |
| `schema` | Deployment-owned, 1–63 byte ASCII identifier beginning with a letter or underscore |
| `tls` | Optional; defaults to `kind: webpki_roots` |
| `statementTimeoutMs` | `100..=300000`; defaults to `30000` |

The schema may otherwise contain ASCII letters, digits, and underscores. Colossus
creates it if needed and sets it as the connection search path. The configured timeout
is applied to both PostgreSQL statements and lock acquisition.

The connection environment variable is consumed in-process and does not need a
`sandbox.environment` grant. Supply it to the Colossus service through the host,
container, or secret-manager configuration.

### PostgreSQL TLS modes

| `kind` | Trust behavior | Intended use |
| --- | --- | --- |
| `webpki_roots` | Built-in WebPKI roots plus `network.caBundlePath` | Public CAs or a shared enterprise CA bundle |
| `custom_ca` | Trust only the adapter-specific PEM bundle | A PostgreSQL-specific private CA |
| `disabled` | No TLS; rejected unless every target is loopback or a Unix socket | Isolated local development and CI only |

An exclusive PostgreSQL CA uses this exact shape:

```yaml
storage:
  path: .colossus/instance.redb
  adapter: postgres
  startupVerification: incremental
  postgres:
    connectionVariable: COLOSSUS_DATABASE_URL
    schema: colossus_production
    tls:
      kind: custom_ca
      caPemPath: /etc/colossus/postgres-ca.pem
    statementTimeoutMs: 30000
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
```

`custom_ca` excludes public roots and `network.caBundlePath`; the PEM file must contain
at least one certificate. With `webpki_roots`, the shared network bundle augments public
roots. See [Network trust configuration](network.md#postgresql) for precedence across
Colossus-owned clients.

## Key and adapter combinations

All four schema combinations are valid, but deployment topology matters:

| Adapter | Platform keys | Environment keys |
| --- | --- | --- |
| redb | Recommended for interactive single-host use | Recommended when a service manager injects secrets |
| PostgreSQL | Suitable only when every process resolves the same protected entries | Recommended for multi-host deployments with a central secret manager |

For PostgreSQL on multiple hosts, each host also has its own local `storage.path` and,
with environment keys, its own `anchor_path`. Manage those local identities and anchors
as durable deployment state even though the journal is shared.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| redb startup reports a writer lease conflict | Another process owns the same `storage.path`; use its worker or stop the duplicate writer |
| PostgreSQL configuration is rejected before connecting | `postgres` requires a `postgres` block, while redb forbids one |
| PostgreSQL is healthy but journal decryption fails | Every process must use the same journal key bytes and stable key ID |
| An old journal becomes unreadable after a config edit | Changing key IDs or secret bytes does not rotate or migrate encrypted events |
| `storage.path` changes but PostgreSQL data does not move | Under PostgreSQL the path is local instance identity, not the database location |
| A copied redb file does not behave like a migration | Adapter and path changes never import keys, anchors, or events |
| The connection string appears directly in YAML | `connectionVariable` accepts the environment variable name, not a literal credential |
| PostgreSQL rejects a schema | Use a 1–63 byte ASCII identifier beginning with a letter or underscore |
| A private database CA is still rejected | Use `caPemPath` under `kind: custom_ca` and confirm the PEM contains certificates |
| `network.caBundlePath` has no effect on PostgreSQL | `custom_ca` is exclusive; use `webpki_roots` to inherit the shared bundle |
| Remote PostgreSQL rejects `kind: disabled` | Disabled TLS is restricted to loopback and Unix-socket targets |
| Journal verification reports an anchor mismatch | Restore and investigate the trusted journal, keys, and anchor as one recovery set |

## Validate the result

First confirm that YAML fields, adapter pairing, paths, and bounds parse without
printing resolved secrets:

```bash
colossus --config .colossus/config.yaml config show
```

Then open the configured adapter and verify the journal, checkpoints, projections, and
secure anchor:

```bash
colossus --config .colossus/config.yaml state doctor
```

For a worker-owned redb deployment, also confirm the worker endpoint:

```bash
colossus --config .colossus/config.yaml worker --status
```

`config show` does not prove that environment variables, platform credential entries,
CA files, or PostgreSQL are available. `state doctor` performs the storage-backed
verification. Run it before placing a new target into service and after an intentional
recovery or migration.

Return to the [configuration overview](../configuration.md).
