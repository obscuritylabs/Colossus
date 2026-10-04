---
title: Workflows and schedules
description: Run reusable workflows, inspect their logic, and schedule workflows or plain-language agent tasks in a Workspace.
audience: user
type: how-to
icon: lucide/calendar-clock
---

# Workflows and schedules

Open **Workflows** to manage reusable definitions, inspect their logic, and run them
with structured inputs. Open **Schedules** to choose when a workflow or agent task runs.
A workflow defines the work; a schedule supplies its timing.

## Import and run a workflow

1. In **Workflows**, choose **Import workflow**, paste existing declarative YAML, and
   select **Validate and review**. Check the name, version, input schema, and exact hash.
2. Select **Register workflow**. Registration does not execute the definition. Different
   content needs a new version; registration cannot replace an existing version.
3. Select the workflow and choose **View workflow logic**. The graph is available before
   creating any schedule.
4. Choose **Run workflow**. Simple input schemas provide labeled fields; **Edit JSON**
   preserves access to the full input object. Nested or complex schemas use JSON directly.
5. Review the exact definition and inputs, then choose **Start workflow**. Inspect the
   independent run and refresh **Workflow run history** to see owned executions.

![Reusable workflows and independent run history](../assets/screenshots/desktop-workflows-library.png)

Workflow resources captured through the production managed SDK and an isolated real
sidecar; surrounding Workspace navigation uses the Desktop test fixture.

History lists this application's runs for the selected definition, newest first.
Selecting a run shows its status, recorded step states, and bounded final JSON result
when available. A result larger than 64 KiB is omitted. Run allocation preserves the
reviewed definition hash and a durable retry identity. If its response is unconfirmed,
**Confirm same run request** repeats that exact identity after an explicit action.

Managed Local stores definitions and schedules in its private Workspace partition.
Separate CLI registrations are not imported automatically. See
[Your first workflow](../extend/workflows/first-workflow.md) for definition authoring.

## Schedule a task

In **Schedules**, choose **Schedule a task**, give it a name, and write instructions in
plain language. Choose **Daily** or **Weekly**, a local time, an IANA timezone, and a
first date. Weekly schedules start on the first selected weekday on or after that date.

![Plain-language task scheduling](../assets/screenshots/desktop-schedule-task.png)

**Advanced** provides a configured model profile, reasoning effort, allowed tool names,
missed-run policy, and initial enable state. The selected provider must support the
chosen effort. **Model default** preserves the profile's configured effort; Echo
requires that default. Every tool call follows current Workspace permissions and
approval rules. New task schedules default to enabled.

Select **Review task**, inspect the instructions, recurrence, first UTC occurrence,
model, tools, and policy, then choose **Create task schedule**. Each occurrence starts a
fresh agent session on the selected runtime. The worker must be running. Managed Local
runs on this computer; an external target runs on its configured host.

A task uses an internal one-step workflow. Its definition and schedule are allocated
atomically, and internal task definitions stay out of the reusable Workflows library.
Select the schedule to inspect its instructions and next occurrence.

## Schedule a reusable workflow

Choose **Schedule workflow** from a selected definition in **Workflows**, or
**Schedule a workflow** in **Schedules**. Select the registered definition, enter a
unique schedule ID, and provide its schema-based fields or JSON inputs.

Choose calendar timing or a **Fixed interval** from one minute through 31 days.
Review the exact definition hash, immutable input snapshot, UTC start, recurrence,
missed-run policy, and initial state before choosing **Create schedule**. Workflow
schedules default to paused.

Calendar recurrence preserves the selected local time across daylight saving changes.
Missing local times are skipped; repeated local times run once at the earlier instant.
A first occurrence in a missing local time is rejected so the reviewed start is explicit.
Fixed intervals measure elapsed time: every 24 hours can shift its local hour across DST.

With one due occurrence, both missed-run policies queue a run. With multiple overdue
occurrences, **Run latest once** queues the latest once; **Skip catch-up runs** queues
none and advances to the next future boundary. Calendar reconciliation is bounded to
10,000 occurrences; a larger backlog blocks and pauses the schedule for inspection.

If creation is unconfirmed, choose **Check stored schedule**. Preserve the original
reviewed request and retry key. Desktop never automatically repeats a mutation or
allocates a new key after a lost response.

## Inspect workflow logic and execution

**View workflow logic** displays the exact definition. Parallel checks separate into
lanes; conditions label True and False paths; bounded loops show their body, next-item
path, and exit. Child workflow nodes identify the referenced definition. Explicit
recovery steps appear as a separate failure path.

![Complex workflow with recorded execution](../assets/screenshots/desktop-workflow-logic.png)

This example completed through the real managed runtime. Select a node or use
**Choose a step** to inspect logic and recorded execution. Zoom, pan, and **Fit workflow**
help navigate larger graphs. Completed steps carry recorded evidence; unvisited paths
remain neutral. Loop counts summarize distinct executions. Desktop checks the definition
hash against the selected schedule or run before displaying an execution overlay.

## Control future work

Select a schedule to inspect its timing, ownership, next occurrence, last dispatch, and
last independent run. **Queued** describes dispatch rather than execution success.
**Inspect last workflow run** shows queued, running, waiting, completed, failed,
cancelled, or interrupted state.

**Review pause** stops future ticks without cancelling existing runs. **Review enable**
preserves the retained boundary and can reconcile overdue occurrences. A tick or control
change invalidates a stale review: refresh and review again. To change immutable fields,
create a replacement schedule and pause the previous one.

A blocked definition is never repinned automatically. Restore its exact definition and
dependencies, or register a new version and create a new schedule. Legacy schedules
with unknown ownership expose metadata only; Desktop cannot claim their inputs, task
instructions, run details, or control authority.

## Availability and agent requests

Managed Local ticks while its worker runs. Retained unselected workers keep ticking;
Desktop retains up to four workers. Future schedules do not pin or wake an idle
Workspace when another needs capacity. Queued, running, or waiting work counts as active
for eviction and configuration drain. Closing the main window leaves Desktop running
in the macOS menu bar or Windows system tray; shutting down Colossus stops workers.
Resume reconciles missed occurrences using the selected policy.

Agent schedule requests use the ordinary policy and approval path. Default policy
requires review for creation, including paused schedules, and enabled-state changes.
Ask and Risk auto prompt; Deny rejects approval obligations. Explicitly elevated Full
access can satisfy an approval obligation, while policy denials continue to deny.
Future occurrences undergo current workflow and effect authorization independently.

External targets need advertised resources and explicit enrolled scopes. Older runtimes
can retain fixed workflow scheduling while unavailable calendar, task, manual-run, or
history features remain disabled. See [External runtime targets](external-targets.md).
Desktop does not widen their grants.

## Next step

Use [Triggers and recovery](../extend/workflows/triggers-recovery.md) for worker operation
and reconciliation of interrupted effects.
