# Shared PostgreSQL hosted Chat

This overlay enables two rolling API/executor replicas with disposable local caches.
It uses the controller's PostgreSQL database for partitioned journals, projections,
settings, encrypted credential records, OAuth coordination and fenced runtime leases.
No shared filesystem or retained chat PVC is required.

Follow [shared hosted Chat setup](../../../../docs/admin/cloud-control-plane.md#scale-hosted-chat-across-replicas)
for the canonical key, OAuth, backup and recovery contracts. Replace the sample
project, provider/model, MCP and egress origins before enabling it. The provider key
is explicitly shared; MCP OAuth uses each user's own account. Connection settings
cannot exceed the deployment's reviewed origins.

Deploy `colossus-chat-anchor` independently with a separate database/role, backup
administration and storage-encryption key. Configure its HTTPS ingress in
`chat.shared.keys.anchor_endpoint`. The executor Secret `colossus-chat-state` must
provide `journal-key`, `signing-key`, `wrapping-key`, `provider-key` and `anchor-token`.
The anchor storage key and database credential belong only to the independent
authority. Existing controller database/authentication/TLS Secrets remain required.
Use an explicit CA bundle if the authority does not use the system trust roots.

The local `emptyDir` stores rebuildable lexical indexes and runtime working
structure. UID/GID `10001` and a `0700` private root preserve host confinement. The
four-runtime limit and 4 GiB process/cache bounds are examples; measure before
adjusting admission or replica budgets. SQL execution leases coordinate owners
across replicas. Accepted effects with an unknown remote outcome are reconciled,
not automatically repeated on takeover.

Render without deploying:

```sh
kubectl kustomize deploy/control-plane/overlays/hosted-chat-sql
```

The result should retain two replicas, RollingUpdate, the base disruption budget,
all original container security controls, the five chat Secret references, an
`emptyDir` cache, and no chat PVC. Keep the previous single-host volume and keys if
migrating existing data: enabling `shared` does not implicitly import it.
