---
title: Operate Colossus Control Plane
description: Deploy the PostgreSQL-backed control plane, enable optional personal hosted chat, and operate enrolled runtimes.
audience: operator
type: how-to
---

# Operate Colossus Control Plane

Colossus Control Plane connects explicitly enrolled agents, retains their released conversation
history, and queues human work on a fixed runtime. Each agent opens an outbound
connection; its workspace and daemon need no inbound public port. The control plane
owns project membership, placement and its PostgreSQL schema. Local runtimes retain
their own journals, provider credentials, tools, policy, approvals and sandboxing.
Optional built-in Chat runs a separate controlled runtime for each project/user on the
server; it has its own protected state and deployment requirements below.

## Deploy on Kubernetes

Provision PostgreSQL with a dedicated database role and cloud schema. The role must
own the schema and its tables so startup can apply versioned migrations; it does not
need access to runtime journals or other application schemas. Configure verified
TLS and keep database credentials in your normal secret manager.
Released conversation records are readable by the database role; configure database,
disk and backup encryption through your deployment platform.

Use `deploy/control-plane/kubernetes`. Pin the server image to an exact published
release or digest, replace both example hostnames, and configure `control-plane.json`.
The template also deploys release-matched public documentation at `/docs/`; pin the
documentation image and retain the Ingress prefix as described in
[documentation hosting](documentation-hosting.md).
The server binary is `colossus-control-plane`; `colossus-cloud-server` remains a
compatibility entry point for existing deployments.

The default template supports on-prem installations without an identity provider.
Set `local_auth` to configure local sessions and provision the first administrator
with `bootstrap_admin.username`, `display_name`, and a `password_variable` secret
reference. No default password or public registration exists. The initial password
must contain at least 15 characters. Include a `bootstrap-password` key in the state
Secret for initial provisioning; after an administrator exists, bootstrap does not
replace that account or its credential. Administrators create, suspend, reactivate,
and reset accounts in **Administration → Users**. Password resets and suspension
invalidate existing sessions. Password hashes use Argon2id; login throttling is shared
across replicas.

To use an identity provider, configure `oidc` with its exact issuer, client, and
human-readable `label`, such as `Keycloak`. Register the exact browser
`/auth/callback` URL. Sign-in uses authorization code, S256 PKCE, state, and nonce.
The login button displays the configured provider label. Set the explicit
`bootstrap_admin.oidc_subject` for the first global administrator. OIDC users are bound
to the verified issuer and subject; email does not automatically link accounts.
Local and OIDC sign-in can be enabled together, or either can be used independently.

Users, project memberships, and project hierarchy live in PostgreSQL. The legacy
`memberships` configuration is a one-time migration seed; subsequent configuration
restarts do not overwrite administrator-managed database roles. A global administrator
can inspect shared project resources and manage users, but cannot read another user's
personal chat history or runtime. Administration gives no implicit execution or effect
approval grant. Project roles explicitly bundle permissions: Viewer reads; Operator
reads, executes, and controls; Approver reads, controls, and answers effect approvals;
Project administrator reads, executes, controls, and administers that project. Every
remote operation still intersects the enrolled runtime's independent local grant.

Projects can contain child projects. Membership remains explicit at each project;
parent membership does not silently grant access to children. Archived projects retain
readable history and administration while execution and control are disabled. Manage
hierarchy in **Administration → Projects** and membership in the project's **Access**
view. **New project** opens the dedicated `/projects/new` page; Cancel returns to the
project directory. Creating projects requires global administrator access.
**Home** summarizes visible projects; **Fleet** opens the agent table, then an
agent's workspace/thread sidebar. **Analytics** reports retained run outcomes and
provider usage. Missing usage and unconfigured prices remain unknown.

Major pages and resources have stable browser URLs. Copy the address bar to share a
project view, agent overview or policy, conversation, or task. For example,
`/projects/example/analytics` opens project analytics, and
`/projects/example/threads/<thread-id>` opens one conversation. Browser Back and
Forward follow navigation history; a page's Back control uses the previous in-app
page or its parent when opened directly. Ordinary links also support opening a new
tab.

Shared links require sign-in and the recipient's existing project permissions. A
link does not grant access, select a different account, or enroll a runtime. Sign-in
preserves the requested page, including an OIDC round trip in the same browser tab.
Route browser requests through to the Control Plane server so direct resource links
can load the application; keep `/api`, `/auth`, and health requests on their existing
server handlers.

Browser-session and PKCE-flow identities are bound to the configured issuer and client
ID. Changing either invalidates existing browser cookies and in-progress sign-ins;
users must authenticate again under the new identity configuration.

The database block uses camel-case keys; server-level keys use snake case:

```json
{
  "database": {
    "connectionVariable": "COLOSSUS_CONTROL_PLANE_DATABASE",
    "schema": "colossus_control_plane",
    "tls": { "kind": "webpki_roots" },
    "maxConnections": 16,
    "connectionTimeoutMs": 5000,
    "statementTimeoutMs": 15000
  },
  "auth_key_variable": "COLOSSUS_CONTROL_PLANE_AUTH_KEY",
  "maintenance": {
    "batchLimit": 256,
    "deliveredOutboxRetentionSeconds": 86400
  }
}
```

Supply a PostgreSQL connection URL in `COLOSSUS_CONTROL_PLANE_DATABASE`; the configuration
contains only its environment-variable name. `webpki_roots` verifies the database
hostname and certificate chain. For a private CA, use
`{"kind":"custom_ca","caPemPath":"/etc/colossus/tls/database-ca.pem"}` and add that
PEM file to the mounted TLS Secret. Production rejects disabled database TLS.

Generate a separate random 32-byte hex OIDC-flow encryption key and store it in a
protected file for Secret creation. All replicas must receive the same key. This key
protects the PKCE/nonce sign-in flow stored in PostgreSQL; it is independent of runtime
journal encryption and machine certificate keys.

```sh
kubectl create namespace colossus
kubectl -n colossus create secret generic colossus-control-plane-state \
  --from-file=database=/secure/cloud/database-url \
  --from-file=auth-key=/secure/cloud/auth-key \
  --from-file=bootstrap-password=/secure/cloud/bootstrap-password
kubectl -n colossus create secret generic colossus-control-plane-oidc \
  --from-file=client-secret=/secure/cloud/oidc-client-secret
kubectl -n colossus create secret generic colossus-control-plane-tls \
  --from-file=ca.pem=/secure/cloud/ca.pem \
  --from-file=ca-key.pem=/secure/cloud/ca-key.pem \
  --from-file=server.pem=/secure/cloud/server.pem \
  --from-file=server-key.pem=/secure/cloud/server-key.pem
kubectl -n colossus create secret tls colossus-browser-tls \
  --cert=/secure/cloud/browser.pem --key=/secure/cloud/browser-key.pem
kubectl apply -k deploy/control-plane/kubernetes
kubectl -n colossus rollout status deployment/colossus-control-plane
```

The deployment has two replicas, rolling updates and a disruption budget. Each
replica uses a bounded asynchronous database pool plus one notification-listener
connection: with the default pool, allow up to 34 database connections for two
steady-state replicas and 51 while a third pod rolls in. Reserve capacity for backup,
monitoring and administration. Change the pool or replica count only with measured
connection and event-rate budgets.

Browser sessions, encrypted OIDC flows, durable commands and runtime ownership leases
are shared in PostgreSQL. Connection fencing prevents a previous replica from committing
runtime updates after a replacement obtains ownership. The service applies checksummed
SQL migrations under a schema-specific advisory lock; independent replicas can start
together. Changed migration SQL fails startup instead of silently changing the schema.
The default chat-disabled deployment needs no journal-anchor PVC. Its history and
operational state live in PostgreSQL; runtime journals remain local to their runtimes.
The optional hosted-chat deployment additionally retains its runtime journals and
credential custody on a private PVC.

Browser HTTPS terminates at the trusted ingress. The separate runtime hostname on
port 443 passes TCP/TLS through to port 8443, preserving HTTP/2 and client certificates.
The runtime server leaf must cover that hostname and chain to the configured connector
CA. The ingress example uses ingress-nginx's
[SSL passthrough](https://kubernetes.github.io/ingress-nginx/user-guide/tls/#ssl-passthrough);
enable its controller flag or use your environment's equivalent TCP forwarding.
Keep browser SSE buffering disabled. `/health/live` reports the server process;
`/health/ready` probes the database. SIGTERM closes streams and drains the server so
connectors can reconnect and resume from committed cursors.

Coordinate auth-key rotation across all replicas. An in-progress sign-in encrypted
with an old key must restart after rotation; existing browser sessions use independent
hashed identities in PostgreSQL. Preserve the configured connector CA and enrollment
records when rolling the control plane. Pooling and replicas provide concurrency
mechanisms; deployment fleet capacity still needs a representative load measurement.

Home and project analytics show daily run activity for the last 7, 30, or 90 days.
The line chart and its accessible data table use the same retained UTC daily buckets;
tooltips show actual counts. Active runs and queued work show current state regardless
of the selected period. Failed runs describe execution outcomes, not policy violations.

## Enable personal hosted chat

Open **Chat**, select a project, and start a conversation after the operator enables
that project's hosted service. A workstation enrollment is unnecessary. Project
administrators use Chat's settings to configure providers, model profiles and remote
MCP connections within the deployment's allowed origins. Users select configured
models/reasoning in the composer and connect their own accounts in **Connections**.
Conversation streaming, saved history, cancellation, prompts and exact effect approvals
reuse the normal runtime rather than a separate web agent implementation.
Chat loads execution on demand; viewing saved history or choosing a configured model
does not keep a runtime running. Accepted work continues independently from the browser
connection. Fleet lists connected runtimes; personal Hosted execution is a separate target.

Open **Hosted** to inspect your personal execution target in the selected project.
There is one target per account/project; its conversations keep separate histories.
Open the agent to see running, waiting and queued work, browse paginated run history,
open a run for its existing cancellation or response controls, and review execution
access. Only the owner's target and runs are visible, including to administrators.
Read-only inventory and history requests do not load execution.

**Load runtime** requires Execute permission. **Release idle runtime** also requires
Control permission and refuses active or uncertain work. It closes unfinished connection
setup while preserving the target, conversations and managed files. This releases
placement temporarily: the next accepted work loads execution again, and automatic idle
retirement still applies. These controls also work through another shared SQL replica.
Project administrators can open the existing project configuration; personal connections
retain their existing account-owned settings.

Attach UTF-8 text, Markdown, CSV or JSON files in the composer, up to four attachments
per submission and 256 KiB per file. Files use account-owned protected artifact storage;
this does not grant a filesystem, shell or coding environment.
Personal hosted conversations also support Plan refinement and combined approval and
execution, with direct execution or a bounded Goal. Continuation requires the exact
owned draft revision and session mapping; the runtime rechecks the released request.

### Scale hosted chat across replicas

Use `deploy/control-plane/overlays/hosted-chat-sql` for shared hosted state. Its two
API replicas use the server's `database` block for tenant-partitioned runtime journals,
projections, project settings, encrypted credentials and OAuth callback
metadata. Each API replica can serve the same signed-in user. The database places
one execution owner per project/user runtime; other replicas route operations to that
owner through durable cloud commands. This preserves a conversation's session and
memory without requiring browser affinity.

`chat.max_active_runtimes` bounds resident tenant runtimes per worker process, with a
default of 16 and supported range 1–64. `chat.idle_seconds` defaults to 300 and accepts
1–86400 seconds. Fully idle contexts are retired after that interval or earlier under
capacity pressure; active runs, children, interactions and management resources retain
ownership. Additional accepted work remains durably queued until capacity becomes
available. Worker processes are currently colocated with API replicas.

Upgrade all replicas together when moving from self-enrolled Hosted Chat. The service
converts only exact historical owner/instance bindings to Hosted targets, preserves
their journals and conversations, and retires certificate authentication for those
targets. Ordinary Fleet registrations remain connected targets. Back up state and keys
using the procedure below before upgrading.

Enable the shared composition with these secret references inside `chat`:

```json
{
  "shared": {
    "keys": {
      "journal_variable": "COLOSSUS_CHAT_JOURNAL_KEY",
      "signing_variable": "COLOSSUS_CHAT_SIGNING_KEY",
      "anchor_endpoint": "https://chat-anchors.example.com/v1/anchors",
      "anchor_bearer_variable": "COLOSSUS_CHAT_ANCHOR_TOKEN",
      "anchor_ca_pem_path": null,
      "allow_loopback_http": false
    }
  }
}
```

Keep `wrapping_key_variable` configured for the SQL credential/settings encryption
key. Supply independent 32-byte encryption, signing and wrapping masters through your
secret authority on every executor. Journal and checkpoint keys are derived for the
exact runtime namespace and purpose. SQL stores ciphertext and references rather
than these master keys. A missing key, unavailable authority or invalid ciphertext
blocks the operation; no local credential-store fallback exists.

Run `colossus-chat-anchor /etc/colossus/anchor.json` as a separately administered
service. Give it its own PostgreSQL database, role, backup authority, encryption key
and bearer credential, outside the journal database's administration boundary. It
stores encrypted, monotonic checkpoint attestations and exposes no delete operation.
Point `anchor_endpoint` at its HTTPS ingress. An optional CA bundle configures
additional trust; certificate and hostname verification remain enabled.

For example, its configuration contains references only:

```json
{
  "bind": "0.0.0.0:8080",
  "trustedTlsIngress": true,
  "database": {
    "connectionVariable": "COLOSSUS_ANCHOR_DATABASE",
    "schema": "colossus_chat_anchors",
    "tls": { "kind": "webpki_roots" },
    "maxConnections": 4,
    "connectionTimeoutMs": 5000,
    "statementTimeoutMs": 5000
  },
  "bearerVariable": "COLOSSUS_ANCHOR_TOKEN",
  "storageKeyVariable": "COLOSSUS_ANCHOR_STORAGE_KEY"
}
```

`trustedTlsIngress` is an explicit operator assertion for non-loopback listeners;
the authority process serves HTTP behind that ingress. The authority's storage key
is separate from the three executor keys. Restrict access to the trusted executors.
The explicit `allow_loopback_http` client option is only for isolated development.

The SQL overlay uses disposable `emptyDir` caches and rolling updates. No runtime
Redb database, local vault, anchor file or project-settings file is authoritative in
shared mode. Size local caches and per-replica runtime limits using measured workloads.
An execution lease lasts 30 seconds and renews every five seconds. Database-clock
expiry and increasing ownership generations fence stale runtime opens, admissions,
journal/projection writes, credentials and output release. Loss of ownership cancels
in-flight adapter futures; a durable effect start with an uncertain remote outcome is
reconciled as unknown rather than automatically executed again. A remote service may
already have accepted a request, so lease fencing does not promise exactly-once
external effects without that service's idempotency support.

OAuth begin/callback operations can arrive at any API replica. Shared one-use state
and an encrypted management queue bind each operation to its user, project,
connection generation and execution owner. Losing that execution owner interrupts
an in-progress OAuth exchange; the user restarts sign-in. Previously stored account
credentials remain in encrypted SQL custody. Project-wide settings and credential
changes use database coordination and require affected execution to be idle.

Back up the shared cloud/journal database, retain the matching external keys and
connector CA, and back up the independent anchor authority separately. Restore must
satisfy the retained anchors; rolling the journal back below an attested boundary
fails closed. Do not roll back the authority to hide a journal integrity failure.
Existing single-host data is not silently imported by setting `shared`: keep the old
state and keys until a supervised migration is verified, or use a new project for a
fresh shared deployment.

### Configure and verify the shared deployment

Pin images and replace browser/runtime hostnames and TLS material in the SQL overlay.
Select an existing project ID and explicit project memberships. Global administration
alone does not grant access to another account's conversations. Replace the example
model, token limits, provider and MCP endpoints with reviewed values. The sample shares
the provider key; MCP OAuth uses a personal public client with S256 PKCE and the exact
`https://colossus.example.com/auth/mcp/callback` registration. Remove the example MCP
entry until its endpoint, client and authorization server are ready.

Review exact `sandbox.networkDestinations` origins for the provider, MCP endpoint and
OAuth metadata/token servers. Project administrators can select destinations within
this immutable operator ceiling. They cannot add arbitrary network access. Select
runtime admission, CPU, memory, cache and database budgets using measured workloads.
The example permits four resident user runtimes per process and a 4 GiB memory limit.
Controller readiness checks the database; verify actual hosted execution separately.

After the secret authority has written owner-private files, provision the executor
Secret without embedding values in configuration:

```sh
kubectl -n colossus create secret generic colossus-chat-state \
  --from-file=journal-key=/secure/cloud/chat/journal-key \
  --from-file=signing-key=/secure/cloud/chat/signing-key \
  --from-file=wrapping-key=/secure/cloud/chat/wrapping-key \
  --from-file=provider-key=/secure/cloud/chat/provider-key \
  --from-file=anchor-token=/secure/cloud/chat/anchor-token
kubectl apply -k deploy/control-plane/overlays/hosted-chat-sql
kubectl -n colossus rollout status deployment/colossus-control-plane
```

Provider/MCP authentication defaults to personal custody. Missing, expired or corrupt
personal credentials never select another user's or a shared account. Shared static
credentials require an explicit project choice and remain separate from personal
records. The browser cannot import or repoint a native workstation account file. A trusted
operator can explicitly import a shared service account into encrypted SQL custody;
see the [source import procedure](../develop/cloud-control-plane.md#develop-hosted-personal-chat).
A rotating-token refresh interrupted after its durable claim requires explicit
reauthentication before another refresh attempt.
Project settings return connection metadata rather than host paths, environment key
references or credentials. Destination/authentication changes retire the credential
generation; unrelated model edits preserve existing connections.

In Web settings, use **Global → Credentials** to save or remove your personal API keys
for the selected project. Credentials can be saved before a provider or MCP exists.
Use **Workspace → Credentials** to save shared project credentials with project
administration permission. Use **Attach existing credential** to select a configured
connection and review its destination. Provider and static MCP setup also offer
**Save and choose credential**, which saves the connection before continuing to
credential selection. An unattached credential grants no runtime access. Rotation
updates its existing attachments; removal revokes them. A changed endpoint requires
a new attachment. The forms show the destination and saved status; stored secret values are
never read back into the browser. In a shared deployment these writes use the encrypted
PostgreSQL vault and remain available across API replicas.

Use **Global → Connected accounts** for personal MCP OAuth sign-in, reconnect and
disconnect. ChatGPT/Codex appears separately from manual credentials. Personal
subscription linking is not implemented yet and requires a supported server integration;
**View setup requirements** explains the requirement and links to OpenAI's documentation.
An API key does not link a ChatGPT subscription.

Providers shows the full built-in connection preset catalog used by Desktop, with
deployment availability guidance. Codex account enrollment and local-server endpoints
require deployment configuration; a local address refers to the server. Providers from
Desktop's locally imported setup packages are separate configuration and are not
automatically imported into a cloud project.

Providers and Models use the shared Desktop inventories and editors. **Add provider**,
**Add model**, and **Save changes** persist the reviewed project configuration directly.
Display labels are stored separately from runtime profile IDs and credentials.
In Models, **Load models** discovers the saved provider's catalog through the caller's controlled runtime.
The selected model fills in reported limits and capabilities; manual model IDs remain
available. Discovery requires project administration and execution permission and uses
the configured personal or shared credential. It does not accept browser-supplied
endpoints or secret values. Subscription account sign-in remains deployment-managed.

**Role routing** uses the shared routing map and selectors. A specialized role left on
**Use primary** follows the primary model; only explicit overrides are stored.
Feature **Auto** uses advertised model support and provider responses, with separate
controls for tool calls, streaming, image inputs and server compaction.

Personal nodes and threads belong to the authenticated account. Other project members,
project administrators and global administrators cannot read or control them through
thread, task, event, inventory or analytics routes. Shared links do not transfer
ownership. A personal owner with current Execute and Control can answer their exact
one-use approval; shared execution retains the independent project Approve requirement.
Runtime policy, request hashes and response ETags remain authoritative.

Verify with two ordinary project users, using different MCP accounts. Stream a chat,
answer an approval, reconnect through another API replica and continue the same
history. Neither user should see the other's threads, tasks, nodes, tokens or usage.
Restart the execution owner, discard its local caches, and verify history and connector
recovery. Check that unresolved effects are reported as unknown rather than replayed.

The older `overlays/hosted-chat` path is a compatibility alias for the SQL overlay.
A production `chat` block without `shared` is rejected. The single-host composition
is retained only when `local_development` is explicitly enabled, for source fixtures
with protected local state. Keep its private volume and matching keys until any
supervised migration is verified; pending sign-in flows restart after interruption.

Hosted Chat currently has controlled provider and allowlisted remote MCP tooling.
It does not provision a Coder workspace, Kubernetes sandbox or other user environment.
Shell/filesystem tools, stdio MCP, native browser/computer control and plugin execution
require future environment integration. Cloud attachment/artifact transfer, branch
mappings, standalone Plan approve/discard, and explicit plugin-skill selection remain
unavailable. Choose an independently enrolled runtime for capabilities outside this
hosted boundary.

## Configure deployment markings and monitor policy

A global administrator can configure a plain-text classification banner in
**Administration → Settings**, including tone and top or top-and-bottom placement.
The optional `classification` server configuration seeds the marking only when no
saved display setting exists; subsequent administrator edits survive restarts. The
marking is visible on sign-in and authenticated screens and does not enforce data
classification or change native permissions.

An agent's **Policy** view shows bounded configuration released by that authenticated
runtime: access profile, sandbox, approval mode, grant roles/tools, configured model
labels, and a metadata fingerprint. Fresh observations are runtime-reported, not
independent attestation. Older or disconnected observations are marked stale.
Project administrators can set a monitoring baseline for sandbox profile, approval
modes, and tool ceiling; differences are shown as drift without changing the runtime's
local enforcement. Missing canonical denial/approval counters are unavailable, and
failed runs are not treated as policy violations.

Desktop's global **Control Plane** settings manage named endpoint profiles and show
workspace connections. Selecting a profile only pre-fills its endpoint. Enrollment,
local sharing, and grant confirmation remain explicit per workspace.

## Initialize cloud storage

Prepare a dedicated PostgreSQL database or schema and configure the `database` block
with its protected connection-variable reference. Start `colossus-cloud-server` with
that configuration; startup creates the relational tables and applies checksummed
migrations under a schema-specific advisory lock. The cloud database is separate
from every runtime journal.

The initial cloud schema is a fresh relational baseline. Earlier disposable development
schemas must be recreated before startup; there is no legacy cloud-journal import
command or data conversion. A migration checksum mismatch identifies a schema created
with an earlier baseline. Recreate only that disposable cloud schema, then restart
and verify readiness, sign-in and enrollment. Local runtime storage is unaffected.

For deployed schemas, preserve applied migration files and evolve the database through
new migrations. Keep backups and independently archived audit checkpoints as described
in [cloud backup and restore](#back-up-and-restore-the-cloud-database).

## Enroll a CLI daemon

Start an existing configured worker with an owner-private public API directory.
Stop it briefly to enroll a dedicated application using the same directory and
configuration, then restart it. Offline enrollment refuses another active owner.

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api \
  --enroll-application app:colossus-cloud-cli \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond \
  --role primary --tool user.ask --tool filesystem.read \
  --credential-keyring-service dev.colossus.cloud \
  --credential-keyring-account engineering
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api
```

Choose tools and roles explicitly beneath the runtime's existing ceilings. The native
credential store receives the bearer directly; enrollment output contains public metadata
only. Public API administration is supported on Windows, macOS and Linux with their
native owner-private directory checks.

Create an owner-private `connector.json` containing only public paths, pins and native
credential references. Use the exact values returned by local application enrollment:

```json
{
  "descriptor": "/secure/runtime-api/endpoint.json",
  "certificate": "/secure/runtime-api/certificate.pem",
  "instance_id": "replace-with-enrolled-local-instance",
  "certificate_sha256": "replace-with-exact-64-character-local-pin",
  "keyring_service": "dev.colossus.cloud",
  "keyring_account": "engineering"
}
```

Sign into the web app, select the project, open **Fleet**, and create an
invitation with the reviewed label and role ceiling. On the runtime machine, run:

```sh
colossus control-plane enroll --local-config /secure/connector.json \
  --enrollment-url https://colossus.example.com/api/enroll --name engineering
colossus control-plane run --local-config /secure/connector.json --name engineering
```

In another terminal, inspect or stop that exact named CLI connection:

```sh
colossus control-plane status --name engineering
colossus control-plane disconnect --name engineering
colossus control-plane revoke --name engineering
```

Status reports a bounded, owner-private, non-secret heartbeat independently of the
encrypted credential vault. A stale running heartbeat is reported as `unknown`.
Disconnect requests are bound to one connector process generation; an old request
cannot stop a later connection. Revocation disconnects first, then uses the native
client certificate to revoke only its own enrolled node. Retry a failed revocation
with the same enrollment; forgetting credentials alone does not revoke cloud authority.

Paste the invitation into the stdin prompt. It expires after ten minutes and is consumed
once. The connector generates and retains its private key locally. Enrollment binds the
cloud node to the independently verified local instance. The separate `colossus-connector`
binary accepts the same commands without the `control-plane` prefix. The former
`colossus cloud` command remains an alias for existing scripts.

## Enroll Desktop

Select a Managed Local workspace and open its **Control Plane** settings. Paste the web
invitation and enrollment URL, then review the native confirmation showing the cloud
origin and execution/approval authority. Desktop uses a separate native-held application
credential; the renderer receives connection status and public identities only.
External daemon targets use their separately enrolled CLI connector.

Desktop can disconnect, reconnect, revoke, and forget its enrollment. Closing the cloud
connection leaves accepted tasks running locally. Removing a local enrollment does
not revoke it remotely. Use **Revoke enrollment** in Desktop or **Revoke agent** in
the web fleet to remove cloud authority. Desktop revocation requires native confirmation.

## Operate hosts, threads and tasks

**Fleet** groups hosts, their independently enrolled agents, and workspace identities.
A host is an opaque native-installation identity retained under `COLOSSUS_HOME`; it is
not derived from a machine identifier or a filesystem path. Desktop workspaces using
the same home share a host group. CLI deployments with separate homes remain separate
host groups even when they run on the same computer. Grouping grants no additional
access: every agent keeps its own application grant and project enrollment.

Fleet lists one entry per host with its reported computer name, operating system,
workspace count and connection state. Open a host to see its workspaces together in
the left sidebar. Expand a workspace to browse its threads or select it to inspect
its connection, analytics and policy. Search remains scoped to the opened host;
workspaces from other hosts do not appear in its sidebar.

Creating a workspace invitation requires a project and role ceiling, without a
separate agent name. The enrolled runtime supplies the computer and workspace names;
names are display metadata and never replace stable host, workspace or agent IDs.
Each workspace retains its own enrollment and local grant. Existing enrollment aliases
remain available in connection details.

Start a cloud conversation or open saved history. A thread stays assigned to its agent
and workspace. Each submitted
human message creates an ordered task/run; **Tasks** shows execution requests across
the project's agents and opens their associated conversations. Queue acceptance is
separate from the runtime's receipt. The first turn must establish its native session
before a follow-up can be submitted.

Thread output streams through durable ingestion and browser SSE. Reconnection resumes
from committed cursors. Saved messages remain readable when the agent is offline; new
messages in an established conversation queue for that agent to reconnect. The UI marks
incomplete history and output limits explicitly and offers older history pages. Rename
and archive require `control`; message submission requires `execute`. Archive changes
conversation visibility and does not cancel accepted work.

Released session-activity excerpts can be bounded by the runtime, including its 64 KiB
text limit. The UI marks bounded history; it does not promise that every omitted byte
will backfill. Continuation uses the agent's canonical native session context rather
than rebuilding context from the cloud's excerpts.

Local sessions stay private by default. In Desktop's **Control Plane** settings, choose
**Share Desktop history for viewing** or **Share history and allow continuation** and
save **Desktop conversation sharing**. Enabling sharing requires native confirmation.
Sharing includes the source application's existing and future sessions in that
workspace. Disabling sharing stops future synchronization and continuation; already
synchronized cloud history remains retained.

For a CLI deployment, use the source application's protected connection configuration,
not the cloud recipient's bearer. The source caller must have `runs:read` and
`runs:control`, and `runs:execute` when allowing continuation. The recipient must be a
separately enrolled application on that runtime:

```sh
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli --allow-continuation
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli --disable
```

The first command shares history for viewing; the second also allows new recipient-owned
runs to continue the source conversation under the recipient's own grant. Imported
source runs remain read-only: they cannot inherit approval or cancellation authority.
The cloud cannot inspect arbitrary local applications or grant itself sharing access.

## Install and supervise runtime connections

CLI and standalone connectors use the same outbound protocol on Windows, macOS and
Linux. Use the platform's native credential authority for interactive installations:
Windows Credential Manager, macOS Keychain, or Linux Secret Service. A graphical
credential authority must be unlocked for enrollment, connection and rotation. Use
the explicit sealed headless authority below for unattended hosts that lack one.

Run the daemon and connector as the same selected operating-system user, with distinct
process lifetimes. On Linux, supervise both with systemd; on macOS, use launchd;
on Windows, use your service supervisor or Task Scheduler. Preserve the configured
owner-private runtime and Colossus home directories across restarts. Select the exact
enrollment name and local configuration in the connector's supervised command. Inject
headless wrapping keys through the supervisor's protected secret environment.

For concurrent CLI connectors, give each daemon/connector pair its own owner-private
`COLOSSUS_HOME`. A native vault retains one process's exclusive lease while the
connector is running; named enrollments in one home are selected serially. Desktop
shares its native vault within the application and can connect multiple workspaces.

Use `cloud disconnect` or Ctrl-C for graceful connector shutdown. This leaves the
daemon and accepted runs alive. Stop the worker separately with its normal graceful
shutdown when ending local execution. Revoke before uninstalling a enrolled connector,
then use `cloud forget` to delete its native enrollment. Desktop's embedded connector
and Managed Local runtime end when Desktop exits; persistent execution requires a
separately supervised CLI daemon.

Linux Desktop is distributed as an unsigned Developer Preview `.deb` for Debian 12
compatible systems, in native x86-64 and ARM64 builds. Verify the accompanying SHA-256
checksum and install the package with your distribution's package manager. It includes
sealed sidecar, CLI and search executables and requires GTK 3, WebKitGTK 4.1, Secret
Service and ALSA. A running unlocked Secret Service is required for native credentials.
Launch `colossus-desktop` or the desktop menu entry. Revoke enrolled workspaces before
removing the package. User workspace state remains in the configured Colossus home.

## Use an explicit headless authority

Linux servers without Secret Service can inject an operator-owned random 32-byte hex
wrapping key through a secret manager. Set the worker's
`--public-api-vault-key-variable COLOSSUS_HEADLESS_KEY` for both offline enrollment and
normal hosting. Its public API credentials are sealed beneath the public API directory's
`headless-credentials` child. Add the non-secret selector to `connector.json`:

```json
{
  "headless_authority": {
    "directory": "/secure/runtime-api/headless-credentials",
    "key_variable": "COLOSSUS_HEADLESS_KEY"
  }
}
```

Retain all the earlier connector fields. Supply
`--vault-key-variable COLOSSUS_HEADLESS_KEY` to connector enrollment and lifecycle
commands. The injected key protects the vault's small key envelope; the daemon bearer
and connector TLS private key remain sealed on disk. There is no fallback to plaintext
or another credential store when that authority is missing or changed.

## Back up and restore the cloud database

Use your PostgreSQL platform's encrypted backups and point-in-time recovery. Retain
cloud certificate CA material, server keys, the OIDC configuration and flow key in your
secret-management backup; database snapshots alone do not restore these credentials.
Runtime journals and native connector vaults need their own existing backup policy.

For a portable logical backup, configure a protected libpq service and password file
with verified TLS. The commands below use the example `colossus_cloud` schema and keep
connection passwords out of command arguments:

```sh
umask 077
pg_dump --dbname='service=colossus-cloud-backup' --format=custom \
  --schema=colossus_cloud --file=/secure/backups/colossus-cloud.dump
pg_restore --list /secure/backups/colossus-cloud.dump
```

Protect exported dumps with your backup system's encryption and access controls.

Retained entities and events have per-resource audit hashes. These detect changed
retained content, but an administrator can replace the database and its hashes together.
They do not reproduce the runtime journal's independent secure-anchor rollback protection.
Retain audit head checkpoints in an independently protected archive. For an exact
checkpoint alongside a logical backup, pause cloud writers by scaling its deployment to
zero, take the dump, and export the retained audit heads before restarting:

```sh
kubectl -n colossus scale deployment/colossus-control-plane --replicas=0
pg_dump --dbname='service=colossus-cloud-backup' --format=custom \
  --schema=colossus_cloud --file=/secure/backups/colossus-cloud.dump
psql --dbname='service=colossus-cloud-backup' --set=ON_ERROR_STOP=1 \
  --command='COPY (SELECT DISTINCT ON (project_id,entity_kind,parent_id,id) project_id,entity_kind,parent_id,id,revision,chain_hash FROM colossus_cloud.cloud_audit ORDER BY project_id,entity_kind,parent_id,id,revision DESC) TO STDOUT WITH CSV HEADER' \
  > /secure/backups/colossus-cloud-audit-heads.csv
kubectl -n colossus scale deployment/colossus-control-plane --replicas=2
kubectl -n colossus rollout status deployment/colossus-control-plane
```

Accepted runtime work continues while cloud writers are paused. Protect or sign the
export with your independent archive process; keeping it in the same mutable database
is not an independent checkpoint.

Restore first into an empty isolated database owned by the cloud role. Use a separate
protected service reference and the same schema name:

```sh
pg_restore --dbname='service=colossus-cloud-restore' --single-transaction \
  --exit-on-error --no-owner /secure/backups/colossus-cloud.dump
```

Verify migration checksums, enrollment/revocation records, task/run identities, receipts,
sync cursors and audit heads against the archived checkpoint. Before production cutover,
stop all cloud replicas, update their database Secret and restore the matching secret
material. Reconcile certificate renewals and revocations newer than the backup, then
restart and verify original-agent replay. A database restored to an earlier point may
not contain later accepted commands; inspect the original runtime outcomes before
resubmission. Keep an independently restorable pre-cutover backup.

Operational maintenance runs each minute in bounded batches of 256 records per
collection. It republishes pending project wakeup hints and marks them delivered,
retains published outbox metadata for one day, and removes expired browser sessions
and consumed expired OIDC flows. These hints carry project identities only; duplicate
notifications are safe because readers reload durable state from their cursors.
Pending hints are coalesced per project. Import and maintenance markers have no expiry
and remain retained. The optional `maintenance` block accepts `batchLimit` from 1 to
1024 and `deliveredOutboxRetentionSeconds` from 60 to 2592000 seconds. These settings
change operational metadata cleanup, while conversation retention remains explicit.

Archiving a thread retains its messages and runs. Maintenance leaves canonical events,
messages, tasks, receipts, cursors and audit records intact. Establish project retention
periods and independently retained audit checkpoints before adding a schema-aware cleanup procedure. Do not prune
`released_events`, sync cursors, receipts, audit heads or pending outbox rows independently;
they jointly establish replay and reconciliation. Database access and backup retention
must reflect the sensitivity of released conversation content.

## Verify and recover

The fleet shows host/workspace inventory and a ready agent after its authenticated
heartbeat. Start a short thread, verify live output and its terminal state, send a
follow-up, then reopen saved history with the agent offline. Verify pending prompts,
approvals and cancellation with their exact runtime receipts.
Approval requires the independent project `approve` permission plus the native local
grant. Cancellation and ordinary prompt responses require `control`.

Connector certificates last thirty days and renew with a new key at seven days
remaining. `colossus control-plane renew --name engineering` explicitly rotates a certificate;
use the same native vault selector for headless deployments. Interrupted enrollment and
renewal retain their exact pending exchange for retry. Expired or revoked enrollment
requires operator reconciliation or a new reviewed invitation.

Cloud and connector restarts replay the same task/run allocation and exclusive durable
output cursor. Disconnection never moves accepted work to another machine. An unknown
outcome stays explicit; inspect the original local run before deciding whether to submit
new work. Never automatically retry it with a fresh task identity.

Each node admits sixteen active tasks. Each task retains at most 1,000,000 output events and
256 MiB of released output. Reaching that bound marks the web task explicitly and switches
to snapshot/control updates; full runtime output remains local. Sessions are bounded by
the verified ID-token expiry and eight hours; sign in again to resume observation.

Cloud sandbox provisioning, artifact/context transfer and scheduled cloud workflows
require separate ownership contracts. Existing local sandbox and policy behavior still
applies to every task. Source and acceptance commands are in
[cloud development](../develop/cloud-control-plane.md).
