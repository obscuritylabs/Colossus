# Colossus Control Plane deployment

The [operator guide](../../docs/cloud/deployment/index.md) owns authentication,
TLS, PostgreSQL, runtime enrollment, backup and recovery procedures.

- `chart/` is the Kubernetes 1.34+ Helm deployment with three controller replicas,
  documentation and optional bundled PostgreSQL. Built-in hosted Chat is disabled by
  default; independently enrolled runtimes keep their own execution state.
- `chart/examples/hosted-chat.yaml` enables shared PostgreSQL hosted Chat with rolling
  replicas, fenced execution ownership and disposable caches. It requires a separately
  administered checkpoint-anchor service and externally managed keys. The chart replaces
  the former Kustomize base and hosted-chat overlays.
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
helm template colossus-control-plane deploy/control-plane/chart --namespace colossus
helm template colossus-control-plane deploy/control-plane/chart --namespace colossus \
  -f deploy/control-plane/chart/examples/hosted-chat.yaml
```

Pin the server/documentation image tags or digests through chart values. Hosted Chat
also needs external headless keys and the independent anchor authority. Review the
[chart guide](chart/README.md) for Secret contracts and migration from Kustomize.
