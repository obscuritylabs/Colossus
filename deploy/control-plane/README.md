# Colossus Control Plane deployment

The [operator guide](../../docs/admin/cloud-control-plane.md) owns authentication,
TLS, PostgreSQL, runtime enrollment, backup and recovery procedures.

- `kubernetes/` is the default two-replica, stateless controller deployment. Built-in
  hosted Chat is disabled; independently enrolled runtimes keep their own execution
  state.
- `overlays/hosted-chat-sql/` enables shared PostgreSQL hosted Chat with two rolling
  replicas, fenced execution ownership and disposable caches. It requires a separately
  administered checkpoint-anchor service and externally managed keys.
- `overlays/hosted-chat/` is a compatibility alias for the shared SQL overlay.
- `Dockerfile` builds the controller, its optional controlled-chat composition and
  the web assets from the same source. The image also contains `colossus-chat-anchor`
  for the separately administered anchor service. It contains no user execution environment.

Annotated stable and preview release tags build and test both Linux amd64 and arm64
images. Publishing the reviewed GitHub Release publishes the tested images:

| Deployment | Release image |
| --- | --- |
| Control Plane and Web | `ghcr.io/obscuritylabs/colossus-control-plane:TAG` |
| Independent Hosted anchor authority | The same Control Plane image, with `command: [colossus-chat-anchor]` and its own configuration and Secrets |
| Documentation | `ghcr.io/obscuritylabs/colossus-documentation:TAG` |

Use the exact release tag or recorded publication digest; there is no `latest` alias.
The anchor uses independent database credentials and encryption keys, as described in
the operator guide. Its executable is also included in the offline server bundle.
See [release image operations](../../docs/develop/releasing.md#control-plane-containers-and-vs-code-packages)
and [documentation image operations](../../docs/develop/releasing.md#documentation-container)
for publication verification and recovery.

Render locally without contacting a cluster:

```sh
kubectl kustomize deploy/control-plane/kubernetes
kubectl kustomize deploy/control-plane/overlays/hosted-chat
kubectl kustomize deploy/control-plane/overlays/hosted-chat-sql
```

Pin the server/documentation image names and tags or digests in the base
`kustomization.yaml`. Both deployments require the configured PostgreSQL and TLS
Secrets; hosted Chat also needs external headless keys and the independent anchor authority. These templates are examples to adapt to your infrastructure.
