---
title: Operate Colossus Control Plane
description: Deploy the PostgreSQL-backed Kubernetes control plane, enroll runtimes, and operate shared conversations.
audience: operator
type: how-to
---

# Operate Colossus Control Plane

Colossus Control Plane connects explicitly enrolled agents, retains their released conversation
history, and queues human work on a fixed runtime. Each agent opens an outbound
connection; its workspace and daemon need no inbound public port. The control plane
owns project membership, placement and its PostgreSQL schema. Local runtimes retain
their own journals, provider credentials, tools, policy, approvals and sandboxing.

## Deploy on Kubernetes

Use the Helm chart from Docker Hub on Kubernetes 1.34 or newer. The chart replaces
both former Kustomize deployments and targets GKE, EKS, OpenShift, and disconnected
installations. Provision DNS, three schedulable zones, a persistent StorageClass,
a resource metrics API for autoscaling, and your selected ingress/gateway controller.
The controller and its CRDs are platform prerequisites.

Create a values file with your two hostnames and routing choice, then install the
release-matched chart:

```sh
helm upgrade --install colossus-control-plane \
  oci://registry-1.docker.io/obscuritylabs/colossus-control-plane-chart \
  --version X.Y.Z --namespace colossus --create-namespace \
  -f deployment-values.yaml
kubectl -n colossus rollout status deployment/colossus-control-plane
kubectl -n colossus rollout status deployment/colossus-control-plane-docs
```

The [chart values and examples](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart)
own the complete configuration, Secret key contracts, routing prerequisites and
OpenShift/offline overrides. Pin both server and documentation images to reviewed
release tags or digests. Wait for all image and chart publication workflows before
using a new source version. The server image includes the browser application;
release-matched public documentation is deployed separately at `/docs/` by default.

The default configuration uses local accounts. The chart generates a random initial
administrator password, a shared flow-encryption key, connector certificates, and
separate application/administrator database credentials. Read the initial password
privately from Secret `colossus-control-plane-state`, key `bootstrap-password`.
There is no default password or public registration. Bootstrap does not reset an
existing administrator's credential. All generated durable identity Secrets are retained
on Helm uninstall and reused on reinstall under the same release name and namespace.
Back them up alongside the database. Existing Secrets can replace every generated
credential; use stable existing Secrets for GitOps and repeated offline rendering.

Optional OIDC supports GitLab and Keycloak through ordinary issuer/client configuration.
Register the exact `https://BROWSER_HOST/auth/callback` URL and explicitly set the
first administrator's verified subject. Local and OIDC sign-in can coexist, or local
sign-in can be disabled. Accounts bind to verified issuer and subject; email never
links accounts automatically. OIDC certificates must be trusted by the server's OIDC
client independently of database CA settings.

### Database, replicas and storage

The chart installs a pinned PostgreSQL dependency with one persistent instance and
a dedicated non-superuser application role. This database uses explicit plaintext
`in_cluster` transport to its exact same-namespace Service hostname. A database ingress
NetworkPolicy restricts access to server pods; ensure your CNI enforces it. This is an
operator-selected cluster trust boundary, not proof of network isolation or encryption.
The database is a single point of failure; select external managed PostgreSQL for
database HA.

For RDS or an existing database, disable `postgres.enabled` and supply the connection
URL through a Secret. The role must own its dedicated database/schema so startup can
apply checksummed migrations. External database TLS is mandatory: public trust roots
or a mounted private CA bundle both verify the certificate and hostname. The loopback
fixture's disabled-TLS mode remains unavailable to production deployments.

Default autoscaling spans three to six server replicas. Each uses a pool of eight
connections plus a notification listener: 27 at three pods, 54 at six, or 63 during
one-pod rolling surge. Reserve additional PostgreSQL capacity for backup and administration.
Hard zone spreading distributes three replicas across three eligible availability zones.
For more zones, raise the replica minimum to at least the number of zones; smaller
clusters require explicit topology overrides. The default resources are initial budgets
for ten users with agents, rather than a measured capacity guarantee.

Server pods need no persistent volume. PostgreSQL owns cloud state, browser sessions,
OIDC flows, queues, history and fenced runtime leases; local runtime journals remain
local. Migrations run at startup under a schema-specific advisory lock, so replicas can
start concurrently. Helm rollback does not reverse migrations. Follow the
[backup and restore procedure](#back-up-and-restore-the-cloud-database) before upgrades.

### Browser and connector networking

Browser HTTPS terminates at the trusted controller; keep SSE buffering off and preserve
`/api`, `/auth`, `/docs/` and direct application URLs. Connector TLS passes through to
port 8443 with its original client certificate and HTTP/2 stream. It must never be
terminated and re-originated by the ingress controller.

The chart supports ingress-nginx, Istio VirtualServices with injected sidecars or
ambient workloads, and Gateway API HTTPRoute/TLSRoute. Each routing controller needs
an HTTPS browser listener and a distinct connector hostname with TLS passthrough.
ingress-nginx requires `--enable-ssl-passthrough`. Gateway API needs TLSRoute-capable
CRDs/controller; Kubernetes alone does not install them. Platform operators own gateway
listeners, browser HTTPS certificates, mesh admission and network policy. See the chart's
listener examples and the [documentation hosting guide](documentation-hosting.md).

`/health/live` reports process health; `/health/ready` probes the database. SIGTERM
closes streams and drains for up to thirty seconds; pods receive a forty-five-second
grace period so connectors can reconnect from durable cursors. The generated connector
server leaf must be renewed before expiry. Changing `tls.renewalRevision` reissues it
under the retained CA and rolls pods. Externally supplied Secret changes require a
pod rollout; identities are read at process startup.

## Sign-in and project administration

Users, project memberships, and project hierarchy live in PostgreSQL. The legacy
`memberships` configuration is a one-time migration seed; subsequent configuration
restarts do not overwrite administrator-managed database roles. A global administrator
can inspect all projects and manage users, but receives no implicit execution or effect
approval grant. Project roles explicitly bundle permissions: Viewer reads; Operator
reads, executes, and controls; Approver reads, controls, and answers effect approvals;
Project administrator reads, executes, controls, and administers that project. Every
remote operation still intersects the enrolled runtime's independent local grant.

Projects can contain child projects. Membership remains explicit at each project;
parent membership does not silently grant access to children. Archived projects retain
readable history and administration while execution and control are disabled. Manage
hierarchy in **Administration → Projects** and membership in the project's **Access**
view. **Home** summarizes visible projects; **Fleet** opens the agent table, then an
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

Coordinate auth-key rotation across all replicas. An in-progress sign-in encrypted
with an old key must restart after rotation; existing browser sessions use independent
hashed identities in PostgreSQL. Preserve the configured connector CA and enrollment
records when rolling the control plane. Pooling and replicas provide concurrency
mechanisms; deployment fleet capacity still needs a representative load measurement.

Home and project analytics show daily run activity for the last 7, 30, or 90 days.
The line chart and its accessible data table use the same retained UTC daily buckets;
tooltips show actual counts. Active runs and queued work show current state regardless
of the selected period. Failed runs describe execution outcomes, not policy violations.

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
with verified TLS. The commands below use the example `colossus_control_plane` schema and keep
connection passwords out of command arguments:

```sh
umask 077
pg_dump --dbname='service=colossus-cloud-backup' --format=custom \
  --schema=colossus_control_plane --file=/secure/backups/colossus-cloud.dump
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
  --schema=colossus_control_plane --file=/secure/backups/colossus-cloud.dump
psql --dbname='service=colossus-cloud-backup' --set=ON_ERROR_STOP=1 \
  --command='COPY (SELECT DISTINCT ON (project_id,entity_kind,parent_id,id) project_id,entity_kind,parent_id,id,revision,chain_hash FROM colossus_control_plane.cloud_audit ORDER BY project_id,entity_kind,parent_id,id,revision DESC) TO STDOUT WITH CSV HEADER' \
  > /secure/backups/colossus-cloud-audit-heads.csv
kubectl -n colossus scale deployment/colossus-control-plane --replicas=3
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
