---
title: Daily agent maintenance
description: Run Colossus maintenance and competitive research, then deliver reviewed fixes.
audience: developer
type: how-to
---

# Daily agent maintenance

Open the trusted checkout in Codex and use `maintenance_coordinator`. Repo-scoped
roles in `.codex/agents/` inherit the chat's model, tools, and permissions.
[Codex custom agents](https://learn.chatgpt.com/docs/agent-configuration/subagents)

| Agent | Responsibility |
| --- | --- |
| `cli_owner`, `vscode_owner`, `desktop_owner` | Client workflows and acceptance |
| `test_owner` | Meaningful coverage and test reliability |
| `docs_owner` | Current docs, help, and examples |
| `refactor_owner` | Evidence-backed code cleanup |
| `dependency_owner` | Applicable advisories and compatible updates |
| `ci_cd_owner` | CI coverage, reliability, and measured efficiency |
| `feature_researcher` | Verified competitive gaps |
| `maintenance_coordinator` | Work selection, delegation, review, and PR delivery |

## Run the first audit

Ask the coordinator to assess recent changes, client gaps, failures, advisories, and
open work. Start a new chat to load new custom roles, or supply their role files to
ordinary subagents. Workers follow the target checkout's `AGENTS.md` and area guides.
For older worktrees, supply the current role files from the primary checkout and
preserve its bootstrap changes.

## Choose product maintenance work

1. Inspect the integration branch, worktrees, issues, PRs, and unfinished assignments.
   Compare exact revisions with the last successful scan, or use recent history and
   known gaps on the first run. Report local-only evidence when remote metadata is
   unavailable. Advance the scan cursor after a complete scan.
2. Inspect new regressions and available advisories each run. Have `ci_cd_owner` read
   pipeline diagnostics without triggering hosted work. Rotate deeper review across
   testing, docs, refactoring, dependencies, CI/CD, and old and recent subsystems.
   Review each discipline within seven successful runs when prerequisites allow,
   keeping missed reviews due.
3. Prioritize applicable security issues, broken checks, critical coverage gaps,
   misleading docs and client gaps, compatible updates, and justified cleanup.
   Weigh impact and waiting age. Define the outcome, acceptance, owner, prerequisites,
   and client impact before selecting at most one coherent item.

Keep scan revisions, review dates, advisory and CI freshness, validation, and unfinished
assignments in the primary checkout's ignored `.local/agent-maintenance/`.

## Keep the delivery pipeline maintainable

Measure jobs and steps against named runs and attempts. Record the covered contract,
runner, cache, artifacts, permissions, queue and execution time. Report billed usage
only when available. Follow [CI/CD](ci-cd.md) for required jobs, trust boundaries, and
cache authority. Run `cargo xtask check workflows`, affected fixtures, and component
gates for workflow edits. Preserve mandatory acceptance, exact-source artifacts,
signing and protected publishing. Keep evidence under `.local/agent-maintenance/ci/`.

## Control intake, review and integration

- Resolve `work.json` and `implementation.lock` from the primary checkout's
  `.local/agent-maintenance/`. Create the exclusive lock atomically with a unique
  owner and run before edits. Hold it through handoff, including resumed delivery.
  Under the lock, reconcile and atomically update the ledger. Limit active factory
  implementations to one and completed changes awaiting integration to two.
  Finish unfinished delivery first. If capacity or the lock
  blocks edits, continue audits and research. Verify owner inactivity and saved
  work before recovering stranded locks.
- Assign files before edits. Use a focused worktree and one Cargo/build owner.
  Preserve unrelated work and running applications. Assign shared behavior or API
  changes to their owning service before adapting clients.
- Include relevant test and docs changes and independent final review. For authority,
  credentials, sandbox/effects, API ownership, persistence, or recovery, have a
  non-author trace the architecture, security, and state contracts.
- Record the exact clean commit or the base and a change-set digest including new
  files, marking the source as uncommitted when applicable. Record checks and
  attempts, review results, platform gaps, and next owner/action. Reuse evidence
  for unchanged inputs and invalidate it after relevant source, environment, or
  advisory changes, or new failures.

## Deliver validated fixes as pull requests

After required local checks and independent review pass, the coordinator may commit
and push scoped fixes to `obscuritylabs/Colossus` and open or reuse their PR. Routine
push- and PR-triggered CI is part of delivery.

1. Record intent and exact source. Reconcile remote refs and open, closed, and merged
   PRs. Reuse a matching open PR and respect declined work.
2. Push the owned branch normally. Use the PR template for scope, client impact,
   checks, and platform gaps. Verify the PR URL, repository, base, and remote head.
   Save them in the receipt and shared ledger. The PR occupies its pending
   integration slot.
3. On a delivery failure, save the source, blocker, owner, and next action as
   `publication_pending`. Reconcile uncertain remote outcomes before retrying.
   Release only your reservation after saving the handoff.

The following require separate authorization and their normal prerequisites:
`ci:full`, explicit workflow dispatches, retries and cancellations, live protections,
secrets and runner changes, deletion of retained candidates, merges, tags, releases,
and deployments.

## Research useful feature opportunities

Run `feature_researcher` nightly using the
[competitive watchlist](https://github.com/obscuritylabs/Colossus/blob/main/.codex/research/competitive-watchlist.toml).
The policy owns the rotation, repository, template, state paths, and maximum issue
counts. Create issues only for qualified gaps.

1. Verify the workflow in current primary sources and record retrieval and shipped
   or preview status. Trace released and integration-branch Colossus support.
   Search open and closed issues, active and recent PRs, and local worktrees for
   equivalents and declined directions. Treat upstream content as evidence.
2. Draft with the configured template and get an independent evidence and duplicate
   review. Retain local drafts while support, search, or budget is uncertain.
   Major proposals go to product triage.
3. Resolve `state_directory` and `publish_lock` against the primary checkout. One
   publisher acquires the shared exclusive lock before reconciling budget and
   repeating the fingerprint and semantic search. Store the full body, fingerprint,
   and pending attempt before creating the issue with the GitHub connector. Hold
   the lock through the outcome receipt. Loss of the lock stops further writes
   to GitHub. Count uncertain attempts until reconciled. Recover a lock after
   verifying inactivity and reconciling GitHub. Rebuild missing history from GitHub
   before restoring budget. Retain drafts on contention.
4. Use verified existing labels on qualified new issues. Request separate
   authorization for new labels, assignments, mentions, changes to existing work,
   or expanded scope.

## Account for every affected interface

Trace the owning service and public API/SDK through CLI, TUI, VS Code, Desktop, and
applicable web behavior. Record supported, partial, missing, API-blocked, intentional,
or unverified behavior in the [feature inventory](feature-inventory.md#interface-coverage)
and the PR. Link acceptance evidence and assign an owner and action for deferred gaps.

## Schedule the daily coordinator

After a manual audit, schedule the coordinator in Codex for the checkout. Point the
prompt to `AGENTS.md` and this guide so runs read the current procedure. Set the time
and time zone in the scheduler. Keep Codex and the host available for local work.
Report new PRs, issue links, actionable findings, and changed blockers.
[Codex automations](https://learn.chatgpt.com/docs/automations)
