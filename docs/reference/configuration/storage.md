---
title: Storage configuration
description: Configure ephemeral, redb, or PostgreSQL canonical journal storage.
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
| Journal adapter | Retains the canonical append-only event history in process memory, redb, or PostgreSQL |
| Protection mode | Stores payloads as plaintext canonical JSON or authenticated ciphertext |
| Journal key | Encrypts event payloads in protected mode |
| Signing key | Signs checkpoints in protected mode |
| Secure anchor | Keeps the last trusted journal sequence and hash outside the journal so rollback or truncation can be detected |
| `storage.location` | Selects the confinement base for a relative storage path |
| `storage.path` | Selects the local redb file or the local instance identity for `ephemeral` and PostgreSQL |

redb and PostgreSQL support `keys.kind: none` and the fully protected `platform` and
`environment` modes. Ephemeral storage requires `none`: a persistent secure anchor
cannot safely describe a fresh process-local journal. PostgreSQL TLS protects the
database connection; it does not change payload protection at rest. Normal worker IPC
uses an independent owner-only `<storage.path>.worker-auth` secret and never derives
authentication from storage keys.

For deployment topology, worker ownership, projections, and the public application API,
see [Storage and worker](../../admin/storage-worker.md).

## Choose a starting point

| Scenario | Adapter | Key provider | Guidance |
| --- | --- | --- | --- |
| One-shot job with no restart or recovery requirement | `ephemeral` | `none` | Keeps canonical state, projections, the default memory index, and automatic MCP OAuth state in process memory |
| Disposable container that must survive a process restart | `redb` | `none` | Place the file on an `emptyDir` or other Pod-lifetime volume |
| Desktop managed deployment | `redb` | `platform` | The OS credential store protects keys and the anchor |
| Headless single-host service | `redb` | `environment` | Inject both keys and preserve the file-backed anchor |
| One long-running worker | `redb` | Either | Let the worker own the single redb writer lease |
| Shared or multi-process deployment | `postgres` | Usually `environment` | Put the journal in PostgreSQL and inject identical keys into every runtime |
| Same-host PostgreSQL processes under one OS identity | `postgres` | `platform` can work | Every process must resolve the same service and key accounts |

Start with redb unless the complete run is deliberately process-local or multiple
processes need direct access to the canonical journal. Changing `adapter` is not a
migration: Colossus does not import, merge, copy, or delete another adapter's journal.

## Process-local ephemeral storage

Use `ephemeral` for a one-shot command or Kubernetes Job whose result is delivered to
an external system and whose Colossus history is intentionally discarded at process
exit:

```yaml
storage:
  location: workspace
  path: .colossus/ephemeral-instance
  adapter: ephemeral
  keys:
    kind: none
```

This runs the production redb journal and projection implementation over redb's memory
backend. It creates no canonical database or writer-lock file. The default Tantivy
memory index and the MCP `oauthCredentialStore: auto` selection are also process-local.
`storage.path` remains a logical identity for adjacent explicitly persistent features;
ordinary one-shot commands do not use those files. Because the journal never leaves the
current process, `colossus worker` is refused under this adapter: serving, `--status`,
`--shutdown`, and public API hosting all require a worker another process can reach.
`colossus worker --once`, which recovers and drains inside the running process, remains
available.

Every invocation starts empty. Sessions, workflow progress, child jobs, approvals,
idempotency claims, effect evidence, audit history, projections, memories, and OAuth
tokens disappear when the process exits or crashes. A later Kubernetes retry is a new
run and cannot determine whether an earlier external effect completed. Use file-backed
redb or PostgreSQL whenever retry, resume, audit retention, or unknown-outcome recovery
matters. `state doctor` reports `persistence: process` and emits an explicit security
posture warning in this mode.

Explicit external audit exporters, `mcp.oauthCredentialStore: platform`, Chroma
projection metadata, and other separately configured adapters keep their documented
persistence behavior; `ephemeral` governs the canonical journal/projection store and
the automatic local defaults.

## Keyless plaintext default

The sparse configuration created by `colossus config init` contains only the storage
placement that differs by scope:

```yaml
storage:
  location: home_workspace
  path: state.redb
```

Omitted `adapter`, `startupVerification`, and `keys` resolve to `redb`, `incremental`,
and `none`. Use `config show` to inspect that complete resolved shape, or pass
`--storage-keys none|platform|environment` to pin a protection choice in the authored
file. Payload
descriptors use `plaintext-json-v1`, `key_id: none`, an empty nonce, and hex-encoded
canonical JSON. Colossus still checks each payload hash and JSON shape when it is read,
maintains the record-hash chain, stream indexes, outbox, and projections, and performs a
complete replay for `audit verify` or `startupVerification: full`.

Incremental keyless startup performs bounded local head, hash, index, outbox, and
projection-bound checks without replaying historical payloads. Signed checkpoints and
secure anchors are disabled. This mode protects integrity against ordinary corruption,
not confidentiality or rollback by an attacker who can consistently rewrite all local
state.

## Local redb with platform keys

This is the recommended protected local configuration:

```yaml
storage:
  location: home_workspace
  path: state.redb
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

### `storage.location`

`location` selects the base for a relative `storage.path`:

| Value | Behavior |
| --- | --- |
| `workspace` | Resolve from the canonical workspace; this is the compatibility default when omitted |
| `home_workspace` | Resolve beneath the current workspace's `cli` home partition |

The global `config init` writes `home_workspace`; `config init --local` writes
`workspace`. Under `home_workspace`, `path` must be relative and confined. Absolute
paths, parent components, and any result outside the CLI partition are rejected. This
partition is distinct from Desktop Managed Local state. See
[Colossus home and workspace resolution](../colossus-home.md).

### `storage.path`

With `location: workspace`, `path` may be workspace-relative or absolute. File-backed
adapters create the parent directory when needed. With `location: home_workspace`, it
must be a confined relative path and resolves from the selected workspace's CLI home
partition. Ephemeral storage validates this identity without creating its leaf or
parent directories.

Its exact meaning depends on the adapter:

| Adapter | Meaning of `storage.path` |
| --- | --- |
| `redb` | The canonical redb database file and the basis for its writer lease |
| `ephemeral` | A logical local instance identity; it is not a database path |
| `postgres` | A local instance identity used to derive worker IPC and adjacent local state paths; it is not the PostgreSQL journal location |

With ephemeral storage or PostgreSQL, changing `storage.path` changes local instance
identity without moving canonical data. Keep it stable for a deployed instance.

Do not treat copying a redb file, changing this path, or switching adapters as a storage
migration. Provision the destination explicitly, verify its keys and secure anchor, and
follow the operational recovery process.

### `storage.adapter`

| Value | PostgreSQL block | Concurrency model |
| --- | --- | --- |
| Omitted or `redb` | Must be absent | One local writer lease; other clients should use the worker |
| `ephemeral` | Must be absent | One fresh process-local store; no redb writer lease |
| `postgres` | Required | Canonical journal is shared through PostgreSQL |

The canonical spelling is `postgres`. An unknown adapter, a `postgres` block under
redb or ephemeral, or `adapter: postgres` without that block fails configuration
validation. Ephemeral storage additionally rejects `platform` and `environment` keys.

### `storage.startupVerification`

| Value | Startup behavior |
| --- | --- |
| Omitted or `incremental` | Protected mode verifies the signed checkpoint tail; keyless mode performs bounded local integrity checks |
| `full` | Replay, decode, and verify every journal event before every writable start |

Incremental verification is the default. A legacy, absent, quarantined, or incompatible
secure anchor causes one complete bootstrap audit and writes a version-two attestation.
That first start can take tens of seconds for a large existing journal. Later clean
starts inspect one checkpoint boundary plus any uncheckpointed tail instead of
decrypting all history.

Set `startupVerification: full` when policy requires complete replay before every
writable start. The `audit verify` command always performs a complete audit regardless
of this setting. `state doctor` reports the configured mode, actual verification path,
verified sequence range, event count, and secure-anchor version.

## Key providers and fixed protection mode

`kind: none` is the default. `kind: platform` and `kind: environment` both enable the
complete protected tier: authenticated payload encryption, signed checkpoints, and
secure anchors. Encryption and signing cannot be enabled independently.

Each redb file and PostgreSQL schema stores a durable protection marker. An ephemeral
store initializes a plaintext marker for its process lifetime. A nonempty persistent
store created before this marker existed is classified as encrypted. A configured
mismatch fails startup before event writes; Colossus never creates mixed payload
algorithms and does not migrate protection in place. Use a fresh path or schema to
harden or simplify a deployment.

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
  location: workspace
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
| `anchor_path` | With `location: workspace`, a workspace-relative or absolute integrity path; with `home_workspace`, a confined relative path resolved beneath the CLI partition |

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
| `none` | Disabled; `checkpoint` returns no checkpoint |
| `none` with `ephemeral` | Disabled; all journal state ends with the process |
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

With this example PostgreSQL journal payloads are encrypted with
`COLOSSUS_JOURNAL_KEY`. A PostgreSQL configuration may instead use `keys.kind: none`;
every runtime opening one schema must select the same protection mode.

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

The supported adapter/protection combinations are:

| Adapter | No keys | Platform keys | Environment keys |
| --- | --- | --- | --- |
| ephemeral | One-shot process-local jobs | Invalid | Invalid |
| redb | Simple jobs and protected local volumes | Protected interactive single-host use | Services with injected secrets |
| PostgreSQL | Simple shared jobs where database controls are sufficient | Same-host processes resolving the same entries | Multi-host deployments with a central secret manager |

For PostgreSQL on multiple hosts, each host also has its own local `storage.path` and,
with environment keys, its own `anchor_path`. Manage those local identities and anchors
as durable deployment state even though the journal is shared.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| An ephemeral retry cannot find its prior run | This is expected; every process starts with an empty journal |
| Ephemeral configuration rejects protected keys | Use `keys.kind: none`, or select redb/PostgreSQL when anchors and protected persistence are required |
| redb startup reports a writer lease conflict | Another process owns the same `storage.path`; use its worker or stop the duplicate writer |
| PostgreSQL configuration is rejected before connecting | `postgres` requires a `postgres` block, while redb and ephemeral forbid one |
| PostgreSQL is healthy but journal decryption fails | Every process must use the same journal key bytes and stable key ID |
| An old journal becomes unreadable after a config edit | Changing key IDs or secret bytes does not rotate or migrate encrypted events |
| Startup reports a payload-protection mismatch | The nonempty path or schema was initialized in another mode; select it or use fresh storage |
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
