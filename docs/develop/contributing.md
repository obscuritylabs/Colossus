---
title: Contributing
description: Repository conventions for making focused, reviewable Colossus changes.
audience: developer
type: how-to
---

# Contributing

Use [Source setup and test tiers](setup-testing.md) for toolchains, focused commands,
component gates and local launchers.

1. Create a focused branch and keep unrelated worktree changes intact.

2. Install the local Conventional Commit hook:

    ```bash
    ./scripts/install-git-hooks.sh
    ```

3. Make the smallest coherent change, following the [architecture](architecture.md),
   [security boundaries](security-architecture.md) and [crate structure](crate-structure.md):

    - keep `colossus-domain` dependency-free;
    - keep CLI and TUI as request/render interfaces;
    - keep crate roots thin;
    - put policy, tool, model, workflow, and state behavior in their owning services;
    - test supported behavior; remove a deleted feature's tests unless they protect an
      intentional rejection, migration or compatibility contract.

    Map unclear ownership before adding code. Repair security or state invariants
    exposed by tests rather than weakening the assertions.

4. Iterate with focused package checks and directly affected tests. Once the change
   is finished, inspect the final diff and run the PR gate before opening or updating
   the pull request:

    ```bash
    git diff --check
    cargo xtask pr --base origin/main
    ```

    This is the final local check, not the edit loop. It includes the full Rust gate
    when Rust is selected; do not also run `cargo xtask check rust` on unchanged source.

5. Use a Conventional Commit message:

    ```text
    <type>[optional scope][!]: <description>
    ```

    Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
    `refactor`, `revert`, `security`, `style`, and `test`.

6. Describe the behavioral outcome, affected boundaries and validation in the PR.
   Resolve actionable human and automated review findings and required checks. With
   the final head green, conflict-free and non-draft, request hosted acceptance once
   through the [`ci:full` procedure](ci-cd.md#request-pre-merge-acceptance).
