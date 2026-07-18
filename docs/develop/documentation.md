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

4. For tutorials and how-tos, include **Goal**, **Prerequisites**, **Steps**,
   **Expected result**, **Verification**, **Failure path**, and **Next step**.

5. Put installed-binary commands in user and operator pages. Keep Cargo, source
   launchers, and repository verification commands in Develop.

6. Link to the canonical owner instead of copying:

    - installation in Get started;
    - field names in Configuration reference;
    - access semantics in Administer;
    - tool definitions and schemas in Reference;
    - release history in the root changelog.

7. Use Mermaid only when relationships are clearer than prose. Add adjacent prose that
   explains the same sequence or structure without relying on color. The pinned,
   repository-local Mermaid runtime is a third-party build artifact; update its version,
   license, and documentation contract together. Wrap each diagram in a labeled,
   keyboard-focusable `diagram-scroll` region so dense diagrams remain readable on
   narrow screens. Do not replace the local runtime with a CDN import.

8. Add the page to explicit `zensical.toml` navigation and use lowercase directory
   routes. If replacing a historical URL, update the checked-in redirect manifest.

9. Build with the repository wrapper:

    ```bash
    ./scripts/docs-site build
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
console errors. Run the focused documentation contract before the repository completion
gates.

## Failure path

If a page needs two audiences or two documentation types, split it. Move reconstruction,
acceptance evidence, parity notes, and release-maintainer procedures to
`internal/documentation/`; that directory is excluded from site navigation, publication,
and search. Do not add template overrides, custom JavaScript, analytics, external fonts,
or CDN diagram loaders.

## Next step

Request review from the owner of the documented contract and from a reader in the
declared audience.
