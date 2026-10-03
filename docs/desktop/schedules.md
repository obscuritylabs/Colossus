---
title: Workflow schedules
description: Import an existing workflow, review a fixed cadence, and inspect independent executions in the selected Workspace.
audience: user
type: how-to
icon: lucide/calendar-clock
---

# Schedule a workflow

Open **Schedules** in the Workspace navigation to create, inspect, pause, or enable a
registered workflow schedule. A schedule starts independent workflow runs; it does not
resume a chat or add a future user message.

## Register a workflow

Choose **Import workflow**, paste an existing declarative workflow YAML, and select
**Validate and review**. Check its name, version, input schema, and exact definition
hash before registering it. Registration does not execute the workflow. Different
content needs a new version; registration cannot silently replace an existing version.

Desktop Managed Local stores its library and schedules in its private Workspace
partition. A workflow registered against separate CLI state is not automatically
available here. The managed TUI can inspect existing state but cannot register a new
definition. For authoring help, see [Your first workflow](../extend/workflows/first-workflow.md).

## Create and review a schedule

1. Choose **Create schedule** and select a registered workflow. Its detail provides the
   canonical hash and input schema; unavailable pinned dependencies prevent scheduling.
2. Enter a unique schedule ID and a JSON input object conforming to that schema.
3. Choose a fixed elapsed cadence from one minute through 31 days and an explicit first
   occurrence. Local times that do not exist or occur twice during a clock change are
   rejected; choose UTC to identify an exact occurrence.
4. Choose the policy for multiple overdue occurrences and whether to enable future ticks
   immediately. New schedules default to paused.
5. Review the frozen UTC start, definition hash, inputs, cadence, policy, and initial
   state, then choose **Create schedule**.

Every 24 hours means elapsed time: its local hour may shift across daylight saving
changes. With **one** due occurrence, both misfire options queue a run. With **multiple**
due occurrences, **Fire once** queues the latest once; **Skip** queues none and advances
to the next future boundary.

If creation is unconfirmed, choose **Check stored schedule**. Keep the original reviewed
request and retry key when reconciling it. Desktop never automatically repeats a
mutation or allocates a new key after a lost response.

## Inspect and control future work

Select a schedule to inspect its immutable fields, input snapshot, application and chat
origin when known, next boundary, and last dispatch. **Queued** describes dispatch, not
execution success. **Inspect last workflow run** shows the independent run's current
queued, running, waiting, completed, failed, cancelled, or interrupted state.

**Review pause** stops future ticks without cancelling queued or running workflows.
**Review enable** preserves the retained boundary, so enabling an overdue schedule may
reconcile missed occurrences. A tick or another control change invalidates a stale
review: refresh and review again. To change immutable fields, create a new schedule and
pause the previous one.

A blocked definition is never repinned automatically. Restore the exact pinned
definition and dependencies, or register a new version and create a new schedule.
Legacy schedules with unknown ownership show metadata only; Desktop cannot claim their
inputs, run details, or control authority.

## Understand availability and agent requests

Managed Local ticks while its worker runs. Retained unselected workers keep ticking;
Desktop retains up to four workers. Future schedules do not pin or wake an idle
Workspace when another needs capacity. Queued, running, or waiting workflow work counts
as active work for eviction and configuration drain. Closing the main window leaves
Desktop running in the macOS menu bar or Windows system tray; shutting down Colossus
stops workers. Resume reconciles missed occurrences using the chosen policy.

Agent schedule requests use registered definitions, exact tool ceilings, and the normal
policy and approval path. Creating even a paused schedule and changing its enabled
state require review under the default policy. Ask and Risk auto prompt; Deny rejects
approval obligations. Explicitly elevated Full access can satisfy an approval obligation,
while policy denials continue to deny. Future occurrences undergo current workflow and
effect authorization independently of the original chat approval.

External targets need advertised workflow resources and explicit enrolled scopes. See
[External runtime targets](external-targets.md). Desktop does not widen their grants.

## Next step

Use [Triggers and recovery](../extend/workflows/triggers-recovery.md) for worker operation
and reconciliation of interrupted effects.
