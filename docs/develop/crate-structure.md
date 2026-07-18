---
title: Rust crate structure
description: Keep Rust crate roots as readable API and composition maps with responsibility-focused modules.
audience: developer
type: concept
---

# Rust crate structure

Colossus crate roots are maps, not implementation files. A reader should be able to open
`lib.rs` or `main.rs`, understand the crate's responsibilities and public surface, and
then choose one clearly named module for the behavior they need.

## Root contract

A crate root may contain:

- crate-level documentation and narrowly scoped lints;
- module declarations;
- public re-exports that define the supported crate API;
- dependency imports shared by a tightly coupled private implementation;
- small constants or composition code that genuinely describes the whole crate;
- the binary entrypoint wrapper required by Rust.

A crate root should not accumulate configuration parsing, protocol metadata, repository
folding, service methods, adapter implementations, command dispatch, rendering, or unit
tests. Put those in modules named for their responsibility.

The repository enforces a 250-line ceiling for tracked Rust crate roots with:

```bash
./scripts/check_crate_roots.sh
```

The ceiling is a backstop, not a target. Most roots should be much smaller. Do not raise
it to accommodate new behavior; extract a module.

## Choosing modules

Split along concepts that a maintainer can search for, not at arbitrary line counts.
Common module roles include:

| Responsibility | Typical module names |
| --- | --- |
| Public data and protocol shapes | `contract`, `types`, `operations`, `frames` |
| Strict configuration | `config`, `validation`, `profiles` |
| Application behavior | `service`, `execution`, `schedules`, `subscriptions` |
| Ports and persistence | `repository`, `journal`, `projection` |
| External effects | `provider`, `gateway`, `http`, `process`, `platform` |
| Interface behavior | `commands`, `dispatch`, `render`, `state`, `terminal` |

One large cohesive enum or match can remain together when splitting it would make the
contract harder to follow. A module should be split again when it owns more than one
reason to change, not merely because it crosses an arbitrary size.

## Visibility and dependencies

- Re-export only the intended public API from the crate root.
- Use `pub(super)` or narrower visibility for collaboration between private sibling
  modules; moving code must not accidentally expand the external API.
- Keep cross-module calls directional. Shared helpers belong in a small common module,
  not in whichever implementation happened to need them first.
- Prefer explicit imports for independent modules. A private, tightly coupled module may
  use a parent composition prelude when the root deliberately owns that dependency set.
- Keep `colossus-domain` dependency-free and preserve the dependency direction in
  [Architecture overview](architecture.md).

## Tests

Keep production roots free of inline test implementations. A small crate can use
`src/tests.rs`; as a suite grows, use `src/tests/` modules named for behavior or adapter
contracts. Integration and platform acceptance tests remain under the crate's `tests/`
directory.

Moving tests is structural work: preserve fixture bytes exactly, especially YAML, JSON,
signatures, hashes, and whitespace-sensitive protocol examples.

## Change checklist

When adding or changing behavior:

1. Identify the owning crate and responsibility-focused module.
2. Keep interfaces such as CLI and TUI free of runtime, policy, and state logic.
3. Keep the crate root limited to API and composition.
4. Preserve public paths with deliberate re-exports.
5. Add or update focused tests in the owning module or test suite.
6. Run the crate-root check, focused tests, and the repository completion gates from
   [Contributing](contributing.md).
