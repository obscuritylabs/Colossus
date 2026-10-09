---
title: Host public documentation
description: Serve the release-matched Colossus documentation and search beside an on-prem Control Plane.
audience: operator
type: how-to
---

# Host public documentation

The Control Plane Kubernetes template serves public documentation at
`https://colossus.example.com/docs/`. The documentation image contains the same
canonical Zensical pages, local search index, diagrams, and compatibility redirects
as its source release. It runs separately from the Control Plane and needs no
database, account, credentials, or persistent volume.

## Deploy with the Control Plane

Follow [Control Plane deployment](cloud-control-plane.md#deploy-on-kubernetes) and
select both images in `deploy/control-plane/kubernetes/kustomization.yaml`:

- `ghcr.io/obscuritylabs/colossus-control-plane:TAG`
- `ghcr.io/obscuritylabs/colossus-documentation:TAG`

Use the same reviewed stable or preview source tag for both, or pin their published
digests. Wait for the documentation candidate and publication workflows to succeed;
the source checkout alone does not establish that a registry image exists. On first
GHCR publication, verify public package visibility and an anonymous pull of its
published digest before advertising the image. Replace the example browser hostname
and supply its TLS Secret. The browser
Ingress routes `/docs` to the documentation Service and `/` to the Control Plane;
keep `/docs/` intact and do not add a path rewrite. The same browser certificate covers
both applications. A separate hostname can route its `/docs` prefix to the same
documentation Service.

```sh
kubectl kustomize deploy/control-plane/kubernetes
kubectl apply -k deploy/control-plane/kubernetes
kubectl -n colossus rollout status deployment/colossus-documentation
curl --fail https://colossus.example.com/docs/
curl --fail https://colossus.example.com/docs/admin/cloud-control-plane/
curl --fail https://colossus.example.com/docs/search.json
```

The documentation deployment has two replicas and a disruption budget. It runs as
UID/GID 10001, drops all capabilities, disables privilege escalation and service-account
tokens, uses a read-only root filesystem, and mounts only a bounded temporary directory.
Readiness reads the actual generated homepage; liveness checks the static server.
GET and HEAD are supported. Missing pages retain HTTP 404 responses.

## Browse and search

The Control Plane rail's **Documentation** link opens `/docs/` in a new tab. Sign-in
is not required for these published pages. The site's search runs in the reader's
browser and uses the local `/docs/search.json` index; fonts, scripts and diagrams
are bundled rather than loaded from a public CDN.

Agents with independently authorized HTTP access can read the same page URLs and
search index. Each index item contains a title, relative `location`, and searchable
page text. Resolve locations against the `/docs/` base, retaining any fragment.
For example, `admin/cloud-control-plane/#deploy-on-kubernetes` resolves under
`https://colossus.example.com/docs/`. This is public static documentation, not a new
runtime tool or a permission grant. Native runtime network policy still applies.

An installation that serves documentation elsewhere can select the rail destination
when building the web image with `VITE_COLOSSUS_DOCUMENTATION_URL`. HTTP(S) URLs and
same-origin root-relative paths are supported; invalid values fall back to `/docs/`.
See the [source build commands](../develop/documentation.md#build-the-on-prem-documentation-image).

## Verify and update

Open a direct page URL, search for a documented command, and follow a result. Verify
that navigation stays on your documentation origin beneath `/docs/`, and inspect
the browser for missing assets. Legacy routes such as `/docs/GETTING_STARTED.html`
redirect to the corresponding local page, preserving supported anchors.

To inspect an image before Kubernetes deployment:

```sh
docker run --rm --read-only --user 10001:10001 --cap-drop ALL \
  --security-opt no-new-privileges \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m,mode=1777 \
  --publish 127.0.0.1:8081:8080 \
  ghcr.io/obscuritylabs/colossus-documentation:TAG
```

Open `http://127.0.0.1:8081/docs/`. Update the documentation image together with its
Control Plane release; the static service can roll independently. If a page fails,
check the selected image, `/docs` Ingress backend and unmodified prefix first. Restore
the previous known image digest to roll back. Public documentation does not require
access to application authentication, runtime enrollments, or database backups.
