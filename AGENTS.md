# Colossus

Colossus is a Rust runtime for AI agents with explicit authority boundaries.

## Read for the task

Read the relevant guide before editing its area; follow its links for deeper detail.

| When working on | Read |
| --- | --- |
| Setup, toolchains or local run commands | [Source setup](docs/develop/setup-testing.md); [toolchain inventory](docs/develop/toolchain-inventory.md) |
| Crate dependencies, service ownership or interface boundaries | [Architecture](docs/develop/architecture.md) |
| Tools, subprocesses, policy, audit, sandboxing or bundles/plugins | [Security architecture](docs/develop/security-architecture.md) |
| Rust implementation or module layout | [Rust practices](docs/develop/rust-practices.md); [crate structure](docs/develop/crate-structure.md) |
| Persistence, projections or recovery | [State and recovery](docs/develop/state-recovery.md) |
| Adding, moving or removing tests | [Test strategy](docs/develop/testing.md) |
| Public API, SDK or Desktop transport | [Application SDK](docs/develop/application-sdk.md) |
| Documentation | [Documentation authoring](docs/develop/documentation.md) |
| CI or release automation | [CI/CD](docs/develop/ci-cd.md); [releasing](docs/develop/releasing.md) |
| Legacy configuration or state | [Rust cutover decision](docs/develop/adr/0001-rust-runtime-cutover.md) |
| Commits, PRs or merge readiness | [Contributing](docs/develop/contributing.md) |

## Validation and handoff

Use `cargo xtask` for repository checks; run it without arguments to list commands.
Use focused tests while iterating. Rust changes require `cargo xtask check rust`
before completion; other component gates and pre-PR validation are described in
[Source setup](docs/develop/setup-testing.md). Focused tests do not replace completion
gates. Resolve actionable review findings and required checks before merging, following
[Contributing](docs/develop/contributing.md).

When continuing planned work, reuse the primary checkout's ignored `.local/` notes
across worktrees. Keep task plans and handoffs out of published docs and commits.
