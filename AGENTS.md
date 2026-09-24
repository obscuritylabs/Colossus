# Colossus Agent Guide

Guidance for AI coding agents working in this repository. `CLAUDE.md` is a symlink to
this file, so Claude Code, Codex, and any other agent read the same guide.

## What this repository is

Colossus is an alpha-stage Rust runtime for running AI agents under explicit authority:
models, tools, policy, approvals, sandboxing, durable state, and audit in one system that
works online or fully offline. It ships as one binary exposing a CLI and a terminal UI; a
Tauri desktop app (`apps/desktop`) and the authenticated SDKs (`sdk/`) drive the same
runtime over a narrow authenticated boundary.

Rust is the active root implementation. Python 0.5 is retained only on the
`python-v0.5.0` and `python-legacy` branches; do not reintroduce its package or state.
Configuration and canonical state use the Rust YAML and redb formats; never silently
import the legacy Python state.

## Toolchain

Rust 1.96.0, edition 2024, resolver 3 (pinned in `rust-toolchain.toml`).
`unsafe_code` is `forbid` and `clippy::all` is `deny` workspace-wide; `missing_docs`
warns. Protobuf builds use the vendored `protoc`, so no ambient `protoc` is needed.

Install the Conventional Commit + pre-commit hooks once per checkout:

```bash
./scripts/install-git-hooks.sh
```

Optional mise provisioning is documented in `docs/develop/setup-testing.md`;
`docs/develop/toolchain-inventory.md` maps current pins and check owners. Keep
direct Cargo/script commands working. Local task plans, when present, live in the
primary checkout's ignored `.local/` directory. Reuse those notes across worktrees;
keep planning and handoffs out of published documentation and commits.

## Commands

Everything repo-wide runs through `cargo xtask` (source in `xtask/`). Run
`cargo xtask` with no arguments for its usage text.

### Test tiers — use the smallest one that proves the change

```bash
cargo test -p colossus-policy --lib                  # focused: one crate's unit tests
cargo test -p colossus-cli --test config_security    # focused: one integration target
cargo test -p colossus-policy --lib -- name_of_test  # single test by name filter
cargo xtask dev                                      # fast: cheap checks + all workspace lib tests
cargo xtask check rust                               # completion gate for Rust changes
cargo xtask pr --base origin/main                    # pre-PR: change-selected full validation
```

`cargo xtask check rust` owns formatting, crate-root structure, locked metadata, Clippy,
workspace tests, and fuzz-harness gates — the same gates PR validation uses. Run it before
declaring an implementation complete. **It needs permission to bind local loopback
sockets**; several integration tests start temporary local servers, and sandboxed runs
otherwise fail late with `Operation not permitted`.

The focused and fast tiers shorten the feedback loop but never replace the completion gate.

### Other check components

```bash
cargo xtask check sidecar
cargo xtask check sdk
cargo xtask check desktop
cargo xtask check docs
cargo xtask check dependencies
cargo xtask check workflows
./scripts/check_crate_roots.sh    # 250-line ceiling on tracked crate roots
```

### Building and running

```bash
cargo build --workspace
./scripts/colossus-dev --approval-mode full-access tui   # isolated dev TUI, state under .colossus/
./scripts/desktop-dev                                    # Desktop + debug Managed Local sidecar
./scripts/docs-site build && ./scripts/docs-site serve    # pinned containerized docs toolchain
```

For cold or cross-worktree builds, any Cargo command may be routed through
`./scripts/cargo-sccache` (e.g. `./scripts/cargo-sccache xtask dev`). Keep this opt-in:
plain `cargo` must keep working when `sccache` is absent.

## Architecture

Ports and adapters with a strictly inward dependency direction. Read
`docs/develop/architecture.md` before changing any boundary, and
`docs/develop/security-architecture.md` before touching tools, subprocess execution,
policy, audit, or bundle handling.

Layers, innermost first:

| Layer | Representative crates |
| --- | --- |
| Domain and contracts | `colossus-domain` (dependency-free), `colossus-contracts` |
| Ports (application-owned interfaces) | `colossus-ports` |
| Application services | `colossus-agent`, `-session`, `-context`, `-work`, `-memory`, `-workflow`, `-research`, `-telemetry` |
| Security and catalog | `colossus-access`, `-policy`, `-tools` |
| Infrastructure adapters | `colossus-provider`, `-journal-redb`, `-journal-postgres`, `-projection`, `-sandbox`, `-integrations`, `-mcp`, `-plugins`, `-bundles`, `-search`, `-codex-auth` |
| Public API and SDK | `colossus-api-proto`, `-api`, `-api-runtime`, `-grpc`, `-sdk` |
| Composition and interfaces | `colossus-runtime`, `-worker-protocol`, `-worker`, `-cli`, `-tui`, `-presentation` |

Rules that changes are judged against:

- `colossus-domain` has no dependencies. Ports are owned by the application, never by
  infrastructure.
- `colossus-runtime` is the only composition root: it constructs adapters and mints
  opaque permit-bearing executors. Adapter constructors stay private to it.
- CLI, TUI, Desktop, gRPC, and SDK crates are interfaces or translations only. No model,
  tool, policy, workflow, or state logic lives there — they construct requests, call
  application services, and render typed results.
- Every external or sensitive operation is an effect and takes one centralized,
  evidence-producing path: request journaled → Safety Kernel validation → policy decision
  → optional one-use approval → authenticated single-use permit → adapter → quarantined
  output → release decision → terminal journal event. A convenience helper must never
  bypass policy, quarantine, audit, or post-effect release; an adapter cannot mint its own
  authority.
- Canonical writes append journal events; read models and indexes are replaceable.
  Listing aggregates must page the stream-identifier index, never rescan global history.
- The Desktop process does not link runtime, model, tool, policy, or worker-host crates.
  It talks to the sidecar over `colossus-worker-protocol`.

## Rust conventions

- **Crate roots are maps, not implementation.** `lib.rs`/`main.rs` may hold crate docs,
  module declarations, public re-exports, and small composition code. Configuration
  parsing, protocol metadata, service methods, adapter implementations, command dispatch,
  rendering, and unit tests belong in modules named for their responsibility. The
  250-line ceiling is a backstop, not a target — extract a module rather than raising it.
- Split modules along concepts a maintainer can search for, not at line counts. A module
  is due for a split when it owns more than one reason to change.
- Use `pub(super)` or narrower between private siblings; moving code must not accidentally
  widen the public API. Preserve public paths with deliberate re-exports.
- Preserve object-safe port traits — runtime composition uses `dyn Trait`.
- `Result` for recoverable failure; `panic!`/`unwrap`/`expect` only in tests or where a
  local invariant already proves the state impossible.
- Never hold a synchronous lock across `.await`. Bound channels, tasks, retries, output,
  and concurrency, and document who owns shutdown and cancellation.
- Redact credentials, model-private content, untrusted bodies, and sensitive paths before
  they reach logs or error context.
- Do not add a crate, macro, cache, or alternate runtime because a generic Rust checklist
  suggests it. This repo's architecture, lints, and executable checks win over external
  advice. See `docs/develop/rust-practices.md`.

## Tests

Put each assertion at the lowest boundary that can prove the contract: unit tests next to
the module or in `src/tests.rs` / `src/tests/`; integration tests in `<crate>/tests/`;
end-to-end coverage in CLI/Desktop suites and CI workflows. Don't promote a unit test to a
process test for realism, and don't replace a security, protocol, persistence, or
packaging boundary test with a mock for speed.

Tests protect supported behavior, not history. When removing a feature, remove its
feature-specific tests; add rejection, migration, or tombstone tests only when the
post-removal behavior is itself an intentional compatibility or security contract. Add or
update tests for every other behavior change. Moving tests is structural work — preserve
fixture bytes exactly (YAML, JSON, signatures, hashes, whitespace-sensitive protocol
examples). `docs/develop/testing.md` records per-crate test ownership.

## Commits and review

Conventional Commits, enforced by the commit-msg hook. Allowed types: `build`, `chore`,
`ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `security`, `style`, `test`.

Before merging, inspect unresolved human and automated review threads plus required
checks, and address actionable findings in code and tests — a green build is not a
substitute for resolving review. Hosted pre-merge acceptance (`ci:full`) is a separate
final-PR step; see `docs/develop/ci-cd.md`.

## Where to read more

`docs/develop/` holds the deep detail: `architecture.md`, `security-architecture.md`,
`crate-structure.md`, `rust-practices.md`, `testing.md`, `setup-testing.md`,
`contributing.md`, `runtime-ports.md`, `ci-cd.md`, `releasing.md`, `application-sdk.md`.
Keep this file the short map and put new depth there.
