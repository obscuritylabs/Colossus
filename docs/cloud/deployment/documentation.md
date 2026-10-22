---
title: Host documentation
description: Serve and verify the release-matched public documentation beside your Cloud installation.
audience: operator
type: how-to
icon: lucide/book-open
---

# Host documentation

The Helm chart serves documentation at your browser origin's `/docs/` path by
default. The separate static service needs no database, sign-in, or persistent
volume. Search, fonts, scripts, and diagrams work without a public CDN.

## Choose the matching release

Use the same published stable or preview tag for both images, or their reviewed
publication digests:

| Helm setting | Image |
| --- | --- |
| `image` | `ghcr.io/obscuritylabs/colossus-control-plane:TAG` |
| `documentation.image` | `ghcr.io/obscuritylabs/colossus-documentation:TAG` |

The chart selects release-matched tags automatically. Wait for both image
publications to succeed. The browser route must forward `/docs` to the docs
Service without rewriting the prefix; the same browser certificate covers it.

## Verify navigation and search

```sh
kubectl -n colossus rollout status deployment/colossus-control-plane-docs
curl --fail https://colossus.example.com/docs/
curl --fail https://colossus.example.com/docs/cloud/deployment/
curl --fail https://colossus.example.com/docs/search.json
```

Open **Documentation** in the Cloud rail, visit a direct page link, search for a
command, and follow a result. Navigation should stay beneath `/docs/` on your
documentation origin. Check the browser for missing assets.

Agents with authorized HTTP access can read the same pages and `search.json`.
Resolve each search item's relative `location` against the `/docs/` base, retaining
its fragment. This public content adds no runtime network permission.

## Preview an image locally

```sh
docker run --rm --read-only --user 10001:10001 --cap-drop ALL \
  --security-opt no-new-privileges \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m,mode=1777 \
  --publish 127.0.0.1:8081:8080 \
  ghcr.io/obscuritylabs/colossus-documentation:TAG
```

Open `http://127.0.0.1:8081/docs/`. The service supports GET/HEAD and keeps missing
pages as HTTP 404 responses. Stop the preview with Ctrl+C.

## Update or use a separate origin

Update docs with the Control Plane release. If pages fail after an update, check
the image digest, `/docs` backend, and unmodified prefix first. Restore the previous
known docs digest to roll back the static service.

For a separate documentation origin, select `VITE_COLOSSUS_DOCUMENTATION_URL`
when building the browser app. See the
[source build instructions](../../develop/documentation.md#build-the-on-prem-documentation-image).
