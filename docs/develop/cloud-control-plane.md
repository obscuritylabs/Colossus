---
title: Develop and test Colossus Control Plane
description: Run the real local OIDC, browser, host and runtime connection stack and its acceptance checks.
audience: developer
type: how-to
---

# Develop and test Colossus Control Plane

The stack uses Rust/Tokio for the server and connector, React/TypeScript with shared
shadcn-based Colossus UI for the browser, Diesel/diesel-async for the dedicated
PostgreSQL schema, and optional named OIDC or local Argon2id authentication. `colossus-cloud` owns project authority,
host/agent/workspace inventory, conversations, fixed-runtime tasks and the asynchronous
`CloudStore` port. `colossus-cloud-postgres` owns SQL, pooling, migrations, indexed
queries, transactions and replica coordination; it does not implement `EventJournal`.
Runtime journals and their recovery adapters remain runtime-owned.

Persisted users and issuer/subject identities own project membership. Optional local
login uses bounded Argon2id work, shared database attempt limits, and session revocation
epochs; the explicit bootstrap configuration provisions the first administrator.
Hierarchy membership is not inherited. Global administrator read access does not
create a runtime grant. See [operator setup](../admin/cloud-control-plane.md) for the
production configuration and administration flow.

`colossus-cloud-protocol` owns the closed versioned connection contract.
`colossus-cloud-server` hosts HTTP/OIDC, browser SSE and end-to-end TLS 1.3 mutual-TLS
gRPC. `colossus-connector` uses only the public SDK under a dedicated native application
grant. Desktop embeds that connector; CLI exposes it through `control-plane` (`cloud` remains an alias), and the standalone
binary owns the same lifecycle commands.

## Run the local stack

Start the disposable identity provider and PostgreSQL fixtures:

```sh
docker compose -f deploy/cloud/local/compose.yml up -d
cargo run -p colossus-cloud-server --example init_local -- /private/tmp/colossus-cloud
source /private/tmp/colossus-cloud/local.env
cargo run -p colossus-cloud-server -- /private/tmp/colossus-cloud/cloud.json
```

On Linux, select an absolute owner-private path such as `/tmp/colossus-cloud`.
The example generates a private local CA/server key, a protected `local.env` with the
disposable database URL and a random OIDC-flow key, loopback HTTP on 8090 and mutual-TLS
gRPC on 8443. Source that environment in the server terminal without printing its
contents. PostgreSQL is also used locally; only deterministic unit fixtures use
`MemoryCloudStore`. The generator refuses to overwrite an existing configuration.
Development HTTP and disabled database TLS require explicit loopback-only configuration
and are rejected for production endpoints.

In another terminal:

```sh
npm ci --prefix apps/web --ignore-scripts --legacy-peer-deps --install-links
npm run dev --prefix apps/web
```

Open `http://127.0.0.1:5180` and sign in as `alice` with `local-only-password`. The fixture
realm/client and explicit subject membership match the generated local configuration.
Those checked-in credentials exist only for this disposable loopback fixture.

Generate an isolated credential-free Echo runtime:

```sh
cargo run -p colossus-cloud-server --example init_runtime -- /private/tmp/colossus-runtime
cargo build --locked -p colossus-cli -p colossus-connector --bins
```

Follow [runtime enrollment](../admin/cloud-control-plane.md#enroll-a-cli-daemon) with
the generated runtime config and API directory. Use `--tool echo` for the Echo fixture.
Cloud enrollment to the loopback host additionally requires `--allow-loopback-http`.
Start the worker and connector in separate terminals. The browser should show the
agent and host/workspace inventory ready. Start an Echo thread, verify released output,
and send a second turn in the same conversation. Disconnect the connector and reopen
saved history, then reconnect and reconcile queued work on that same agent.

For Desktop, prepare current verified binaries before launching the native app:

```sh
cargo xtask desktop prepare --profile debug
npm run tauri --prefix apps/desktop -- dev -- --locked
```

Select an isolated Managed Local workspace and use its global **Control Plane** settings. Native
confirmation, keyboard/focus behavior, and actual platform credential storage require
an unlocked graphical desktop. Renderer fixtures do not establish those properties.

Build the standalone Control Plane image from the repository root:

```sh
docker build -f deploy/control-plane/Dockerfile -t colossus-control-plane:local .
kubectl kustomize deploy/control-plane/kubernetes
```

The default image uses the release profile. `--build-arg PROFILE=dev` builds a stripped
debug image for local Linux acceptance. Builder/runtime images and local fixtures are
digest pinned. Production deployment and Secret configuration belong to the
[operator guide](../admin/cloud-control-plane.md).

Run the [local documentation image](documentation.md#build-the-on-prem-documentation-image)
on loopback port 8081 alongside the web development server. Vite forwards the default
`/docs/` Documentation link there without a path rewrite; it serves public static
pages and local search independently of Control Plane sign-in.

Build and validate Linux Desktop in its disposable native builder:

```sh
docker build -f deploy/cloud/linux-desktop.Dockerfile --target artifacts \
  --output type=local,dest=/tmp/colossus-linux-desktop .
```

The host architecture selects x86-64 or ARM64. This exports an unsigned Developer
Preview Debian package and its checksum. The builder verifies the native masked
credential dialog under an isolated Xvfb display, including validation, clipboard
preservation and cancellation. The package seals its sidecar/CLI/search digests into
the main ELF executable. Linux packaging also runs in the full pre-merge acceptance
gate. A native Linux checkout with the builder's listed dependencies can use
`COLOSSUS_DESKTOP_RELEASE_CHANNEL=developer_preview COLOSSUS_DESKTOP_TEAM_ID=UNSIGNED
sh scripts/package-desktop-linux` after installing Desktop npm dependencies.

## Work on cloud storage and synchronization

The relational tables are separated by domain: project membership, host/agent/workspace
inventory, conversation threads/messages, tasks, commands/receipts, session/run mappings,
enrollment and certificate rotation. Indexed domain columns accompany versioned JSONB
SDK snapshots. The cloud store commits related entity revisions, command delivery,
released events and sync cursors with audit and outbox records in the same transaction.
Entity compare-and-swap and per-feed ordering replace the old global journal-head lock.

Lists use bounded project-scoped queries and timestamp/identifier cursors. Thread detail
returns the latest messages in chronological order and the latest runs, with separate
cursors for earlier pages. Source discovery remains caller-authorized through the SDK;
cloud membership cannot disclose an unrelated local application's history.

The connector advertises an opaque native-installation host identity and its workspace.
Desktop can expose multiple independently granted workspace agents under that host.
A cloud thread maps to a fixed node and native session; every human turn remains a
separate immutable task/run allocation. The runtime owns execution, policy and released
content. The cloud owns metadata, membership and the ordered submission queue. Concurrent
human submissions and metadata changes reconcile exact revisions and idempotency keys.

Local source sessions require explicit workspace sharing. Imported runs stay read-only.
When continuation is shared, a new cloud-owned run can continue that native session under
the connector's independent scopes, roles and tools. Disabling sharing stops future
synchronization; it does not delete already retained cloud history.

Each production replica has a bounded asynchronous pool and a dedicated PostgreSQL
notification listener. Notifications contain project identities only and wake readers;
all browser/event consumers reread durable records from their cursors. Notification loss
or reconnect cannot stand in for durable acknowledgement. Browser sessions and encrypted
OIDC flows are shared. Connection leases carry fencing generations checked within runtime
transactions so a stale replica cannot commit output after replacement ownership.

Configured legacy memberships seed audited database identities and memberships once,
preserving their exact existing permission ceilings. Database roles are authoritative
after migration; restart does not overwrite administrator edits. Explicitly assigning a
built-in role replaces a legacy customized permission set with that role's exact bundle.
Every request and SSE revalidation checks the active user, session security epoch and
current project membership. All replicas share authentication configuration and keys.
Session and PKCE-flow hashes bind the issuer/client namespace; changing identity
configuration requires new browser sign-ins. Upgrade the former configuration-only
authentication deployment with all old replicas stopped before admitting users under
the new identity schema; do not depend on revocation enforcement across mixed versions.

`CloudStore::maintain` drains coalesced project wakeup hints with `FOR UPDATE SKIP LOCKED`,
marks publication and prunes delivered hints older than one day. Minute maintenance
passes also remove expired browser sessions and consumed expired OIDC flows in bounded
batches of 256 records per collection. Import/maintenance markers remain unexpired.
The maintenance contract never deletes canonical thread/run events, messages, tasks,
cursors or audit chains; their archive boundary requires an explicit retention design.

Numbered SQL migrations under `crates/colossus-cloud-postgres/migrations` run transactionally
at store startup under a schema-specific advisory lock. Applied checksums are persisted.
Add a new migration for a deployed schema; do not change already applied SQL. Schema
rollback files are destructive operator tools and are never executed by ordinary startup.
The database and independently archived audit heads require tested restore procedures;
[cloud operations](../admin/cloud-control-plane.md#back-up-and-restore-the-cloud-database)
owns those steps and the credential references.

The offline `migrate-journal` entry point imports the prior cloud journal into an empty
SQL store, preserving stable task/run/command identities and source event/cursor state.
It records a verified source marker and resumable progress; ordinary startup refuses an
incomplete import. Keep the source frozen and target server stopped during import.
[Operator migration](../admin/cloud-control-plane.md#migrate-an-existing-cloud-journal)
owns the installed-binary command, source backup and production cutover sequence. This
conversion never migrates or opens local runtime journals.

## Export and verify an independent audit checkpoint

The source helper under `colossus-cloud-postgres` signs metadata-only audit heads with
a separate 32-byte Ed25519 seed. Supply a protected database-reference JSON file using
`CloudDatabaseConfig`'s camel-case fields and an environment-variable reference for the
connection URL. Keep `COLOSSUS_CLOUD_AUDIT_SIGNING_SEED` in your secret manager; it must
be independent of the OIDC-flow key and runtime journal keys.

```sh
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  export --config /secure/cloud/database-reference.json --project engineering \
  --audit-key-variable COLOSSUS_CLOUD_AUDIT_SIGNING_SEED \
  --output /secure/backups/engineering-checkpoint.json
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  verify-file --anchor /secure/backups/engineering-checkpoint.json \
  --public-key INDEPENDENTLY_PINNED_PUBLIC_KEY_HEX
cargo run --locked -p colossus-cloud-postgres --example audit_checkpoint -- \
  verify-db --config /secure/cloud/database-reference.json \
  --anchor /secure/backups/engineering-checkpoint.json \
  --public-key INDEPENDENTLY_PINNED_PUBLIC_KEY_HEX
```

Export creates a new owner-private file and reports its public key, checkpoint digest
and head count. Pin that public key through an independent trusted channel; accepting
an arbitrary key carried by the exported file defeats the trust boundary. Archive the
checkpoint independently of the mutable database. Offline verification rejects changed
signed metadata or a wrong key; database verification checks retained audit chains and
rejects revisions below the anchored heads. Checkpoints contain resource metadata and
hashes rather than released conversation plaintext. They complement encrypted database
backups and restore tests.

The adapter's
[source README](https://github.com/obscuritylabs/Colossus/tree/main/crates/colossus-cloud-postgres)
owns the helper's exact configuration, maintenance and reproducible load-measurement
commands. The running service already performs bounded operational maintenance;
`audit_checkpoint maintain` is an explicit developer/operator diagnostic using the same
policy and does not prune canonical history.

## Acceptance and completion checks

```sh
cargo test --locked -p colossus-cloud -p colossus-cloud-protocol -p colossus-cloud-postgres
cargo test --locked -p colossus-cloud-server --lib
cargo test --locked -p colossus-credentials --test headless
cargo test --locked -p colossus-sidecar --test native_lifecycle -- --ignored
cargo xtask check web
cargo xtask check rust
cargo xtask check desktop
cargo xtask check docs
cargo xtask pr --base origin/main
```

The server acceptance owns a signed local OIDC issuer, real browser HTTP/SSE, encrypted
native enrollment records, the production mutual-TLS transport and connector, and a
real runtime using a deterministic credential-free model endpoint. It verifies prompt
responses, policy-bound file-write approval, cancellation, certificate rotation, durable
host restart, and revocation. No external model key or provider service is required.
The native sidecar acceptance requires a usable OS credential store and loopback sockets;
it proves independently granted cloud and Desktop callers cannot read each other's runs.

The ignored `managed_sidecar_cloud_run_isolated_and_application_exit_disconnects`
server acceptance uses `COLOSSUS_ACCEPTANCE_SIDECAR` to select a prepared native sidecar.
It signs in through OIDC, enrolls its independently bootstrapped cloud grant, submits
a browser HTTP task through the mutual-TLS connection, receives released native output,
and verifies application exit disconnects cloud readiness without changing the durable
run identity. The Linux package builder and native macOS/Windows pre-merge jobs run
this check with the platform credential store.

The operator-owned PostgreSQL variant runs the same transport/runtime/restart flow
against the local fixture. Inject `COLOSSUS_CLOUD_TEST_DATABASE` with its connection
URL, then run:

```sh
cargo test --locked -p colossus-cloud-server --lib postgres_oidc_to_runtime_and_recover -- --ignored
```

Each invocation owns a uniquely named `cloud_e2e_` schema in the disposable fixture
database. Never point this acceptance command at an operator's production database.
The lower-level database conformance uses a separate explicit reference:

```sh
cargo test --locked -p colossus-cloud-postgres --lib \
  postgres_atomic_recovery_and_replica_conformance -- --ignored
```

Inject `COLOSSUS_CLOUD_TEST_DATABASE_URL` with the disposable fixture connection URL.
That test owns a generated `cloud_acceptance_` schema. It verifies atomic state/event/cursor
commits, optimistic contention, chronological pages, notification fanout, shared browser
sessions, one-use consumption, lease fencing and changed retained-record detection.

Repository tests additionally cover thread continuation and read-only session sharing,
exact idempotent receipts, project isolation,
contiguous output and changed-duplicate rejection, role ceilings, admission bounds,
certificate renewal ACK reconciliation, OIDC signature/issuer/audience/nonce/expiry
validation and CSRF. The headless acceptance runs isolated processes to prove sealed
authority survives restart and fails closed when its wrapping key changes.

Use the actual web app for desktop/mobile, light/dark, keyboard/dialog and browser
console acceptance. Test native enrollment confirmation separately on the supported
platform. The Desktop gate includes the cloud web gate, and changes under `apps/web`
select that gate through the existing CI classification contract.

Correctness checks do not establish a fleet-capacity claim. Measure concurrent streaming
agents separately from registered idle agents. Record released events/second and size,
command acknowledgement and launch latency, history-query concurrency, database pool wait,
transaction latency and connection count. Repeat with reconnect backfill, simultaneous human
submissions and replica replacement. Use representative workspace/thread histories and the
same TLS/database topology as deployment before selecting replica and pool budgets.

Keep local invitations, private keys, screenshots, logs and task plans in ignored
development output or owner-private temporary directories. Remove disposable fixtures
with `docker compose -f deploy/cloud/local/compose.yml down` after acceptance.
