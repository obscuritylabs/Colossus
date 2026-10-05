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

## Decision

Use a separate cloud application and a reusable native connector. `colossus-cloud`
owns durable project/node/task operations; `colossus-cloud-server` composes OIDC,
HTTP/JSON, SSE, certificate issuance and an independent redb/PostgreSQL journal.
`colossus-cloud-protocol` owns a separate major-versioned Protobuf bidirectional stream
with closed typed SDK operations. `colossus-connector` implements enrollment and the
outbound stream for CLI, standalone and native Desktop hosts.

Browser authentication uses OIDC authorization code with PKCE, verified issuer,
audience, signature, nonce, expiry and access-token hash. Explicit subject/project
memberships grant independent read, execution, control, approval and administration
permissions. Opaque browser sessions use protected cookies and same-origin CSRF checks.
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
cursor changes atomically, and checkpoint signed storage before acknowledging them.
Exact retries reconcile existing state; changed duplicates and cursor gaps fail.
Unknown outcomes stay explicit and retain their admission slot. They never cause
automatic execution on another node.

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

Desktop-managed sidecars stop with Desktop. Persistent work uses an independently
supervised daemon and connector under one explicitly chosen OS-user identity. Windows
uses protected discovery and native credentials; macOS uses Keychain; graphical Linux
uses Secret Service and native GTK credential entry. Headless Linux explicitly injects
a wrapping-key authority and stores only authenticated encrypted envelopes. Missing,
changed or unavailable authority fails closed.

## Consequences

The web frontend reuses React/TypeScript and Colossus UI. Kubernetes manifests and
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

Sandbox provisioning, scheduling, cloud artifact transfer and cross-run context
mapping remain separate extensions. See [cloud development](../cloud-control-plane.md)
and [operations](../../admin/cloud-control-plane.md) for current setup and acceptance.
