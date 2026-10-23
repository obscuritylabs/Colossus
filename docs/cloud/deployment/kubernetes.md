---
title: Install with Helm
description: Install a release-matched Colossus Control Plane and verify your first sign-in.
audience: operator
type: how-to
icon: lucide/package
---

# Install with Helm

Start with a published chart release and its matching server and documentation
images. The chart includes its PostgreSQL dependency; installation needs no chart
dependency download.

## Prepare the cluster

Have these available before installation:

- Kubernetes 1.34+, Helm 3.19+, and three schedulable availability zones.
- A default or explicitly selected persistent StorageClass.
- A resource metrics API for CPU autoscaling.
- Separate browser and connector DNS names.
- An installed routing controller and a browser HTTPS certificate.
- A CNI that enforces NetworkPolicy.

The default ingress-nginx controller must enable `--enable-ssl-passthrough`.
For Istio, Gateway API, OpenShift, or fewer zones, apply the
[configuration choices](configuration.md) before installing.

## Set your hostnames and certificates

Create `deployment-values.yaml`:

```yaml
browserHost: colossus.example.com
connectorHost: runtimes.colossus.example.com
routing:
  mode: ingress-nginx
  browserTlsSecret: colossus-browser-tls
# Set this if your cluster has no suitable default StorageClass.
# postgres:
#   persistence:
#     storageClass: your-storage-class
```

Point both names at the routing controller. Create the namespace and browser
certificate Secret from protected certificate files:

```sh
kubectl create namespace colossus
kubectl -n colossus create secret tls colossus-browser-tls \
  --cert=/secure/cloud/browser.pem --key=/secure/cloud/browser-key.pem
```

The chart creates a separate connector CA and server identity. Connector TLS must
pass through the gateway intact; it does not use the browser certificate.

## Install the release

Replace `X.Y.Z` with a published chart version. Wait for its server, documentation,
and chart publishers to finish before using it.

```sh
helm upgrade --install colossus-control-plane \
  oci://ghcr.io/obscuritylabs/colossus-control-plane-chart \
  --version X.Y.Z --namespace colossus \
  -f deployment-values.yaml
kubectl -n colossus rollout status deployment/colossus-control-plane
kubectl -n colossus rollout status deployment/colossus-control-plane-docs
```

The bundled database uses plaintext transport to its exact same-namespace Service
hostname. This is an operator-selected network trust boundary. Use the
[external database option](configuration.md#use-an-external-database) for verified TLS.

## Sign in and check the installation

Read the initial password privately from Secret `colossus-control-plane-state`,
key `bootstrap-password`. Sign in at your browser hostname as `admin`.
There is no default password or public registration.

1. Create a project and grant your users [project access](../administration.md).
2. Open `/docs/` and try documentation search.
3. [Connect a runtime](../connect-runtimes.md), then start a short conversation.
4. [Back up identities and database state](operations.md) before accepting real work.

If pods stay pending, check zone eligibility and storage first. If sign-in or
enrollment fails, check DNS, browser HTTPS, and connector TLS passthrough.

## Replace a Kustomize installation

Follow the chart's
[migration instructions](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart#replace-a-kustomize-installation).
Retain the database, namespace, connector identities, auth key, and existing Secrets.
Deployment selectors differ, so plan a controlled cutover. Deleting the old
Kustomization wholesale can also delete its namespace.
