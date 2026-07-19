---
title: Goals and subagents
description: Run bounded goal iterations and delegate independent work to durable child-agent jobs.
audience: user
type: how-to
---

# Goals and subagents

## Goal

Use a bounded goal loop for iterative work and delegate an independent task without
losing policy, context, recovery, or audit controls.

## Prerequisites

- An active session ID.
- Provider routes for `primary` and, when desired, `subagent_default`.
- Tool and sandbox grants appropriate to the objective.
- A clear completion condition.

## Steps

### 1. Start a bounded goal

```bash
colossus --config .colossus/config.yaml goals run \
  "Produce a verified repository health report" \
  --session SESSION_ID --max-iterations 5
```

The maximum is a hard bound, not a target. Each iteration reuses the ordinary provider,
tool, policy, context, and journal services.

An approved plan can also execute through Goal Mode:

```bash
colossus --config .colossus/config.yaml run \
  --execute-plan PLAN_ID --goal --goal-max-iterations 5
```

### 2. Inspect goal state

```bash
colossus --config .colossus/config.yaml goals list \
  --session SESSION_ID
colossus --config .colossus/config.yaml goals show GOAL_ID
```

### 3. Queue an independent child job

```bash
colossus --config .colossus/config.yaml agents queue SESSION_ID \
  "Review the storage adapter and return file-backed findings"
colossus --config .colossus/config.yaml agents status \
  --session SESSION_ID
```

The model can request the same bounded delegation through `agent.delegate`. Recursive
delegation is denied.

### 4. Drain and inspect

```bash
colossus --config .colossus/config.yaml agents drain
colossus --config .colossus/config.yaml agents list \
  --session SESSION_ID
colossus --config .colossus/config.yaml agents show JOB_ID
```

The long-running worker can also drain queued jobs.

## Expected result

The goal reaches `complete` or a clearly recorded terminal condition within its
iteration bound. The child job has its own durable status and bounded result while
remaining attached to the parent session.

## Verification

Inspect the goal and job records after restarting Colossus. Completed results should
remain available. Queued jobs remain queued; jobs that were running when ownership was
lost become `interrupted` and are never replayed automatically.

## Failure path

- **Goal exhausts its iterations:** narrow the objective, inspect recorded progress, and
  start a new deliberate run rather than silently extending the bound.
- **Child remains queued:** run `agents drain` or check worker readiness and concurrency.
- **Child is interrupted:** inspect any unknown effects before using `agents requeue`.
- **Delegation is denied:** child scopes can only narrow the parent's visible tools and
  authority.
- **A result is too broad:** split the request into independent, bounded child tasks.

## Next step

Preserve reusable constraints with [Memories](memories.md), or gather citations with
[Deep research](deep-research.md).
