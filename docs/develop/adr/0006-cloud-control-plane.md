---
title: "ADR 0006: Cloud control plane and outbound connections"
description: Cloud protocol, project authority, enrollment trust, native grants, lifecycle and durable recovery decisions.
audience: developer
type: concept
---

# ADR 0006: Cloud control plane and outbound connections

- Status: accepted
- Date: 2026-10-05

## Context

Workstations and independently supervised daemons must accept cloud work behind NAT
without exposing the runtime's loopback API. A browser cannot hold runtime credentials
or inherit the Desktop caller's authority. Stream reconnection alone does not establish
whether a mutation was accepted or whether output was retained durably.

Fleet inventories and long-lived conversations need indexed project, host, workspace
and history queries. Independent agents must commit their events concurrently without
a cloud-wide journal head. Cloud metadata, connection ownership and browser sessions
need their own lifecycle, while native execution journals retain their separate
recovery boundary. Host grouping must not disclose unrelated applications' sessions.

## Decision

Use a separate cloud application and a reusable native connector. `colossus-cloud`
owns project/host/agent/workspace/thread operations and a cloud-owned asynchronous
storage port. `colossus-cloud-postgres` implements that port with Diesel/diesel-async,
a bounded pool and an independent relational PostgreSQL schema. `colossus-cloud-server`
composes OIDC, HTTP/JSON, SSE and certificate issuance. Cloud storage does not implement
the runtime journal port; local runtime journals retain their existing protections.
`colossus-cloud-protocol` owns a separate major-versioned Protobuf bidirectional stream
with closed typed SDK operations. `colossus-connector` implements enrollment and the
outbound stream for CLI, standalone and native Desktop hosts.

Browser authentication uses OIDC authorization code with PKCE, verified issuer,
audience, signature, nonce, expiry and access-token hash. Explicit subject/project
memberships grant independent read, execution, control, approval and administration
permissions. Opaque browser sessions use protected cookies and same-origin CSRF checks.
Configured memberships form an authority ceiling and reconcile changed/removed persisted
grants at startup. Every request intersects local configuration with database permissions;
replicas share one configuration. Browser-session and PKCE-flow identities bind the issuer
and client namespace so an identity-provider/client change cannot reuse old cookies.
Browser code receives released views and one-use invitations, never native bearers,
provider keys or machine private keys.

Machine connections use mutual TLS 1.3 over HTTP/2, normally on port 443. Kubernetes
connector ingress preserves end-to-end TLS. One-use, expiring invitation redemption
binds a node to one project, independently verified local runtime instance and signed
CSR. Certificate renewal persists its identity and CSR before exchange, reconciles a
lost reply and atomically replaces the host fingerprint. Revocation is durable and
rechecked during active connections. Certificate authority and connector keys are
Ed25519 and remain native/operator-held.

Each connector authenticates the pinned local public API as a dedicated enrolled
application. Native Desktop bootstrap provisions separate primary, approval-broker and
connector grants, delivered and activated atomically. Cloud roles intersect immutable
local roles, scopes and tool ceilings. Runtime policy, audit, approvals, sandboxing,
effect recovery and output release remain runtime-owned. No private worker RPC,
arbitrary filesystem/network tunnel or generic renderer capability crosses this boundary.

Allocate each task to one node and one local run, with caller/project-scoped stable
idempotency keys and durable command receipts. Persist allocation, receipt and event
cursor changes atomically with per-resource audit and delivery-outbox records before
acknowledging them. Entity revisions and per-feed ordering allow independent resources
to progress without a cloud-wide journal-head lock.
Exact retries reconcile existing state; changed duplicates and cursor gaps fail.
Unknown outcomes stay explicit and retain their admission slot. They never cause
automatic execution on another node.

Group native-installation hosts, independently enrolled runtime agents and workspace
identities without changing authority. A durable thread maps to one runtime and native
session. Human turns allocate ordered immutable tasks/runs; cloud owns thread metadata
and the queue while the runtime owns execution and released content. Default discovery
contains only the connector application's own runs. Existing local sessions require
explicit source-application workspace sharing. Imported source runs remain read-only;
shared continuation creates recipient-owned runs beneath that recipient's native grant.
Disabling sharing stops future release and continuation while retained cloud history
remains available for authorized offline reads.

Store browser sessions and encrypted OIDC sign-in flows in PostgreSQL so callback and
browser requests may reach different replicas. All replicas share an independent
OIDC-flow envelope key. Bind each runtime connection to an expiring database lease and
monotonic fencing generation; validate that lease within runtime-output transactions.
A replaced replica cannot commit under its earlier generation. PostgreSQL notifications
wake consumers, while durable outbox/state reads and cursors establish recovery.

Exchange typed domain values through `CloudStore`; the PostgreSQL adapter owns their
mapping to relational columns and reconstruction. Normalize core account, project,
membership, inventory, conversation, enrollment and settings fields. Retain JSONB for
SDK payloads, released events and opaque authorization envelopes rather than whole
core domain records. Project-scoped foreign keys, constraints and column indexes
support bounded queries and atomic changes to related records.
Keep PostgreSQL row models private to the adapter and map fields explicitly in Rust.
Bind and decode native columns through Diesel; do not use whole-object JSON parameters
or SQL-built JSON projections for core entities. Canonical audit serialization and
genuine JSONB payload serialization remain independent of the relational row mapping.

Apply checksummed schema migrations transactionally under a schema-specific advisory
lock. Ordinary cloud writes do not acquire that migration lock. Retained entity/event
audit chains detect changed retained content, but do not protect against replacement,
deletion or rollback of the database and its hashes together. Independent audit-head
checkpoints, restricted database roles, encrypted backups and tested restore procedures
are operator requirements; the cloud deployment does not retain a journal-anchor PVC.

Output resumes after the highest contiguous acknowledged sequence. Bound frames,
queued messages, active tasks, retained events and bytes, and connection/request
timeouts. Reaching retention limits is explicit; the exact run's snapshots and controls
continue. HTTP mutations acknowledge durable queuing; the UI checks runtime receipts
before confirming an approval response or cancellation. Shutdown drains for a bounded
period. Disconnecting a connector does not cancel accepted work.

Channel startup has a ten-second deadline across TCP, TLS and HTTP/2, including
certificate renewal and revocation. Opening stream response headers and receiving its
first welcome have separate ten-second deadlines; an established stream remains live
under heartbeat and backpressure bounds. An authenticated peer withholding HTTP/2
progress must return the same connector to bounded retry without changing run identity.

Before connecting and every five-second heartbeat, the connector probes the local
runtime's authenticated caller-scoped read API with a three-second deadline and a
one-item page. It discards the page locally. An open SDK client alone does not establish
daemon readiness: unavailable or stalled local service closes the cloud stream and
returns to bounded reconnect, retaining the same enrollment and accepted work.

Desktop-managed sidecars stop with Desktop. Persistent work uses an independently
supervised daemon and connector under one explicitly chosen OS-user identity. Windows
uses protected discovery and native credentials; macOS uses Keychain; graphical Linux
uses Secret Service and native GTK credential entry. Headless Linux explicitly injects
a wrapping-key authority and stores only authenticated encrypted envelopes. Missing,
changed or unavailable authority fails closed.

## Consequences

The web frontend reuses React/TypeScript and shared shadcn-based Colossus UI. Kubernetes manifests and
portable container images avoid a cloud-provider dependency. Linux Desktop preview
packages bind the sealed executable manifest into the running ELF and verify every
bundled executable digest. They report unsigned distribution accurately and do not
advertise a stable automatic update channel.

The OpenID Connect dependency brings `rsa`, affected by
[RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html). The advisory
concerns private-key timing leakage. Colossus uses this dependency only to verify
provider signatures with public keys; it has no RSA private-key signing or decryption
path. Keep the exact documented supply-chain exception limited to that use and remove
it when the dependency no longer contains the affected version. Adding any RSA
private-key operation requires revisiting this decision and exception.

Sandbox provisioning, scheduling and cloud artifact/context transfer remain separate
extensions. Registered fleet size and replica count are not capacity evidence;
representative concurrent stream, history-query and failover measurements are required. See [cloud development](../cloud-control-plane.md)
and [operations](../../admin/cloud-control-plane.md) for current setup and acceptance.
