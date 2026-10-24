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
Hierarchy membership is not inherited. Global administration does not disclose another
user's personal chat history or runtime, and creates no execution grant. See [operator setup](../cloud/deployment/index.md) for the
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

Follow [runtime enrollment](../cloud/connect-runtimes.md#enroll-a-cli-daemon) with
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
cargo xtask check helm
```

The default image uses the release profile. `--build-arg PROFILE=dev` builds a stripped
debug image for local Linux acceptance. Builder/runtime images and local fixtures are
digest pinned. The Helm chart replaces the Kustomize manifests and is packaged with its pinned
PostgreSQL dependency for disconnected installs. Static validation covers Kubernetes
1.34, routing variants, OpenShift identities, authentication and real server config;
it does not install or upgrade a cluster. Production deployment and Secret configuration belong to the
[operator guide](../cloud/deployment/index.md).

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

## Develop hosted personal chat

The optional server `chat` block enables built-in conversations without an enrolled
workstation. `colossus-cloud-chat` composes a protected runtime for the authenticated
project/user; the server's Hosted execution service dispatches durable Cloud commands
directly through the caller-bound SDK and publishes released events to browser SSE.
Hosted targets are independent from Fleet enrollment and certificates. A browser supplies neither
a node owner nor a runtime path. The project/user identity is stable across restarts;
accepted tasks and native sessions retain that fixed placement.

`GET /api/projects/{project}/hosted` discovers the caller's managed target and explicit
capabilities without loading a runtime. Chat's existing status endpoint remains
compatible and returns configured availability independently from residency. New work
is durably accepted before runtime capacity is acquired; a bounded supervisor wakes
cold contexts and rediscovers unfinished work after restart. `chat.idle_seconds`
defaults to 300 seconds; capacity pressure can retire a fully idle context earlier.

Managed file upload uses `POST /api/projects/{project}/hosted/files` with `file_name`,
`media_type`, `content_base64` and a caller-scoped `idempotency_key`. The initial Hosted
surface accepts UTF-8 TXT, Markdown, CSV and JSON up to 256 KiB per file. Attach its
returned artifact ID through the existing SDK input part. Downloads use the exact
owner-authorized `/hosted/files/{artifact}` route and force attachment disposition.
Shared replicas route file operations to the fenced owner through encrypted management;
file data stays in the protected artifact journal. No file is extracted onto a worker.

Use `chat.projects` in `deploy/control-plane/chart/examples/hosted-chat.yaml`
as an operator-owned seed. The Helm gate renders and validates the complete production
server configuration. The example contains no secret values. Select real project/model and
MCP identities, supply the named headless keys through a protected environment, and use
an isolated private `chat.home` for development. The example is a Linux deployment;
local paths must name a canonical owner-private directory on the development host.
`RuntimeConfig::controlled_chat_template` owns the allowed-tool/action baseline and
`RuntimeOpenOptions::for_controlled_chat` independently constrains catalog and effect
policy. Host composition always rewrites journal, anchor, lexical index and workflow
paths into the selected user's private state. Semantic Chroma projection is currently
rejected until its tenant and credential ownership are defined.

To seed the same controlled service from an existing reviewed runtime configuration,
use the source-only generator:

```sh
cargo run --locked -p colossus-cloud-server --example init_chat -- \
  /private/tmp/colossus-hosted-chat example /secure/runtime.yaml
```

The new directory must be owner-private. The generated `chat.json` is the server's
`chat` block, not a complete server configuration. It selects the source primary
provider as an explicit shared service connection and retains only that provider's
model profiles and roles. It copies configuration references without loading or
exporting credentials, and creates no keys. Inspect its exact egress origins, supply
the named independent headless keys, and merge the block into an otherwise reviewed
server configuration. For a Codex primary provider, add the explicit protected native
account file as the fourth argument; that account is intentionally shared. MCP
connections are added separately through reviewed project settings.

For shared SQL deployments, native account paths are rejected. Set the trusted project
seed's `codex_auth_shared` to `true`, keep `codex_auth_path` absent, and retain the
explicit shared provider policy. After SQL migration and before starting that project's
runtimes, the operator can import one selected validated account into encrypted SQL:

```sh
cargo run --locked -p colossus-cloud-server --example import_chat_codex -- \
  /secure/control-plane.json example /secure/service-account/auth.json 0
```

The final argument is the exact expected credential revision; zero creates a new
record. The utility prints only the committed revision and refuses live project
execution leases. Use an explicitly selected service account: the imported snapshot
becomes the cloud authority and is no longer synchronized with the native account
file. Provider reads use shared SQL custody; there is no file fallback. Before a
rotating refresh exchange, a durable CAS claim blocks another executor from refreshing
the same credential. An interrupted or ambiguous exchange remains blocked until an
explicit account reimport/reauthentication, rather than replaying a rotating token.

Project administrators manage provider profiles, model profiles/routes, remote MCP
definitions, and connection network policy through bounded settings DTOs. Missing
connection policy defaults to `allow_all`; `allow_list` accepts exact canonical
HTTP(S) origins. Global publication validates the definitions independently of project
policies. Administrative projections retain blocked definitions and their provenance;
the execution projection filters unavailable connections, models, and roles before
credential resolution and runtime composition. A host-bound unavailable-role set
prevents a filtered explicit model assignment from falling back to Primary, including
roles resolved internally by Research, delegation, or risk evaluation. The host
adds only the permitted configured connection origins to the runtime network grant.
Private roots, journal keys and Codex account paths are not browser configuration.
Submission validates deterministic connection-binding composition before creating a
durable task, thread, message, or branch. Invalid bindings return a request error
without leaving queued work; this validation does not resolve secret values or
contact a provider. Credentialed loopback HTTP endpoints follow the same transport
validation as their saved definitions; external endpoints require HTTPS.
Credential references are generated by the host; settings responses contain safe
metadata rather than environment selectors or secret bytes. Retained project revisions
live in the protected chat state and preserve unrelated connection generations when
only models change. Authentication/destination changes retire that connection's old
credential/OAuth namespace.

Static provider/MCP credentials default to the current user's own protected records.
Explicit shared static keys occupy a separate project scope. MCP authorization-code
flows are separate from named manual credentials. The credential-store HTTP surface
returns names, types, revisions and current attachments only. Personal stores bind the
project and caller; project stores require administration permission. One encrypted,
bounded vault record atomically commits the catalog, secret values and attachments.
The catalog accepts at most 256 credentials, 4096 attachment records and the existing
1 MiB vault-record bound. Individual secrets retain the 64 KiB limit. Edits use the
existing lifecycle lock and shared SQL project edit gate, with revision conflicts
requiring a fresh review. Current execution is retired before attached values change.
The runtime resolver accepts only host-composed destination bindings, never a bare
credential ID. Existing connection secrets remain readable and are adopted when their
owner edits or reuses them. Explicit disconnect tombstones prevent legacy values from
reappearing after removal; malformed named custody never falls back to legacy values.

MCP authorization-code
flows use a public OAuth client, fixed HTTPS callback, S256 PKCE, one-use state and exact
user/project/connection binding. The runtime receives an injected headless OAuth vault and never
falls back to a graphical OS credential store. Failures retain categorical public
errors without substituting the operator's or another user's identity.

Personal history ownership is enforced in repository services and storage predicates,
including task/command references, events, inventory and usage aggregates. An owner
with current Execute and Control can answer an exact approval in their personal node
and thread through `respond_personal_task`; `respond_task` retains project Approve for
shared execution. Preserve the runtime's exact interaction ID, ETag, request hash and
mutation identity in both paths. Private threads on shared nodes do not gain the
personal-node approval exception.

Plan refinement and combined approve-and-execute continuation are available only for
an owned personal node and thread with current Execute and Control. Admission binds
the draft revision to the source cloud run/task, native session and exact owner before
calling the ordinary SDK; the runtime independently checks canonical revision and
session ownership. Direct execution and bounded Goal continuation use the same path.
Shared nodes retain their existing Plan admission policy.

Set `chat.shared` to select the shared SQL composition. The server's cloud database
also stores hosted journals/projections under an immutable project/user hash; stream,
idempotency, outbox and projection keys include that namespace. `RuntimeSharedStorage`
injects a fenced journal, projections, keys and stable identity into controlled chat
without changing native Desktop/TUI storage composition. A database-clock lease with
an increasing generation is acquired before writable runtime open. Runtime persistence
and the effect gateway verify that generation; ownership loss drops an adapter future
and keeps an unresolved durable effect start for unknown-outcome recovery.

Project revisions, purpose-bound encrypted vault records, connector enrollment and
one-use OAuth management state live in `hosted_chat_*` SQL tables. API replicas resolve
the execution owner and use existing durable command/resource transport. Settings
mutations coordinate with active owners through a project lifecycle gate. OAuth's
nonserializable protocol session stays at the execution owner; callbacks route there.
Owner loss requires restarting an interrupted login rather than replaying an ambiguous
code exchange. Browser affinity and local API sessions are unnecessary.

`shared_chat_keys` derives encryption/signing material from externally configured
masters per namespace and purpose. Its bounded HTTPS client reads and conditionally
advances an independent anchor authority. The separate `colossus-chat-anchor` binary
uses encrypted SQL CAS records in its own database/administration boundary; placing
anchors beside the runtime journal would weaken rollback detection. Local lexical
indexes are rebuildable on takeover. Shared mode never falls back to local Redb or a
platform credential vault. Omitting `shared` retains the earlier explicit single-host
fixture only when the server sets `local_development`; production requires shared
storage. The Helm hosted values replace the former Kustomize deployment paths.

See [operator enablement](../cloud/deployment/hosted-chat.md)
for exact references, independent authority deployment, restore and migration limits.
The Helm hosted example uses the chart's replicas with disposable caches; it does not provision
an execution environment or an external secret manager.

This is controlled chat rather than complete Desktop/TUI parity. Provider/model choice,
conversation streaming and policy-governed remote MCP reuse existing runtime behavior.
Coder/Kubernetes environment provisioning, filesystem/shell tools, stdio MCP, native
browser/computer control and plugin execution need separate environment contracts.
Cloud attachment/artifact transfer, branch mappings, standalone Plan approve/discard
and explicit plugin-skill selection remain unavailable at the cloud task boundary.

## Work on cloud storage and synchronization

The relational tables are separated by domain: project membership, host/agent/workspace
inventory, conversation threads/messages, tasks, commands/receipts, session/run mappings,
enrollment and certificate rotation. `CloudStore` exchanges typed domain values, and
the PostgreSQL adapter maps ordinary fields to authoritative relational columns and
reconstructs domain objects on reads. Core records have no JSONB document column;
private Diesel row models perform explicit Rust conversions and bind/read native
column values without a whole-object JSON intermediate. The generic storage port
retains shared transactions and revisions while PostgreSQL row types stay inside its
adapter. Canonical JSON encoding is used separately for audit hashes.
Login display metadata uses child rows and bounded sets use native arrays. JSONB is
reserved for SDK requests/snapshots, command operations/replies, runtime policy
observations, released events and opaque authorization envelopes. Queries filter
named columns and use SQL `NULL` for an absent command reply. The cloud store commits
related entity revisions, command delivery,
released events and sync cursors with audit and outbox records in the same transaction.
Entity compare-and-swap and per-feed ordering replace the old global journal-head lock.

Lists use bounded project/account-scoped queries and timestamp/identifier cursors.
Personal owner predicates are applied before pagination and operational aggregation;
legacy rows with no owner retain shared project visibility. Thread detail
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
after bootstrap; restart does not overwrite administrator edits. Explicitly assigning a
built-in role replaces a legacy customized permission set with that role's exact bundle.
Every request and SSE revalidation checks the active user, session security epoch and
current project membership. All replicas share authentication configuration and keys.
Session and PKCE-flow hashes bind the issuer/client namespace; changing identity
configuration requires new browser sign-ins.

`CloudStore::maintain` drains coalesced project wakeup hints with `FOR UPDATE SKIP LOCKED`,
marks publication and prunes delivered hints older than one day. Minute maintenance
passes also remove expired browser sessions and consumed expired OIDC flows in bounded
batches of 256 records per collection. Maintenance metadata remains unexpired.
The maintenance contract never deletes canonical thread/run events, messages, tasks,
cursors or audit chains; their archive boundary requires an explicit retention design.

Numbered SQL migrations under `crates/colossus-cloud-postgres/migrations` run transactionally
at store startup under a schema-specific advisory lock. Applied checksums are persisted.
The initial migration is a fresh relational baseline. Recreate disposable development
schemas from older baselines; no cloud data import is supported. Add a new migration
for a deployed schema; do not change already applied SQL. Schema
rollback files are destructive operator tools and are never executed by ordinary startup.
The database and independently archived audit heads require tested restore procedures;
[cloud operations](../cloud/deployment/operations.md)
owns those steps and the credential references.

[Cloud storage initialization](../cloud/deployment/configuration.md#use-an-external-database)
owns the installed-binary startup and database preparation steps. Cloud schema changes
never migrate or open local runtime journals.

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
cargo xtask quick test -p colossus-cloud -p colossus-cloud-chat -p colossus-cloud-protocol -p colossus-cloud-postgres
cargo xtask quick test -p colossus-cloud-server -- --lib
cargo xtask quick test -p colossus-credentials -- --test headless
cargo test --locked -p colossus-sidecar --test native_lifecycle -- --ignored
cargo xtask pr --base origin/main
```

Use the focused package commands while iterating. The final PR gate includes the
complete Rust gate and other selected components; do not also run
`cargo xtask check rust` on the same source. Control Plane acceptance also requires
Desktop (which includes the web renderer) and documentation validation. Run
`cargo xtask check desktop` and `cargo xtask check docs` separately if the PR gate
did not select them; a cloud-crate-only change does not select those surfaces.

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
The shared hosted Chat acceptance requires two independently disposable **local**
PostgreSQL databases. Supply their URLs through `COLOSSUS_CLOUD_TEST_DATABASE_URL`
for controller/journal state and `COLOSSUS_CHAT_TEST_ANCHOR_DATABASE_URL` for the
independent authority. It creates fresh schemas, synthetic keys and an isolated
child process; no external provider credentials or OS credential store are used.

```sh
cargo test --locked -p colossus-cloud-server --lib \
  auth::tests::transport::hosted::shared::shared_hosted_chat_two_replicas \
  -- --ignored --exact --test-threads=1
```

The fixture runs two real API/execution replicas with separate caches and a stable
mutual-TLS ingress. It verifies cross-replica chat/settings and provider catalog routing, personal credential
isolation, OAuth begin/callback routing with session ownership and replay checks,
expired-owner fencing, connector custody retaining its captured generation,
cache-free takeover, retained messages/session/OAuth custody and idempotent command
replay without another execution.

The lower-level database conformance uses a separate explicit reference:

```sh
cargo test --locked -p colossus-cloud-postgres --lib \
  postgres_atomic_recovery_and_replica_conformance -- --ignored
```

The personal-history conformance additionally proves account filtering before pages,
counts and token usage, private node/resource admission, legacy shared history,
owner-bound invitations, retained audit decoding and cross-account cursor rejection:

```sh
cargo test --locked -p colossus-cloud-postgres --lib \
  postgres_personal_history_filters_before_pagination_and_aggregates -- --ignored
```

Inject `COLOSSUS_CLOUD_TEST_DATABASE_URL` with the disposable fixture connection URL.
That test owns a generated `cloud_acceptance_` schema. It verifies atomic state/event/cursor
commits, optimistic contention, chronological pages, notification fanout, shared browser
sessions, one-use consumption, lease fencing and changed retained-record detection.

The hosted PostgreSQL acceptance tier also runs every ignored Control Plane repository
test, including terminal role migration, replica route lookup, and fenced capability
replacement. Run that suite serially against the disposable database because notification
hints are database-wide even when rows use separate generated schemas. Supply the
additional synthetic `COLOSSUS_CLOUD_TEST_AUDIT_SIGNING_SEED` (64 hex characters) for the
checkpoint fixture:

```sh
cargo test --locked -p colossus-cloud-postgres --lib -- --ignored --test-threads=1
```

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
