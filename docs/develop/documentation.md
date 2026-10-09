---
title: Documentation authoring
description: Audience, page-type, metadata, linking, diagrams, and build rules for the Colossus documentation site.
audience: developer
type: how-to
---

# Documentation authoring

## Goal

Add or revise a page with one reader, one outcome, one canonical fact owner, and a clean
Zensical route.

## Prerequisites

- A source checkout with Docker available for the pinned documentation toolchain.
- The target audience and page type selected before writing.
- The owning implementation or reference contract available for verification.

## Steps

1. Choose one audience:

   - `user` for completing work with Colossus;
   - `operator` for configuring, securing, and recovering deployments;
   - `developer` for schemas, internals, and contributing.

2. Choose one page type:

   - `tutorial` for a guided learning journey;
   - `how-to` for a specific outcome;
   - `concept` for a mental model;
   - `reference` for exact contracts.

3. Add required frontmatter:

   ```yaml
   ---
   title: Short page title
   description: One sentence describing the reader outcome.
   audience: user
   type: how-to
   ---
   ```

4. Structure tutorials and how-tos around the reader's task. Put the first useful
   action near the top, and use headings that name the work or decision. Include
   prerequisites, expected outcomes, verification, recovery advice, and next steps
   where they help the reader; do not require those as fixed headings. Avoid repeating
   the introduction in a separate **Goal** section or adding boilerplate to simple
   procedures.

5. Put installed-binary commands in user and operator pages. Keep Cargo, source
   launchers, and repository verification commands in Develop.

6. Link to the canonical owner instead of copying:

   - installation in Get started;
   - field names in Configuration reference;
   - access semantics in Administer;
   - tool definitions and schemas in Reference;
   - release history in the root changelog.

7. Use Mermaid only when relationships are clearer than prose. Add adjacent prose that
   explains the same sequence or structure without relying on color. Zensical's native
   renderer consumes the pinned, repository-local Mermaid runtime instead of its network
   fallback; update the local runtime, license, and documentation contract together.
   Wrap each diagram in a labeled, keyboard-focusable `diagram-scroll` region so dense
   diagrams remain readable on narrow screens. Do not replace the local preload with a
   CDN import.

8. Add the page to explicit `zensical.toml` navigation and use lowercase directory
   routes. If replacing a historical URL, update the checked-in redirect manifest.

9. Validate the site and its executable examples with the repository gate:

   ```bash
   cargo xtask check docs
   ```

   Preview locally with:

   ```bash
   ./scripts/docs-site serve
   ```

## Expected result

The page is discoverable in the intended audience lane, has valid metadata, builds in
strict mode, has no broken internal links or anchors, and contains no duplicated
canonical contract.

## Verification

Check the page at mobile and desktop widths in both color schemes. Verify keyboard focus,
overflow, tables, code copy, search discovery, diagrams, missing assets, and browser
console errors. The documentation gate builds the site in strict mode and runs published
configuration and workflow examples through the Rust parsers.

## Failure path

If a page needs two audiences or two documentation types, split it. Record durable
architecture choices as an [ADR](adr/index.md), keep current evidence discoverable in the
[feature inventory](feature-inventory.md), and leave superseded reconstruction narratives
in Git history. Do not maintain a detached internal specification. Do not add template
overrides, custom JavaScript, analytics, external fonts, or CDN diagram loaders.

## Next step

Request review from the owner of the documented contract and from a reader in the
declared audience.

## Build the on-prem documentation image

The public documentation container uses the canonical source and pinned Zensical
generator. Its build selects `/docs/` URLs in a copied configuration and passes that
mount root to the shared compatibility redirect generator. It does not modify
`zensical.toml` or the GitHub Pages publication. The digest-pinned Nginx runtime serves
static files as a non-root user. No generator, application backend or database runs
inside the final image.

The portable copy uses full-page navigation and local search. Zensical 0.0.50's
instant-navigation sitemap reader requires absolute URLs, so only
`navigation.instant` and `navigation.instant.progress` are disabled in that copy.
The canonical GitHub Pages configuration retains those features.

From the repository root:

```sh
docker build -f deploy/documentation/Dockerfile \
  --build-arg COLOSSUS_VERSION=development \
  --build-arg COLOSSUS_REVISION="$(git rev-parse HEAD)" \
  --build-arg SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
  -t colossus-documentation:local .
node deploy/documentation/smoke.mjs colossus-documentation:local
kubectl kustomize deploy/control-plane/kubernetes
```

The smoke runs the actual image with its non-root identity, read-only root filesystem,
dropped capabilities, a bounded temporary mount and no network. It verifies HTTP from
inside that isolated container: pages, local assets, search-index routes, compatibility
redirects, HEAD, missing resources and unsupported methods. It removes its own
container when finished. Release builds set exact tag/revision labels and the source
commit timestamp; timestamp normalization keeps generated input and output metadata
consistent across candidate builds.

The Control Plane defaults to this same-origin `/docs/` destination. To link a separate
documentation deployment, build the web image with an explicit destination:

```sh
docker build -f deploy/control-plane/Dockerfile \
  --build-arg VITE_COLOSSUS_DOCUMENTATION_URL=https://docs.example.com/docs/ \
  -t colossus-control-plane:local .
```

For a renderer-only build:

```sh
VITE_COLOSSUS_DOCUMENTATION_URL=https://docs.example.com/docs/ \
  npm run build --prefix apps/web
```

The destination is compile-time presentation configuration, not a new browser or
runtime permission. See [operator deployment](../admin/documentation-hosting.md) for
TLS, Ingress and release-image selection.

For the local Control Plane web development server, run the documentation image
on loopback port 8081 in a separate terminal:

```sh
docker run --rm --read-only --user 10001:10001 --cap-drop ALL \
  --security-opt no-new-privileges \
  --tmpfs /tmp:rw,noexec,nosuid,size=16m,mode=1777 \
  --publish 127.0.0.1:8081:8080 colossus-documentation:local
```

Vite forwards `/docs` to that listener without rewriting the path. The default
Documentation link therefore works through the same web development origin on
port 5180. Stop this container with Ctrl+C in its terminal when finished. The
static documentation listener needs no Control Plane session, API or credentials.
