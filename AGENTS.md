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
| Front-end screens, components, CSS or themes | [Front-end development](docs/develop/frontend.md); [shared UI package](apps/ui/README.md) |
| Documentation | [Documentation authoring](docs/develop/documentation.md) |
| CI or release automation | [CI/CD](docs/develop/ci-cd.md); [releasing](docs/develop/releasing.md) |
| Legacy configuration or state | [Rust cutover decision](docs/develop/adr/0001-rust-runtime-cutover.md) |
| Commits, PRs or merge readiness | [Contributing](docs/develop/contributing.md) |
| Daily product maintenance or cross-client capability coverage | [Agent maintenance](docs/develop/agent-maintenance.md); [feature inventory](docs/develop/feature-inventory.md) |

## Product maintenance and client ownership

Use `maintenance_coordinator` for daily product health and cross-client work.
The [maintenance guide](docs/develop/agent-maintenance.md) owns roles, work
limits, shared reservations, independent review, research, and
[PR delivery](docs/develop/agent-maintenance.md#deliver-validated-fixes-as-pull-requests).
Validate scoped fixes, then deliver them as PRs in `obscuritylabs/Colossus`.
Explicit hosted requests, merges, releases, and deployments require separate
authorization and their normal prerequisites.

Assign shared behavior to its canonical service before adapting clients. For
capability changes, trace CLI, TUI, VS Code, Desktop, API/SDK, and applicable web
workflows. Record evidence and a named owner and action for deferred gaps in the
[feature inventory](docs/develop/feature-inventory.md) and PR.

## Front-end development

Prioritize `@colossus/ui` components and design tokens. Match existing settings surfaces
(such as Providers) before introducing new presentation patterns. Use shared typography,
color, radius, and control-size tokens; do not hardcode font sizes, font families, or
palette colors in feature CSS. Keep transport and capability decisions in the host.
Verify text-size preferences, supported palettes, compact layouts, keyboard behavior,
and the rendered screens as described in the [front-end guide](docs/develop/frontend.md).

## Validation and handoff

Use `cargo xtask` for repository checks; run it without arguments to list commands.
Iterate with `cargo xtask quick <check|clippy|test> -p <package>` and directly affected
test targets. Rust changes require the complete Rust gate before completion:
`cargo xtask check rust`, or `cargo xtask pr --base <revision>` when Rust is selected.
The PR gate includes that same Rust gate; do not run both on unchanged source.
Other component gates and pre-PR validation are described in
[Source setup](docs/develop/setup-testing.md). Focused checks do not replace completion
gates. Resolve actionable review findings and required checks before merging, following
[Contributing](docs/develop/contributing.md).

When continuing planned work, reuse the primary checkout's ignored `.local/` notes
across worktrees. Keep task plans and handoffs out of published docs and commits.
