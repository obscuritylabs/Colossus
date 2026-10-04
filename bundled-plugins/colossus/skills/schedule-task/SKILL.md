---
name: schedule-task
description: Create, inspect, pause, or enable recurring plain-language tasks and workflow schedules with exact local timing, workspace permissions, and durable confirmation.
allowed-tools: workflow.task.schedule workflow.definition.list workflow.definition.get workflow.schedule.list workflow.schedule.get workflow.schedule.create workflow.schedule.set_enabled user.ask
---
# Schedule a task

Use this skill when the user asks for recurring work: a briefing, monitor, review,
reminder, or an existing workflow run on a calendar. A workflow defines work; a
schedule chooses when it runs. Plain-language tasks use `workflow.task.schedule`.
Reusable definitions use `workflow.schedule.create` after inspecting their exact
registered hash and input schema.

## Establish the request

Identify the requested instructions, workspace/runtime, daily or weekly recurrence,
local time, IANA timezone, and first future occurrence. Confirm any genuinely missing
information with one concise question. Use stated preferences and trusted current
context; do not silently guess a timezone or invent a model profile. A UTC offset is
not an IANA timezone. Preserve wall-clock time through daylight saving changes.

Offer configured model/effort preferences only when the user requests them. Otherwise
use workspace/model defaults. Select only exposed tools needed for the work; a tool
ceiling does not grant permission. Do not add sending, publishing, filesystem writes,
or process execution merely to make a summary or briefing more useful.

Inspect owned schedules to avoid creating a duplicate of the same request. Explain
that the selected worker must be running and schedules do not wake a sleeping
computer. Each task occurrence starts a fresh independent execution.

## Allocate through the scheduling tool

Present the intended instructions and timing when clarification is needed. Honor the
normal runtime review and permission flow. This skill and its `allowed-tools` metadata
never expand tool grants, skip approval, or override a policy denial.

Call `workflow.task.schedule` with:

- a stable lowercase `schedule_id` and a unique `idempotency_key` retained for this
  exact request;
- `task`: `name`, `instructions`, explicit `tools`, and optional `options` containing
  only configured `model_profile` and `reasoning_effort`;
- `calendar`: named `timezone`, strict `HH:mm` local `time`, and ISO `weekdays`
  (Monday=1; empty means daily);
- `starts_at`: an exact first future UTC instant matching that calendar;
- `misfire_policy`: `fire_once` or `skip`, and the requested `enabled` state.

Keep the serialized task within the 48 KiB inline review bound. A larger request
must be reduced so the complete instructions and preferences can be reviewed.

Use timezone-aware evidence to calculate `starts_at`; do not approximate an offset.
Missing local times are skipped, repeated times run once at the earlier instant,
and the first occurrence must be a real matching calendar boundary. If tools cannot
resolve the next boundary safely, ask for an explicit first date/time instead of
inventing one. With multiple missed occurrences, `fire_once` queues the latest once
and `skip` queues none; a single due boundary queues under either policy.

Do not provide workflow IDs, hashes, cadence, owner, session, or run fields to
`workflow.task.schedule`; the trusted runtime derives them. If scheduling tools or
required application scopes are missing, explain the missing access and direct the
user to Schedules. Never claim a task was saved from a draft or denied call.

## Confirm and reconcile

After a successful allocation, inspect the returned ID with `workflow.schedule.get`.
Report the stored name, recurrence/timezone, next occurrence, enabled/paused state,
and model/effort if selected. Creation and dispatch do not prove execution success.

If a mutation response is lost, inspect that same `schedule_id` first. Compare the
stored instructions, task preferences, timing, and policy to the exact request.
Replay only the same request with the same retry key after an explicit reconciliation
step. Never allocate a new key to conceal an uncertain outcome. On conflict or mismatch,
report it and obtain a fresh decision before replacing the task.

Pause/enable uses `workflow.schedule.set_enabled` after fresh inspection and the exact
current `etag`. A stale revision requires another inspection and review. Pause stops
future ticks; it does not cancel queued/running work. To change immutable instructions
or timing, create a reviewed replacement and pause the previous schedule.
