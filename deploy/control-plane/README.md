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
  the web assets from the same source. It contains no user execution environment.

Render locally without contacting a cluster:

```sh
kubectl kustomize deploy/control-plane/kubernetes
kubectl kustomize deploy/control-plane/overlays/hosted-chat
kubectl kustomize deploy/control-plane/overlays/hosted-chat-sql
```

Pin the server/documentation image names and tags or digests in the base
`kustomization.yaml`. Both deployments require the configured PostgreSQL and TLS
Secrets; hosted Chat also needs external headless keys and the independent anchor authority. These templates are examples to adapt to your infrastructure.
