---
title: Tasks, decisions, and plans
description: Track actionable work, preserve binding commitments, and approve a plan before execution.
audience: user
type: how-to
---

# Tasks, decisions, and plans

## Goal

Turn a session into durable, inspectable work: a task for status, a decision that steers
future turns, and an approved plan whose execution is explicit.

## Prerequisites

- An active session ID from `sessions list`.
- A provider route for model-generated plans.
- Approval access for the plan transition and any later effects.

## Steps

### 1. Create and update a task

```bash
colossus --config .colossus/config.yaml tasks create SESSION_ID \
  "Run release gates" --description "Capture the outputs"
colossus --config .colossus/config.yaml tasks list --session SESSION_ID
colossus --config .colossus/config.yaml tasks update TASK_ID \
  --status in-progress
```

Tasks are actionable records. Their status can be `pending`, `in-progress`, `completed`,
`blocked`, or `cancelled`.

### 2. Record a binding decision

```bash
colossus --config .colossus/config.yaml decisions create SESSION_ID \
  "Storage authority" \
  "The hash-chained journal is authoritative" \
  --priority high --rationale "Projections can be rebuilt"
colossus --config .colossus/config.yaml decisions list \
  --session SESSION_ID
```

Active decisions enter later model context as commitments. Archive or supersede a
decision when it no longer applies; do not create a contradictory active duplicate.

### 3. Generate a non-mutating plan

```bash
colossus --config .colossus/config.yaml run --plan \
  --session SESSION_ID \
  "Plan the requested repository change without changing the workspace"
```

Plan Mode narrows the tool surface structurally. You can also create a draft directly:

```bash
colossus --config .colossus/config.yaml plans create SESSION_ID \
  "Cut a release" --step "Run gates" --step "Build archives"
```

Every completed Plan Mode turn performs exactly one successful durable write:
`plan.create` for a new Draft or `plan.update` for the runtime-bound selected Draft.
The model cannot choose another plan ID. A second write is blocked before dispatch. If
the model answers without writing, Colossus permits one corrective turn and then fails
closed. That corrective turn exposes only the runtime-bound `plan.create` or
`plan.update` tool, so it cannot be consumed by more inspection or another interactive
question. A failed or cancelled turn can therefore have persisted zero or one plan;
inspect the typed `PlanWritten` event or returned plan evidence before retrying.

The ordered `PlanStep` values saved by `plan.create` or `plan.update` are part of the
Plan. They are not `TaskRecord` values and do not appear under `/tasks`. Plan Mode does
not expose `task.create` or `task.update`: this keeps planning non-mutating except for
its single bounded Plan write. Create durable Tasks explicitly when that separate
tracking workflow is useful.

New plans start at revision 1. Existing legacy records without the field read as
revision 0. Refinement replaces Markdown content and ordered steps while preserving the
original prompt, and every refinement or lifecycle transition increments the optimistic
revision. Stale updates, approvals, discards, and executions are rejected.

### 4. Create or refine in the terminal

Full-screen TUI and non-TTY line mode use the same process-local workflow in embedded
and worker-backed runtimes:

```text
/plan new
Plan the release, including rollback and focused verification.
/plan show
```

The completed turn selects its Draft and opens a review dock with Keep refining,
Approve, and Discard choices. Approval then opens the Direct/Goal execution dock.
Another ordinary prompt in Plan mode refines that exact selected revision. To return to
an existing plan:

```text
/plans
/plan use PLAN_ID
/plan show
```

`/plan use` accepts only a same-session Draft or Approved plan. An Approved plan cannot
be refined. `/plan new` enters Plan mode and clears selection without discarding the old
record; `/plan off` returns to Execute mode while retaining selection. Mode survives a
session switch, but selection does not, and neither is restored after process restart.

### 5. Approve, discard, or execute

```bash
colossus --config .colossus/config.yaml plans show PLAN_ID
colossus --config .colossus/config.yaml --approval-mode ask \
  plans approve PLAN_ID
colossus --config .colossus/config.yaml run --execute-plan PLAN_ID
```

Approval advances the reviewed Draft to Approved. Execution still authorizes every tool
effect independently. In the terminal, use:

```text
/plan approve
/plan execute direct
```

`/plan discard` is an operator-only transition for a selected Draft or Approved plan; it
retains the record for audit. `/plan execute` without a strategy offers Direct, Goal
Mode, and Cancel. Direct uses one ordinary execution run. Goal Mode defaults to five
iterations, accepts an explicit value from 1 through 50, and preserves the source plan
as lineage:

```text
/plan execute goal 12
```

Cancel or failure before consumption preserves Plan mode and selection. Once consumption
commits, the plan becomes Executed, the terminal switches to Execute mode, and selection
clears even if the subsequent direct run or Goal run fails or is cancelled. Goal failure
or cancellation leaves the Goal Active. Resume only its remaining budget:

```text
/goal resume GOAL_ID
```

The public `RunMode::Plan` entry point without a continuation continues to mean “create
a plan.” Completed results and cancellations expose optional canonical `plan_id`,
`plan_revision`, and `plan_status` fields in the public API, protobuf contract, and SDK
terminal types. A cancellation before persistence leaves them absent.

Colossus Desktop turns a returned Draft into an in-chat decision card:

- **Revise in chat** starts another Plan Mode run bound to the source run and exact
  visible revision.
- **Run once** approves and consumes that exact revision in one ordinary execution run.
- **Run as Goal** does the same with an explicit bounded iteration budget.
- **Advanced workflow** opens the TUI for the complete lifecycle surface.

The typed actions carry a source run ID and revision rather than a renderer-selected
Plan ID. They are available only when authenticated discovery advertises
`plans.continue`; clients fail closed when it is absent.

## Expected result

The session exposes current work, its active commitment, and a plan with an auditable
status and revision. An approved plan can be consumed atomically once, directly or into
bounded Goal Mode, through the normal effect gateway.

## Verification

```bash
colossus --config .colossus/config.yaml tasks show TASK_ID
colossus --config .colossus/config.yaml decisions show DECISION_ID
colossus --config .colossus/config.yaml plans show PLAN_ID
```

Confirm the session identity, lifecycle status, and latest revision on each record.

## Failure path

- **A plan cannot be approved:** confirm it is still a draft and that approval is
  available.
- **A revision conflict is reported:** reload the plan with `/plan use PLAN_ID`, inspect
  the new revision, and make a fresh deliberate choice.
- **Plan Mode reports a missing write:** the corrective turn was exhausted without the
  required `plan.create` or bound `plan.update`; inspect `/plans` before retrying.
- **Execution is denied:** plan approval is not blanket effect authority; inspect the
  denied action and sandbox prerequisite.
- **Execution disconnects after consumption may have begun:** do not retry
  automatically. Inspect `/plans` and the linked run or Goal evidence first.
- **A Goal remains Active after cancellation or failure:** use
  `/goal resume GOAL_ID`; a new Goal would discard the remaining-budget lineage.
- **A task or decision belongs to another session:** use the owning session rather than
  copying its identifier into unrelated work.
- **A decision is outdated:** supersede or archive it so later context is unambiguous.

## Next step

Use [Goals and subagents](goals-subagents.md) when a plan needs bounded iteration or
parallel evidence gathering.
