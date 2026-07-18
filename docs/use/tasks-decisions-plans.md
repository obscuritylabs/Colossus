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
  "The encrypted journal is authoritative" \
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

### 4. Inspect, approve, and execute

```bash
colossus --config .colossus/config.yaml plans show PLAN_ID
colossus --config .colossus/config.yaml --approval-mode ask \
  plans approve PLAN_ID
colossus --config .colossus/config.yaml run --execute-plan PLAN_ID
```

Approval consumes the reviewed draft transition. Execution still authorizes every tool
effect independently.

## Expected result

The session exposes current work, its active commitment, and a plan with an auditable
status. An approved plan can be consumed for execution exactly through the runtime.

## Verification

```bash
colossus --config .colossus/config.yaml tasks show TASK_ID
colossus --config .colossus/config.yaml decisions show DECISION_ID
colossus --config .colossus/config.yaml plans show PLAN_ID
```

Confirm the session identity and lifecycle status on each record.

## Failure path

- **A plan cannot be approved:** confirm it is still a draft and that approval is
  available.
- **Execution is denied:** plan approval is not blanket effect authority; inspect the
  denied action and sandbox prerequisite.
- **A task or decision belongs to another session:** use the owning session rather than
  copying its identifier into unrelated work.
- **A decision is outdated:** supersede or archive it so later context is unambiguous.

## Next step

Use [Goals and subagents](goals-subagents.md) when a plan needs bounded iteration or
parallel evidence gathering.
