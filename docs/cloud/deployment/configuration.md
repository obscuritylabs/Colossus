---
title: Configure your installation
description: Choose Cloud routing, database TLS, identity providers, and platform settings through Helm values.
audience: operator
type: how-to
icon: lucide/settings
---

# Configure your installation

Keep your reviewed settings in `deployment-values.yaml` and pass it on every Helm
upgrade. Store credentials in existing Kubernetes Secrets; inline values remain in
Helm release history. The
[chart values and examples](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart)
are the complete reference.

## Choose routing

| `routing.mode` | Required platform setup |
| --- | --- |
| `ingress-nginx` | Browser TLS Secret; controller SSL passthrough enabled |
| `istio` | Existing Gateway with browser HTTPS termination and connector TLS passthrough |
| `gateway-api` | Existing Gateway with HTTPS and TLS/Passthrough listeners; TLSRoute-capable CRDs/controller |
| `none` | Operator-managed routes to the chart's Services |

Preserve browser `/api`, `/auth`, `/docs/`, direct page links, and SSE streams.
Disable response buffering and streaming request deadlines. Send connector traffic
to port 8443 without terminating its client-certificate TLS session.

Web terminals also require WebSocket upgrades on the browser route. Preserve idle
connections with heartbeat traffic and suitable load-balancer/controller timeouts.
Verify the chosen Gateway controller supports WebSocket forwarding; ordinary HTTP/3
support alone does not establish terminal support.

Terminal requests may arrive at a replica other than the Fleet connection owner.
The chart advertises each pod's internal address through the Downward API and routes
the authenticated relay over port 8443. Permit server-to-server traffic on that port
in deployment-specific NetworkPolicies. Do not terminate its mutual TLS in a mesh
proxy. The relay checks its own purpose-specific authentication in addition to TLS;
an enrolled Fleet certificate alone cannot call it. Sticky sessions are unnecessary.

For a multi-process deployment outside Kubernetes, set
`COLOSSUS_CONTROL_PLANE_RELAY_ADDRESS` to each process's reachable internal `IP:port`
for its existing gRPC listener. Local development derives a loopback address from the
listener. The expiring route directory contains only process metadata, not shell bytes.

Use `mesh.mode: sidecar` or `ambient` with the corresponding chart example. Ambient
mode disables sidecar injection; cluster mesh and network policy must permit its
traffic. Gateway/listener examples are platform manifests, not Helm values.

## Use an external database

Create a dedicated PostgreSQL database/schema and a role that can apply startup
migrations. Put its connection URL in an existing Secret, then set:

```yaml
postgres:
  enabled: false
database:
  external:
    existingSecret: colossus-database
    secretKey: database
    tls:
      mode: webpki_roots
```

For a private CA or RDS CA bundle, select `custom_ca`, set `existingSecret` inside
`database.external.tls`, and store PEM trust roots under its `caKey` (default
`ca.pem`). Both modes verify the database hostname and chain. External plaintext
connections are rejected.

The controller pool defaults to eight connections plus one notification listener
per pod: 27 at three replicas, 54 at six, or 63 during one-pod surge. Reserve
additional capacity for backups and, when enabled, hosted execution.

## Add OIDC sign-in

Use `examples/gitlab.yaml` or `examples/keycloak.yaml` as your starting values.
Set the issuer and client ID, reference the client Secret when required, and
register exactly `https://BROWSER_HOST/auth/callback` with the identity provider.

Local accounts remain enabled unless you set `auth.local.enabled: false`.
OIDC-only bootstrap also needs `auth.bootstrap.oidcSubject`: the first
administrator's verified `sub`, not their email. The shared flow key must match
across replicas. Provider HTTPS certificates must be trusted by the server;
database custom-CA settings do not change OIDC trust.

## Adapt to your platform

| Installation | Set or provide |
| --- | --- |
| OpenShift | `examples/openshift.yaml`; restricted SCC supplies arbitrary identities. No `anyuid` grant is required. |
| Fewer than three zones | Lower `topologySpread.minDomains` and replica minima deliberately; otherwise pods may stay pending. |
| More than three zones | Raise the replica minimum to at least the eligible zone count. |
| Disconnected registry | `examples/airgap.yaml`; mirror the packaged chart and all image layers, then set registry and pull-Secret values. |
| Existing identity Secrets | `auth.state.existingSecret`, `tls.existingSecret`, and optional OIDC/database Secret references. |

Do not run `helm dependency update` in a disconnected installation. The packaged
chart already vendors its dependency. Retain reviewed image digests, local DNS,
browser trust, storage, and network controls. Local accounts and bundled docs
need no external identity service or CDN.
