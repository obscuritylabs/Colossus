# Colossus Control Plane Helm chart

This chart replaces both former Kustomize directories. It targets Kubernetes 1.34+
on GKE, EKS and OpenShift, including disconnected registries. Templates and production
configuration are checked without installing a cluster. These checks do not establish
live controller, storage, mesh, or platform acceptance.

For a complete Windows Docker Desktop installation, use the
[local Helm harness](../../../docs/develop/local-helm.md). Its local profile provisions
Hosted execution, verified-TLS PostgreSQL, and a separate HTTPS checkpoint authority
alongside Fleet and documentation. The production chart retains the explicit Hosted
configuration described below.

## Install

Provision a namespace, three schedulable availability zones, a StorageClass, metrics-server
(or another resource metrics API), a DNS name for browsers and a different name for
connectors. Install the routing controller and its required CRDs separately. Supply a
browser HTTPS certificate through that controller. The chart creates the connector CA
and leaf independently; browser HTTPS never terminates connector TLS.

Create `deployment-values.yaml` with your routing, storage and Secret references; the
values examples below provide starting points.

```sh
helm upgrade --install colossus-control-plane \
  oci://ghcr.io/obscuritylabs/colossus-control-plane-chart \
  --version X.Y.Z --namespace colossus --create-namespace \
  --set browserHost=colossus.example.com \
  --set connectorHost=runtimes.colossus.example.com \
  -f deployment-values.yaml
```

For a source checkout, use `deploy/control-plane/chart` in place of the OCI reference.
The PostgreSQL chart and its common library are already vendored; neither installation
nor packaging needs a dependency download. Chart `version` and `appVersion` equal the
workspace version. Default server and documentation images use the matching `vVERSION`
tag. Wait for all release image/chart publishers to succeed before using a new version.
Override `image.digest` and `documentation.image.digest` to pin reviewed executable
manifests; a digest takes precedence over a tag.

Local authentication is enabled by default. Read the generated initial password privately:

```sh
kubectl -n colossus get secret colossus-control-plane-state \
  -o jsonpath='{.data.bootstrap-password}' | base64 --decode
```

The initial account is `admin`. Bootstrap only runs while no persisted administrator
exists; changing a bootstrap password does not reset an account. Use Administration
for subsequent accounts and password resets. Avoid writing secret values into Git or
terminal logs. Inline secrets supplied through values are retained in Helm release history.

## Replace a Kustomize installation

Retain the existing database, namespace, connector CA/server identity and shared auth
key. Set `postgres.enabled=false` and reference the old Secrets through
`database.external.existingSecret`, `auth.state.existingSecret`, `tls.existingSecret`
and, if used, `auth.oidc.existingSecret`. The former state Secret's database URL key
is `database`. Preserve the existing browser/connector hostnames, OIDC issuer/client
and login configuration; disable bootstrap after an administrator has been provisioned.
This cutover does not import an external database into the bundled PostgreSQL volume.

Pause the previous deployment reconciler and retire its superseded workloads, Services,
Ingresses and disruption budgets before installing the overlapping Helm resources.
Deployment selectors differ, so the existing Deployment cannot be updated in place.
Keep the namespace, retained Secrets and database storage. Do not delete the former
Kustomization wholesale: it includes the namespace. Take backups and schedule a controlled
cutover; no installation or upgrade acceptance is established by the static chart gate.

The former hosted-chat overlays use `examples/hosted-chat.yaml` instead. Preserve their
external key Secret, anchor authority, project IDs, provider/MCP origins and reviewed
runtime configurations through `chat` values. Shared execution remains SQL-backed;
local caches are disposable. Retain any legacy single-host state and matching keys
until a separately supervised data migration is verified.

## Routing and meshes

The chart creates routes against an installed controller; it does not deploy ingress
controllers, gateways, CRDs, namespace labels, or mesh-wide policy. See the prerequisite
listener manifests in `examples/gateway-listeners.yaml`; they are Kubernetes manifests,
not Helm values. Replace their hostnames and choose one implementation.

| `routing.mode` | Rendered resources | Controller prerequisite |
| --- | --- | --- |
| `ingress-nginx` (default) | Separate browser/connector Ingresses | Controller flag `--enable-ssl-passthrough`; browser TLS Secret in release namespace |
| `istio` | Browser HTTP and connector TLS VirtualServices | Referenced Istio Gateway: browser HTTPS/SIMPLE, connector TLS/PASSTHROUGH on 443 |
| `gateway-api` | HTTPRoute and TLSRoute | Gateway with HTTPS termination and TLS/Passthrough listeners; TLSRoute-capable CRDs/controller |
| `none` | Services only | Operator-owned routing with the same transport contract |

Browser routing retains `/docs/`, `/auth/`, `/api/`, direct SPA links and SSE. No prefix
rewrite or SSE buffering is allowed. Istio and Gateway API disable request deadlines on
the streaming browser route; configure any controller-level stream idle timeout as needed.
`routing.publicPort` defaults to 443. Set it to an external TLS port such as 8443 when
both hostnames use that port. This changes the advertised browser origin and connector
endpoint only; configure your gateway or host port mapping separately. Hostname values
remain bare DNS names, without schemes or ports. Register OIDC callback URLs with the
same external port when using a non-default listener.
Do not terminate connector TLS at a gateway or replace TLSRoute with GRPCRoute. Port 8443
must receive the original client certificate and TLS session. `routing.gatewayAPI` names
an existing Gateway and its two listener sections. Cross-namespace gateways must allow
routes from the release namespace. Istio browser certificate Secrets belong beside the
actual gateway pods, which may differ from the VirtualService namespace.

Use `examples/istio.yaml` for injected sidecars or `examples/ambient.yaml` for ambient
workloads. Ambient labels opt pods into ztunnel and explicitly disable sidecars; do not
combine them with namespace-wide sidecar injection. Mesh policies, ingress gateway
membership and CNI policy must permit browser, connector and database traffic. Preserve
application mutual TLS inside the mesh transport; do not add TLS origination to the
opaque connector stream. The chart does not create a waypoint or alter PeerAuthentication. In ambient mode the
database policy also admits HBONE port 15008 from server pods; waypoint deployments
need explicit additional peers. See [Istio network-policy guidance](https://istio.io/latest/docs/ambient/usage/networkpolicy/).

`routing.gatewayAPI.tlsRouteApiVersion` defaults to `gateway.networking.k8s.io/v1`;
select `gateway.networking.k8s.io/v1alpha2` for older installed CRDs. Kubernetes version
alone does not install Gateway API. See the upstream [Gateway API TLS contract](https://gateway-api.sigs.k8s.io/guides/tls/)
and [Istio passthrough guide](https://istio.io/latest/docs/tasks/traffic-management/ingress/ingress-sni-passthrough/).

## PostgreSQL and capacity

The default is one persistent PostgreSQL instance with a 20 GiB PVC. Select a compatible
StorageClass through `postgres.persistence.storageClass`. The chart creates a dedicated
`colossus` database and non-superuser owner through the dependency's initialization
contract. The application never receives the PostgreSQL administrator password.
A database ingress NetworkPolicy allows the server pods in this namespace; the cluster
CNI must enforce NetworkPolicy. Add explicit backup/administration peers through
`database.networkPolicy.extraIngress`.

Bundled PostgreSQL uses the explicit `in_cluster` transport mode, bound to its exact
`SERVICE.NAMESPACE.svc` host. This operator-selected trust boundary permits plaintext
only for that configured Service name; it does not attest DNS, CNI, node, or network
isolation. Kubernetes namespace placement alone is not encryption. The bundled Service
must remain ClusterIP and in the release namespace. It is a single database instance,
so a database failure affects the whole control plane despite three server replicas.

For RDS or an existing PostgreSQL deployment, set `postgres.enabled=false` and use
`examples/external-database.yaml`. Store its URL under `database.external.secretKey`
in the referenced Secret, or let the chart create it from `database.external.url`.
The role must own its dedicated database/schema and apply startup migrations. External
TLS is always verified, even if the URL says `sslmode=disable`. Use `webpki_roots` for
public roots, or `custom_ca` and a PEM Secret for a private CA, including RDS CA bundles.
An existing database in this namespace still uses external verified TLS; the plaintext
exception is for the bundled dependency.

Default CPU autoscaling spans 3–6 server pods with a 70% target and five-minute scale-down
stabilization. Resources are initial budgets for approximately ten users/agents, not a
measured fleet capacity guarantee. Each server uses up to eight pool connections plus
one notification listener: 27 at three replicas, 54 at six, or 63 during a one-pod surge.
The default PostgreSQL connection ceiling is 100; reserve the remainder for administration,
backups and initialization. Recalculate `maxReplicas * (maxConnections + 1)` plus surge
before increasing either limit.

Server and documentation pods use a hard zone-spread constraint (skew 1, minimum three
domains). Three replicas distribute across three eligible zones. Set the minimum replica
count at least to your number of zones if there are more than three. Without three
eligible zones, some pods stay pending; consciously override `topologySpread.minDomains`
and replica minima for a smaller cluster. Storage volume placement/reattachment is owned
by the StorageClass. A disruption budget permits one unavailable pod per application.

Migrations run at application startup under a schema-specific advisory lock. There is
no migration hook or job. Migration checksums fail closed; rollback of application
objects does not reverse a database migration. Follow the operator backup/restore guide
before upgrades. Database history and operational state live in PostgreSQL; no server
PVC is needed.

## Secret lifecycle

| Purpose | Existing-Secret setting | Required default keys |
| --- | --- | --- |
| Shared auth and initial local admin | `auth.state.existingSecret` | `auth-key` (64 hex characters); `bootstrap-password` (15+ characters, while local bootstrap enabled) |
| Connector CA and server identity | `tls.existingSecret` | `ca.pem`, `ca-key.pem`, `server.pem`, `server-key.pem` |
| Optional OIDC client secret | `auth.oidc.existingSecret` | `client-secret` |
| External database URL | `database.external.existingSecret` | `database` |
| External database private CA | `database.external.tls.existingSecret` | `ca.pem` |
| Bundled database application identity | `global.databaseSecretName` | `CUSTOM_USER`, `CUSTOM_DB`, `CUSTOM_PASSWORD`, `uri` |
| Bundled database administrator | `global.postgresAdminSecretName` | `postgres-password` |

All references are to Secrets in the release namespace. Individual keys can be selected
through the corresponding values, except for the bundled database's fixed key contract.
The bundled `uri` must use the same username/password/database as the `CUSTOM_*` values
and the exact Service hostname; percent-encode URL credentials. Default generated
credentials contain only alphanumeric characters. Changing an initialized PostgreSQL
Secret does not run SQL password rotation.

Helm creates and reuses the shared auth key, bootstrap password, database credentials,
and connector CA using live `lookup`. These durable Secrets carry
`helm.sh/resource-policy: keep`; they survive uninstall and must be backed up and
explicitly removed only when retiring the deployment. PostgreSQL PVCs also survive
ordinary StatefulSet deletion. Reinstall using the same release name/namespace to reuse
identities. Repeated offline/GitOps renders cannot discover retained Secrets: provide
stable existing Secrets for those workflows. Helm rollback does not undo account edits
or database credentials. An incomplete retained identity Secret fails rendering instead
of silently minting a replacement CA or database password. Exporting a rendered chart includes private key material.

The generated Ed25519 connector CA lasts ten years; the server leaf lasts one year by
default. Change `tls.renewalRevision` to reissue the leaf under the retained CA and roll
server pods. Existing/supplied certificate lifecycles belong to the operator. Renew
before expiry and roll pods after any externally managed certificate or Secret change;
processes load identities at startup. Configuration changes roll automatically. Supplied
PEM values may create the TLS Secret, but the entire CA/key/server/key set is required.
An external signer integration is not exposed by the current server: the configured
CA private key must remain available for enrollment signing. Supply that key as
unencrypted PKCS#8 PEM (`BEGIN PRIVATE KEY`), which the server CA loader accepts.

## OIDC

Local accounts remain enabled when OIDC is added. Use `examples/keycloak.yaml` or
`examples/gitlab.yaml`; replace issuer/client/subject values and provide the client
Secret if the provider requires one. Register the exact
`https://BROWSER_HOST/auth/callback` URI. Subject is the verified OIDC `sub`, not an email
address. To make OIDC the only login, set `auth.local.enabled=false` and set
`auth.bootstrap.oidcSubject` for the first administrator. The flow key must be shared
across replicas. Changing issuer/client requires browser sign-in again. Providers need
HTTPS certificates trusted by the server's existing OIDC client; database custom-CA
configuration does not change OIDC trust.

## Shared hosted chat

Hosted chat is disabled by default. `examples/hosted-chat.yaml` adds the shared SQL
composition with an external verified-TLS database (`postgres.enabled=false`). Runtime
journals do not accept the bundled database's plaintext exception. Review its project, model,
provider/MCP configuration and immutable network origins before enabling it. The
example increases server resources and limits resident runtimes to four per pod;
measure CPU, memory and database usage before increasing admission or autoscaling.
Hosted journals and credentials add database connections beyond the controller pool
budget above. Autoscaling an executor down may interrupt active work; configure scaling
and disruption policies for your workload and reconcile uncertain effects.

Set `chat.existingSecret` to an operator-managed Secret containing independent
`journal-key`, `signing-key`, `wrapping-key` and `anchor-token` keys. Encode each master
key as 64 hexadecimal characters (32 random bytes), without a trailing newline.
The sample shared provider uses `provider-key`; change `chat.providerKey` to select
another key or set it empty when all credentials are personal. The chart never generates these external
master keys. All API/executor replicas must use the same retained keys and projects.

Deploy the checkpoint-anchor authority independently with its own database, keys,
administration and backups; set `chat.anchor.endpoint` to its HTTPS `/v1/anchors` URL.
For a private CA, reference `chat.anchor.existingCaSecret` and optionally `caKey`.
Loopback HTTP and the single-host development composition are unavailable in this chart.
See the [operator guide](../../../docs/cloud/deployment/hosted-chat.md)
for anchor deployment, OAuth, key custody and recovery.

`chat.projects` accepts native `ChatProjectConfig` snapshots; the server validates their
runtime configuration and authority at startup. `chat.maxActiveRuntimes` accepts 1–64
and `chat.idleSeconds` accepts 1–86400 seconds. A restricted init container creates the
owner-private cache root with mode `0700`; `chat.cacheSizeLimit` bounds its `emptyDir`
(4 GiB by default). No chat PVC is used. The server image and security context also
apply to the init container, including arbitrary OpenShift identities.

## OpenShift and disconnected installations

Use `examples/openshift.yaml` for offline renders. Live OpenShift API discovery also
removes fixed UID/GID/fsGroup values from the server, docs, and PostgreSQL pod contexts.
The restricted SCC supplies arbitrary identities and volume groups; no `anyuid` grant,
root init container or service-account token is required. Use an installed supported
routing option; the chart does not create OpenShift Routes. GKE/EKS use the standard
security contexts; cluster-specific StorageClasses, controllers and registry credentials
remain operator choices.

Transfer the packaged chart and all image manifests/layers into the disconnected
registry. The PostgreSQL archive includes its common-library dependency and upstream
Apache-2.0 licenses. `Chart.lock` pins chart versions and `DEPENDENCIES.sha256` pins the
vendored bytes. Updating dependencies is a reviewed connected build step; never run
`helm dependency update` during disconnected installation.

Use `examples/airgap.yaml`. `global.imageRegistry` overrides registry hosts for all
three images; preserve their repository paths, or set each image repository/tag/digest
individually (`image`, `documentation.image`, `postgres.image`). Server/documentation
digests use `image.digest` and `documentation.image.digest`; PostgreSQL embeds its
digest in `postgres.image.tag` as `TAG@sha256:...`. The PostgreSQL image
is digest-pinned by its dependency; mirrors must retain that digest or supply an explicit
replacement. `global.imagePullSecrets` applies across all workloads. Mirror server and
documentation images from the same source release. Local login and bundled documentation
need no external identity service, fonts, CDN assets, or model provider. Provision DNS,
HTTPS trust and a functioning internal storage/network platform locally.

## Validate and package

With Helm 3.19.0 and repository build tools available:

```sh
cargo xtask check helm
helm package deploy/control-plane/chart --destination .local/helm
```

The gate runs strict lint, a render matrix, negative cases, real Rust configuration
validation, certificate parsing and offline package rendering. It does not install or
upgrade a release. Helm's generated passwords/keys are synthetic test data and are not
printed by the render tests. See `docs/develop/releasing.md` for GHCR publication.
