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

Repo-scoped Codex roles live in `.codex/agents/`: `cli_owner` (CLI and TUI),
`vscode_owner`, `desktop_owner`, `test_owner`, `docs_owner`, `refactor_owner`,
`dependency_owner`, `ci_cd_owner`, `feature_researcher`, and
`maintenance_coordinator`. Use the coordinator for daily product health and
changes that cross interfaces; give workers explicit file
ownership before editing shared code. Role definitions do not schedule daily runs.

Maintain meaningful system and failure-path test coverage, accurate canonical
documentation, compatible dependencies, CI/CD health and justified code cleanup alongside
client features. Review quiet subsystems as well as recently changed code.
Prioritize evidence-backed risks and regressions; avoid test-count targets,
speculative refactoring, dependency churn, or weakened checks. Include test and
documentation impact in every maintenance change and run its required gates.

CI/CD cleanup needs evidence of improved reliability, clarity or build efficiency
with required coverage preserved. Keep trusted-base selection, final-head
acceptance, cache authority and immutable release/publisher boundaries intact.
The coordinator owns ready-work intake, bounded work in progress, independent
security/architecture review for relevant changes, and exact-source integration
receipts with the next owner/action. Local passes do not establish release readiness.

Nightly competitive research follows `.codex/research/competitive-watchlist.toml`.
Verify upstream evidence and current Colossus support, deduplicate open/closed
issues and active/recent PRs, and independently review qualified proposals before
creating issues within the configured limits. Keep research notes and posting
receipts in ignored `.local/agent-maintenance/research/`. A feature proposal does
not authorize major implementation or establish roadmap priority.

For every capability addition, change, or removal, account for CLI, TUI, VS Code,
Desktop, the public API/SDK, and web where applicable. Record supported behavior,
gaps, intentional differences with reasons, and acceptance evidence in the
[feature inventory](docs/develop/feature-inventory.md). A runtime agent tool alone
does not establish a human-facing client workflow. Keep shared behavior in its
canonical service and assign missing API work before client adapters. Include this
coverage in the PR description; deferred gaps need a named owner and follow-up.

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
