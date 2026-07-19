---
title: Contributing
description: Repository conventions for making focused, reviewable Colossus changes.
audience: developer
type: how-to
---

# Contributing

## Goal

Prepare a focused change that respects dependency boundaries and is straightforward to
review.

## Prerequisites

- A source checkout.
- The toolchain and platform requirements in [Source setup and test tiers](setup-testing.md).
- Familiarity with [Architecture overview](architecture.md) and
  [Security architecture](security-architecture.md) for boundary-sensitive work.

## Steps

1. Create a focused branch and keep unrelated worktree changes intact.

2. Install the local Conventional Commit hook:

    ```bash
    ./scripts/install-git-hooks.sh
    ```

3. Make the smallest coherent change. In particular:

    - keep `colossus-domain` dependency-free;
    - keep CLI and TUI as request/render interfaces;
    - follow the [Rust crate structure](crate-structure.md) contract and keep roots thin;
    - put policy, tool, model, workflow, and state behavior in their owning services;
    - add or update tests for every behavior change.

4. Iterate with the smallest relevant test tier, then run the completion gates described
   in [Source setup and test tiers](setup-testing.md).

5. Before merging, inspect every unresolved pull-request review thread and required
   check, including automated ChatGPT/Codex review. Address each actionable finding in
   code and tests; do not treat a green build as a substitute for review resolution.

6. Use a Conventional Commit message:

    ```text
    <type>[optional scope][!]: <description>
    ```

    Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
    `refactor`, `revert`, `security`, `style`, and `test`.

## Expected result

The diff has one clear purpose, tests describe the changed behavior, and no interface or
crate root has absorbed unrelated application logic.

## Verification

Run the repository completion gates and inspect the final diff:

```bash
git diff --check
./scripts/check_crate_roots.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Failure path

If a change crosses an unclear ownership boundary, stop and map the request, service,
port, and adapter before adding code. If a test exposes a security or state invariant,
repair the implementation rather than weakening the test. Preserve user-owned or
unrelated worktree changes.

## Next step

Open a review with the behavioral outcome, affected boundaries, focused tests, and full
gate results in the description. Recheck unresolved human and automated review threads
after the final push and before merge.
