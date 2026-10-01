---
title: Decisions
description: Record a workspace commitment and keep it current as work changes.
audience: user
type: how-to
icon: lucide/scale
---

# Decisions

A key decision records a choice Colossus should carry into later work in the same
workspace. Use one for an architectural commitment or operating rule that should
steer future turns. A [memory](memories.md) is background context; an active decision
is presented to the model as a binding commitment. Neither changes tool permissions
or bypasses policy.

## Record a decision

In Execute Mode, state the choice and why it matters:

> Record a high-priority key decision: the journal is the source of truth and
> projections can be rebuilt. Apply it when changing storage or recovery behavior.

If `decision.create` is available and authorized, Colossus can save it. Check the
record rather than relying on the assistant's reply:

```text
/decisions
```

`/decisions` lists active decisions from the workspace, including ones recorded in
other sessions. Plan Mode does not offer `decision.create`; return to Execute Mode
with `/plan off` before asking the agent to record a decision.

For an exact CLI record, get the current session ID with `/session show` or
`colossus sessions list`, then run:

```bash
colossus decisions create SESSION_ID \
  "Storage authority" \
  "The journal is authoritative; projections can be rebuilt." \
  --priority high \
  --applies-when "Changing storage or recovery behavior" \
  --rationale "Recovery depends on canonical events"
```

Give the decision a short title and a clear statement. `--applies-when` makes its
boundary explicit; `--rationale` preserves why the choice was made. Priorities are
`normal`, `high`, and `critical` (`normal` is the default).

## Know where it applies

An active decision enters later model context across sessions in the same workspace
state. The session where it was recorded stays on the record as provenance. A
session filter on `decisions list` filters that **origin**; it does not limit where
the decision applies.

CLI/TUI and Desktop keep separate state partitions for a workspace, so a decision
recorded in one does not appear in the other. Decisions remain subject to Colossus's
normal instruction, access, approval, and sandbox boundaries.

## Inspect the current commitments

Use `/decisions` in the terminal, or inspect the exact record from a shell:

```bash
colossus decisions list
colossus decisions show DECISION_ID
```

`decisions list` shows active workspace decisions by default. Add
`--session SESSION_ID` when you only want records that originated in one session.
Inspect the decision text, rationale, applicability, priority, and status before
starting work that depends on it.

## Change a decision without leaving a conflict

For a small clarification, update the active record in place:

```bash
colossus decisions update DECISION_ID \
  --applies-when "Changing storage, projections, or recovery behavior"
```

For a changed choice, supersede the old record with a replacement:

```bash
colossus decisions supersede DECISION_ID \
  "Storage authority" \
  "The journal remains authoritative; rebuild projections after schema changes." \
  --priority high \
  --applies-when "Changing storage, projections, or recovery behavior" \
  --rationale "Clarified the migration procedure"
```

Supersession makes the old record inactive and links it to the new active one. If a
decision no longer applies and needs no replacement, archive it:

```bash
colossus decisions archive DECISION_ID
```

Do not leave contradictory active decisions for later turns to reconcile. Archived
and superseded records remain inspectable for their history.

## What's next?

<div class="grid cards" markdown>

-   :lucide-list-checks:{ .lg .middle } **Planning**

    ---

    Turn the current commitments into a reviewable plan before execution.

    [Plan the work :lucide-arrow-right:](planning.md)

-   :lucide-brain:{ .lg .middle } **Memories**

    ---

    Keep non-binding preferences and reusable facts as background context.

    [Manage memories :lucide-arrow-right:](memories.md)

</div>
