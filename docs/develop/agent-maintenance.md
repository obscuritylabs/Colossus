---
title: Daily agent maintenance
description: Coordinate nightly Codex product maintenance, CI/CD health, review and integration handoffs, and competitive feature research.
audience: developer
type: how-to
---

# Daily agent maintenance

Open the trusted Colossus checkout in Codex and start a new chat to load the
repo-scoped agents in `.codex/agents/`. The roles inherit the parent session's
model, permissions, and available tools. They define responsibilities, not
separate access controls or running services. See the
[official custom-agent guide](https://learn.chatgpt.com/docs/agent-configuration/subagents).

| Role | Responsibility |
| --- | --- |
| CLI / TUI owner | CLI commands, TUI workflows, terminal presentation, help, and their acceptance evidence |
| VS Code owner | Extension host, editor workflows, validated webview protocol, public TypeScript SDK integration, and acceptance evidence |
| Desktop owner | Desktop workflows, native Rust SDK adapters, renderer integration, and native/rendered acceptance evidence |
| Test owner | Meaningful system/regression coverage, failure paths, flaky-test causes, and real boundary acceptance |
| Docs owner | Accurate user/operator/developer guides, CLI help, configuration examples, SDK docs, and client limitations |
| Refactor owner | Justified cleanup, clear module/service ownership, reduced duplication, and behavior-preserving refactoring |
| Dependency owner | Applicable advisories, compatible upgrades, deprecations, toolchain/pin consistency, and stale policy exceptions |
| CI/CD owner | Reliable checks, measured build efficiency, workflow clarity, caches, artifact handling, and pipeline trust boundaries |
| Feature researcher | Competitive workflow research, verified Colossus gaps, and qualified GitHub feature proposals |
| Maintenance coordinator | Product-health priorities, client coverage, review rotation, shared prerequisites, delegation, integration, and independent review |

The corresponding spawn names are `cli_owner`, `vscode_owner`, `desktop_owner`,
`test_owner`, `docs_owner`, `refactor_owner`, `dependency_owner`, `ci_cd_owner`,
`feature_researcher`, and
`maintenance_coordinator`.

Application services continue to own behavior. Public API/SDK work goes to its
canonical implementation through a focused worker; it is not copied into a
client to close a UI gap. Shared UI changes require checking Desktop, VS Code,
and web. Consult [architecture](architecture.md), [application SDK](application-sdk.md),
and [shared UI](https://github.com/obscuritylabs/Colossus/tree/main/apps/ui).

## Run the first audit

Ask Codex:

> Use maintenance_coordinator to assess product health across runtime services
> and clients. Delegate test coverage, docs, code maintainability, and dependency
> review to test_owner, docs_owner, refactor_owner, and dependency_owner.
> Have ci_cd_owner inspect pipeline health and unnecessary execution using
> available run evidence, and the client owners inspect applicable workflows.
> Deduplicate existing work, select one concrete maintenance item, implement it
> with appropriate regression evidence and docs, get independent review, and run
> the required completion gates. Commit and deliver the validated fix through the
> automatic pull-request handoff below, then report its verified PR URL.

If the current chat predates the role files or the client does not expose custom
roles, start a new chat. A coordinator can also delegate ordinary subagents with
the corresponding role file supplied as instructions.

If a feature worktree predates this setup, the coordinator can supply the primary
checkout's role instructions and this maintenance guide to workers explicitly.
Follow the target checkout's current `AGENTS.md` and area guides, preserve the
primary checkout's uncommitted setup, and keep its bootstrap diff out of unrelated
fixes. Missing custom-role discovery alone does not restrict work to an audit.

Inspect existing branches, worktrees, and follow-ups before selecting work.
Keep an exact scan base and unresolved assignments in the primary checkout's
ignored `.local/agent-maintenance/` directory. On the first run, use recent
history and the inventory's known gaps. Later runs inspect changes since the
last successful scan and revisit unresolved gaps; a clean recent diff does not
erase older unfinished work. Record which branch/revision was scanned and
whether the remote was refreshed. Never describe a local-only scan as current
remote coverage.

## Choose product maintenance work

Maintain the supported product across runtime/services, public API/SDK, clients,
and native/deployment boundaries. Recent feature diffs are one input; older code,
stale instructions, package support, and unprotected failure cases also need review.

| Discipline | Useful work and acceptance |
| --- | --- |
| Test quality | Map important supported behaviors to assertions; add missing authorization, cancellation, invalid/stale input, recovery, and user-workflow cases. Reproduce and fix flaky-test causes. Follow the existing [test strategy](testing.md); mocks and test counts do not prove real boundaries. |
| Documentation | Compare implemented behavior with canonical guides, help, configuration examples and SDK docs. Fix stale claims/removals and missing instructions, then validate executable examples and rendered pages using [documentation authoring](documentation.md). |
| Code maintainability | Address demonstrated duplication, unclear ownership, dead code, or error-prone control flow with a bounded refactor. Preserve public behavior, fixture bytes, authority and recovery contracts; follow [Rust practices](rust-practices.md) and [crate structure](crate-structure.md). |
| Dependencies | Verify advisories, upstream release notes, support/deprecations and existing exception-removal conditions. Update one compatible family across its actual graphs/consumers, using the [toolchain inventory](toolchain-inventory.md) and [dependency policy](ci-cd.md#pull-request-validation). Run `cargo xtask check dependencies` plus affected completion gates. |
| CI/CD health | Diagnose failures and measured duplicate/slow execution; simplify obsolete or drifting setup while preserving covered contracts and PR/acceptance/release tiers. Follow [CI/CD](ci-cd.md), [release operations](releasing.md) and `cargo xtask check workflows` for workflow changes. |

Each daily run checks new regressions/failures and available advisory/update
evidence, including available pipeline failures, then chooses a deeper review
using last-reviewed dates and overdue follow-ups. Review each of the five disciplines within seven successful daily
runs when prerequisites allow; keep blocked/missed reviews due. Rotate through
product subsystems as well, so a quiet or mature area is not forgotten. Record
category/subsystem review dates and advisory freshness separately from the
code-change scan cursor in `.local/agent-maintenance/`.

Prioritize applicable security problems, broken checks/regressions, critical
coverage gaps, misleading docs/client gaps, compatible updates, and then justified
cleanup. Consider impact and waiting age rather than requiring a change every
night. Do not chase test counts, invent refactors, force breaking upgrades, weaken
checks, or suppress an advisory to make a run successful. Existing dependency
exceptions must retain their stated applicability and removal conditions.

Implement at most one coherent maintenance item per run, including its relevant
test and documentation repairs. Parallelize independent investigation, assign
files before editing, and serialize shared API/UI changes and Cargo builds.
Have the test and docs owners review applicable changes, then obtain independent
final review. Preserve dirty checkouts and active user applications. Use a
focused branch/worktree and the [source completion gates](setup-testing.md).
Reuse valid evidence for unchanged source unless a new failure, environment or
advisory change, or unresolved concern justifies another check. Record unverified
native/platform acceptance explicitly.

## Keep the delivery pipeline maintainable

`ci_cd_owner` owns workflow health and build efficiency. It coordinates test
failures with the test owner, tool/action/image pins with the dependency owner,
and operational instructions with the docs owner. Portable check recipes stay
in `cargo xtask`; workflows own hosted orchestration and provisioning.

Start with a read-only baseline of current job purpose, covered contract,
trigger/tier, runner, dependency, timeout, concurrency, cache, artifacts and
permissions. Inspect available recent runs using exact commits, run IDs and
attempts. Distinguish product failures, infrastructure failures, valid skips and
cancellations. Record queue/wall time, critical paths, repeated failures and
cache behavior when available, with date and sample size. Unknown telemetry
stays unknown; wall time is not billed cost and a cache hit alone is not a speedup.

Use that evidence to choose a bounded cleanup and show the resulting benefit
with coverage preserved. Large files or fewer jobs alone do not establish a
problem or improvement. Keep trusted-base classification/aggregation, fail-closed
selection, mandatory acceptance, cache read/write authority, exact-source
artifacts, signing and protected publishers intact. Current policy requires all
pre-merge/platform acceptance jobs; a lighter policy needs a separate decision.

Workflow edits need `cargo xtask check workflows`, affected fixtures and required
component completion gates. Hosted behavior remains unverified until the
authorized run executes. Routine nightly inspection does not dispatch/rerun or
cancel workflows, request `ci:full`, change live secrets/protection/runner
allocations, delete retained candidates, or publish/deploy. Those operations
retain their existing authorization and final-head/release prerequisites.
Keep baselines, proposed improvements and exact evidence in the primary checkout's
ignored `.local/agent-maintenance/ci/` directory.

## Control intake, review and integration

The coordinator also owns these delivery responsibilities. Assign a named
reviewer or next owner for each item rather than creating a separate always-on
agent for every phase.

- **Ready work:** establish the supported user outcome, acceptance criteria,
  canonical owner, prerequisites and client impact before implementation.
  Research proposals require product triage before major implementation.
- **Work in progress:** keep at most one active factory-owned implementation
  across runs and two completed changes waiting for human integration. Resume
  unfinished work first. All coordinators use the primary checkout's
  `.local/agent-maintenance/work.json` ledger and
  `.local/agent-maintenance/implementation.lock` reservation; do not resolve
  these paths from individual worker worktrees. Atomically acquire the lock
  with exclusive creation and a unique owner/run identity before assigning
  edits, and hold it through the active handoff. While holding it,
  reread/reconcile the active and completed counts, check capacity, and
  atomically update the ledger.
  Failed acquisition or full capacity means no new implementation edits/start.
  Recover a stranded reservation only after
  verifying owner inactivity and saved work, not elapsed time. Retain blocked
  or parked work with a next action. Before new intake, finish publication of
  existing validated local fixes, including when capacity is full. Opening a PR
  does not free a completed change's integration slot. When capacity is full,
  continue publication handoffs, audits and research, and report an unchanged
  required handoff only once.
- **Independent review:** the author does not approve their own patch. Assign
  explicit architecture/security review for authority, credentials, sandbox or
  effect paths, API/ownership, persistence and recovery changes using the
  [architecture](architecture.md), [security](security-architecture.md) and
  [state/recovery](state-recovery.md) guides. Record findings, resolutions and
  review limits; test/dependency scans do not replace this reasoning.
- **Integration receipt:** retain the exact clean source commit, or the base and
  a digest of the assigned change set including new files with an explicit
  uncommitted status. Include checks/results/attempts, independent findings and
  resolutions, native/platform gaps, and a named next owner/action. Invalidate
  evidence after source or relevant input changes. Local checks establish their
  stated scope; hosted final-head acceptance and immutable release candidates
  remain prerequisites for authorized integration and release. Delivered fixes
  also need the verified PR URL and exact published head described below.

These work limits and receipts are agent working conventions. They preserve a
reviewable handoff and do not introduce automatic merge or release authority.

## Deliver validated fixes as pull requests

The maintenance coordinator has standing user authorization to commit validated
maintenance fixes, push their focused branches and open pull requests in
`obscuritylabs/Colossus`. This includes documentation, tests, cleanup, compatible
dependency updates and CI/CD repairs. Delivering a PR is part of finishing the
maintenance item; a fix must not be reported as delivered while it exists only
locally. This authorization also covers publishing previously completed local
fixes before selecting new work.

The coordinator is the single publisher under the existing primary checkout's
implementation reservation. Keep the shared ledger reconciled while holding the
reservation, including when resuming publication of an older completed item.
Complete the handoff in this order:

1. Resolve actionable independent review findings and pass the required local
   completion gates on the final source. Reuse valid unchanged-source evidence
   under the rules above. Preserve any remaining hosted or platform acceptance
   gaps explicitly.
2. Commit only the assigned changes on the focused branch. Record the exact
   source commit and intended repository, branch and base before publication.
   Check remote branch state and existing open or closed PRs before creating a
   new PR. Reuse an existing open PR whose head matches the validated commit.
   Reconcile a different head or an already merged change before acting. Respect
   closed or declined PR decisions; do not automatically reopen them or create
   replacement PRs.
3. Push only the owned branch with a normal push. Do not force-push or update
   unrelated refs. Open the PR using the repository template, with the supported
   outcome, client coverage, checks, independent review and unverified acceptance.
   Routine CI triggered by the authorized push or PR is expected.
4. Verify the returned PR URL, repository, base and exact head. Save them with
   the check and review evidence in the integration receipt and shared work
   ledger, then report the PR URL. An open PR remains a completed change awaiting
   integration and continues to count toward the limit of two.

If checks, credentials, network access, push or PR creation prevent delivery,
retain a durable `publication_pending` record with the exact saved source,
failure or uncertainty, next owner and next action. Do not claim a delivered PR
without a verified URL and head. Reconcile GitHub branch and PR state before
retrying an uncertain creation so a lost response cannot create duplicate PRs.
Keep blocked work due for the next run, and release only the owned reservation
after its saved handoff is complete. Capacity limits prevent new implementation,
not recovery and publication of already completed work.

This standing authorization covers branch pushes and PR creation. Explicit
workflow dispatches, reruns or cancellations, `ci:full` requests, changes to live
protections, secrets or runner allocations, candidate deletion, merges, tags,
release publication and deployments still require separate authorization and
their existing final-head, platform and release prerequisites. Opening a PR does
not establish integration or release readiness.

## Research useful feature opportunities

The coordinator dispatches `feature_researcher` each night alongside maintenance.
It reads the agent policy in
[competitive-watchlist.toml](https://github.com/obscuritylabs/Colossus/blob/main/.codex/research/competitive-watchlist.toml),
which owns the target repository, products, sources, themes, rotation and issue
limits. This is an agent-read policy, not native Codex scheduling configuration.
Resolve its `state_directory` and `publish_lock` against the primary checkout
root, never an individual worker worktree. Every run and worktree shares that
research ledger and exclusive publishing lock.
The initial watchlist covers Codex, Claude Code, Cursor, Cline, Aider and OpenHands.
Rotate two products per run and keep source reviews due until completed; the
policy targets all products within seven successful runs.

Translate a competitor workflow into a useful user outcome, then inspect current
Colossus service/API and client behavior at named revisions. Preserve the
distinction between released support, integration-branch code, active PRs,
partial support and intentional authority/deployment differences. Verify claims
with current official docs, release notes or maintainer source, including the
retrieval date and shipped/preview status. A source landing page alone does not
establish a particular capability. Missing source/network access leaves a draft,
not a claim that Colossus is behind.

Search open and closed issues, active and recent merged/closed PRs, and local
worktrees with capability synonyms and user outcomes. Read likely matches;
checking only recent issue titles is insufficient. Link existing work and respect
declined proposals. Material new evidence for a declined direction needs a
product decision rather than automatic reopening or a replacement issue.

For a worthwhile, verified gap, prepare the complete
[feature proposal template](https://github.com/obscuritylabs/Colossus/blob/main/.github/ISSUE_TEMPLATE/competitive-feature.md).
Include the user problem/value, source evidence, actual Colossus coverage,
client/API scope, alternatives, service/authority implications, rough scope and
confidence, acceptance criteria, related work and unknowns. Get an independent
review of the evidence and duplicate search before creating the issue through
the GitHub connector in the configured repository. These proposals are ready for
product triage; they do not approve major implementation or roadmap priority.

The initial limits are at most one new issue per nightly run and three per
rolling seven days, with no quota. Keep source/rotation records, drafts, stable
capability fingerprints, pending create attempts and returned issue URLs in the
primary checkout's ignored `.local/agent-maintenance/research/` directory. Search
the fingerprint and semantic equivalents again immediately before posting and
record the prepared proposal before the mutation. An uncertain create outcome
reserves budget until reconciled; never retry blindly. Rebuild lost posting
history from GitHub before claiming fresh budget. Unavailable search or unknown
budget leaves a local draft. Limits and duplicate handling are agent instructions,
not a separate server-enforced issue budget.

Parallel research/review uses one publisher. Before the final budget and duplicate
checks, atomically acquire the configured shared publishing lock with a unique
run/owner identity. Hold it through the pending-attempt write, issue creation and
receipt recording; retain the draft when another publisher owns it. A stranded
lock needs verified owner inactivity and GitHub reconciliation before recovery,
not an elapsed-time expiry. Unknown outcomes retain their pending reservation.

Qualified issue creation is part of the research assignment. This research
authorization does not permit changing issue state, comments, assignees, labels
or milestones on existing work, nor pushing code, approving major features,
merging or publishing releases.
Use only verified existing labels on a new proposal when appropriate. Notify for
a new issue or an actionable changed decision/blocker; retain unchanged research
candidates quietly. Major proposals go through normal product triage before
the implementation agents take them on.

## Account for every affected interface

For each capability addition, change, or removal, trace the canonical service and
public API/SDK, then the CLI, TUI, VS Code, Desktop, and applicable web workflows.
Check discovery, configuration, execution, approvals, cancellation, inspection,
and reconnection where relevant. The same behavior can use different controls.
An agent tool or a rendered control alone does not prove an operator workflow.

Maintain material coverage records and acceptance links in the existing
[feature inventory](feature-inventory.md#interface-coverage). Use:

- **Supported:** the applicable workflow is implemented; link acceptance evidence.
- **Partial:** identify the implemented subset and the missing operation.
- **Missing:** applicable client behavior is absent.
- **API-blocked:** identify the public contract prerequisite and its owner.
- **Intentional difference:** explain the product, deployment, or authority reason.
- **Unverified:** available evidence does not establish support.

Record a named owner and concrete follow-up for deferred gaps, linking an
existing issue or PR when available. Keep daily plans, logs, and handoffs under
`.local/`; the published inventory owns durable capability facts only. Include
coverage in capability-changing PRs using the repository PR template. This is
an agent/reviewer requirement; it is not an automated proof of client parity.

## Schedule the daily coordinator

After inspecting a manual run, create one daily scheduled task in Codex for this
checkout. A task in the maintenance chat preserves its context; a standalone
task is suitable when each run should begin independently. Use this durable prompt:

> Run daily Colossus product maintenance using maintenance_coordinator and the
> current AGENTS.md and docs/develop/agent-maintenance.md. Inspect new changes,
> failures, advisory/update evidence and unresolved work in the primary checkout's
> .local/agent-maintenance/ notes. Delegate relevant work to the client, test,
> docs, refactor, dependency, and CI/CD owners. Inspect available pipeline failures
> without triggering hosted work. Rotate deeper review across the five
> maintenance disciplines within seven successful runs and through older product
> subsystems. Dispatch feature_researcher nightly using the competitive watchlist
> and issue policy; verify sources/current support, deduplicate, independently
> review and create only qualified proposals within the issue limits. Resolve
> research state_directory and publish_lock against the primary checkout, never a
> worker worktree; all runs share one research ledger and publishing lock.
> Research proposals await product triage. Deduplicate work and implement at most one clearly specified,
> evidence-backed maintenance item with its test/docs repairs. Use a focused
> branch/worktree and ready-work intake. Coordinate through the primary checkout's
> .local/agent-maintenance/implementation.lock reservation and
> .local/agent-maintenance/work.json ledger, using exclusive lock creation,
> independent security/architecture review where relevant, and all required gates.
> Keep one active implementation and at most two completed changes awaiting human
> integration; finish or hand off existing work before starting another. Resume
> publication of validated local fixes even when capacity is full; opening a PR
> does not free an integration slot. Keep exact scan
> revisions, category review dates, freshness, validation and unresolved work,
> plus an exact-source receipt with the next owner/action for every completed item.
> After required local gates and independent review pass, commit the assigned
> maintenance fix, push its focused branch and open or reuse a PR in
> obscuritylabs/Colossus under the automatic delivery policy above. Verify and
> record its URL and exact head in the shared ledger and integration receipt.
> Retain publication_pending work and reconcile uncertain outcomes before retries.
> Qualified feature-issue creation follows the research policy. Explicit hosted
> dispatch/rerun/cancellation or ci:full, live infrastructure changes, merges,
> tags, releases and deployments still require separate authorization and their
> prerequisites. Report new PR URLs and actionable findings, completed changes,
> failures or meaningful blocker changes; stay quiet while unchanged.

Choose the daily time and time zone in the scheduler. Keep the computer on and
Codex running for work that needs local files. Review the first few results and
adjust the bounded assignment if needed. Role files alone do not activate a
schedule. See [official scheduled-task guidance](https://learn.chatgpt.com/docs/automations).
