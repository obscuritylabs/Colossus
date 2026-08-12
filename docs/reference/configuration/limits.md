---
title: Runtime limits configuration
description: Understand and tune agent turns, concurrency, token budgets, context thresholds, and resource ceilings.
audience: operator
type: reference
---

# Runtime limits configuration

Colossus uses several independent limits rather than one global runtime budget. The
`agent` and `subagents` blocks configure model-loop and child-scheduler bounds. Model,
context, memory, research, sandbox, MCP, and storage limits live with their owning
configuration blocks.

The distinction matters:

| Limit type | What it controls | Example |
| --- | --- | --- |
| Termination bound | Number of model rounds before a run must stop | `agent.maxTurns` |
| Reservation | Space held for generated model output | `maxOutputTokens` |
| Threshold | When context compaction starts and where it aims | `compactAtPercent`, `targetPercent` |
| Scheduler concurrency | Number of jobs that may run together | `subagents.maxConcurrent` |
| Job-count bound | Total collection jobs attempted in one operation | `research.maxWorkers` |
| Effect ceiling | Wall time, bytes, processes, memory, or parallel effects | `sandbox.*` |
| Adapter timeout | One provider, MCP, search, or database operation | `timeoutMs`, `statementTimeoutMs` |

Limits are not permission grants. A run still needs tool visibility, policy
authorization, sandbox grants, credentials, and approvals. A policy decision or
adapter-specific declaration may narrow a configured ceiling but cannot use the
ceiling to authorize an otherwise denied effect.

## Choose a starting point

| Scenario | Starting guidance |
| --- | --- |
| General interactive use | Keep the defaults until observed runs show a specific bound is too tight |
| Predictable automation or CI | Lower `maxTurns` and child concurrency so failure and cost are tightly bounded |
| Long-running worker | Size subagent/effect concurrency and research job counts against provider quotas and host capacity |
| Long conversations | Declare the model's real context window first, then tune compaction percentages |
| Large tool output | Raise the relevant adapter cap only after raising and reviewing the sandbox byte ceiling |
| Slow database or tool | Change that operation's timeout; do not assume a larger agent turn count adds time |

The generated configuration omits the agent and child-scheduler blocks so defaults can
evolve without pinning old generated values. Omitting either block currently selects
`agent.maxTurns: 100` and `subagents.maxConcurrent: 10`.

`config show` states both limits even when the file omits them, so the resolved turn and
concurrency bounds stay inspectable.

Add the blocks only to override those defaults. For example, a small automation worker
might use:

```yaml
agent:
  maxTurns: 12
subagents:
  maxConcurrent: 2
```

These values are operational choices, not universal recommendations. Model latency,
provider quotas, workload shape, approval flow, and host resources should determine the
final settings.

## Agent turns

### `agent.maxTurns`

| Property | Value |
| --- | --- |
| Default | `100` |
| Valid range | `1..=100` |
| Scope | Model/provider rounds in one agent run |
| Exhaustion result | Terminal `agent.max_turns` error and durable `run.max_turns.v1` event |

A turn is one trip through the model loop. A model may return a final response before
the limit. When it requests tools, Colossus validates and executes authorized calls,
then another model continuation consumes another turn. Argument-repair and required
Plan Mode recovery can also consume turns.

`maxTurns` is a stopping bound, not a target. It does not directly cap:

- Tool calls that may appear in a model response.
- Generated tokens in each response.
- The wall-clock duration of a provider request or tool effect.
- Child-agent or sandbox concurrency, or the number of research collection jobs.

The CLI may explicitly override the configured default for one run:

```bash
colossus --config .colossus/config.yaml run --max-turns 8 \
  "Inspect the failure and return a bounded diagnosis"
```

The override must still be in `1..=100`; it can be higher or lower than the configured
value. Durable child-agent jobs use the configured `agent.maxTurns`, not the parent's
one-off CLI override.

The create-run API reserves zero as a transport sentinel for the configured positive
default, and Desktop uses that sentinel while its override field is blank. Zero is not
an unlimited mode and is not valid for `agent.maxTurns` in YAML.

Increasing this value can multiply provider usage because each turn may make another
generation request. Raise it when runs genuinely need more model/tool continuations,
not to compensate for an unrelated timeout or context-window problem.

## Child-agent concurrency

### `subagents.maxConcurrent`

| Property | Value |
| --- | --- |
| Default | `10` |
| Valid range | At least `1` |
| Scope | Durable child-agent jobs executing concurrently in one runtime |

Queued child jobs remain durable. When Colossus drains the queue, it starts at most
`maxConcurrent` jobs in a batch and waits for them to reach a terminal state before
starting more. Interrupted child work is not replayed automatically.

Each child is a complete bounded agent run: it selects the configured child model role,
uses `agent.maxTurns`, and remains subject to ordinary tools, policy, sandbox, context,
and audit controls. Recursive child delegation is denied.

This field is independent from:

| Field | Separate scope |
| --- | --- |
| `research.maxWorkers` | Query/lane collection jobs attempted inside one research run |
| `sandbox.maxConcurrency` | Concurrent effects for one actor/run |
| `sandbox.maxProcesses` | Process-tree size for one supported sandbox effect |
| Provider service limits | External request rate, concurrent-request, and token quotas |

A runtime with four concurrent children can still let each child issue effects up to
its own permitted sandbox concurrency. Tune the combined envelope, not each number in
isolation.

Use the queue status to observe the configured maximum, active jobs, and available
slots:

```bash
colossus --config .colossus/config.yaml agents status
```

See [Goals and subagents](../../use/goals-subagents.md) for queue, drain, cancellation,
and recovery workflows.

## Model and context budgets

Model limits are declared per profile. They describe the selected model; Colossus does
not discover or guess them from the provider:

| Field | Constraint |
| --- | --- |
| `models.profiles.*.contextWindowTokens` | At least `1024` |
| `models.profiles.*.maxOutputTokens` | Positive and small enough to leave a positive input budget |

Colossus reserves output and a safety margin before calculating model-visible input:

```text
safety margin = max(ceil(context window / 10), 512)
input budget  = context window - max output - safety margin
```

For a 128,000-token context window with 16,000 output tokens, the safety margin is
12,800 and the effective input budget is 99,200 tokens. A request may lower the output
limit, but it cannot raise it above the configured model maximum.

Context compaction percentages apply to that derived input budget, not to the provider's
advertised context window:

```yaml
context:
  autoCompaction: true
  compactAtPercent: 70
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: true
```

With the 99,200-token input budget above, automatic compaction begins around 69,440
estimated tokens and aims for about 44,640. Colossus uses conservative byte-based token
estimation, so these are planning thresholds rather than provider billing measurements.

| Context field | Constraint | Default |
| --- | --- | ---: |
| `targetPercent` | `1..99` and lower than `compactAtPercent` | `45` |
| `compactAtPercent` | `1..99` | `70` |
| `preserveRecentMessages` | `0..=1024` messages | `8` |

Increasing `maxOutputTokens` reduces the available input budget. Increasing
`compactAtPercent` delays compaction but does not enlarge the model window. A large
`preserveRecentMessages` value can also make compaction less effective because those
messages are never summarized automatically.

See [Provider and model configuration](providers-models.md#model-profiles) for complete
profiles and [Context, memory, and research configuration](context-memory-research.md)
for compaction behavior.

## Memory and research bounds

These limits bound how much supporting material one operation may collect or compose:

```yaml
memory:
  indexEnabled: true
  indexPath: null
  retrievalLimit: 6
  semantic:
    kind: disabled
research:
  maxSources: 20
  maxWorkers: 4
  search:
    kind: disabled
```

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `memory.retrievalLimit` | Maximum memories composed into one model turn | `1..=100` | `6` |
| `research.maxSources` | Maximum canonical evidence sources in one research run | `1..=100` | `20` |
| `research.maxWorkers` | Maximum query/lane collection jobs attempted in one research run | `1..=16` | `4` |

`maxSources` does not guarantee that many usable sources will be found. `maxWorkers`
caps total query/lane collection work, not the child-agent scheduler, and can increase
search traffic and pressure on external services.

## Sandbox resource ceilings

Sandbox limits bound individual effects and their process trees. These are the default
values; see [Sandbox configuration](sandbox.md#resource-limits) for complete backend
examples.

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `sandbox.timeoutMs` | Complete effect wall time, including confirmed cleanup | Positive, with backend minimums | `30000` |
| `sandbox.maxOutputBytes` | Request, result, and captured-output ceiling in bytes | At least `1024` | `1048576` |
| `sandbox.maxProcesses` | Process-tree count where supported | Positive | `16` |
| `sandbox.maxMemoryBytes` | Process-tree memory in bytes where supported | Positive | `268435456` |
| `sandbox.maxConcurrency` | Concurrent effects per actor/run | Positive | `1` |

Backend-specific minimum timeouts are:

| Backend case | Minimum `timeoutMs` |
| --- | ---: |
| Native | No additional configured minimum |
| OCI without network | `5000` |
| OCI with network destinations | `10000` |
| Windows Job Object | `10000` |

A policy permit, MCP declaration, pack declaration, or individual request may impose a
smaller timeout or output cap. It cannot widen the sandbox ceiling. For example, an MCP
server's `maxOutputBytes` must be at least 1,024 bytes and no greater than
`sandbox.maxOutputBytes`.

Output limits use bytes, while model limits use tokens. Increasing one does not change
the other. Raising `maxConcurrency` can multiply the effective process count, memory
demand, network traffic, and output volume, so size it together with host and worker
capacity.

## Timeouts are local to an operation

There is no single configuration field that sets a deadline for an entire agent run.
Common timeout fields apply at different boundaries:

| Field | Scope | Default / constraint |
| --- | --- | --- |
| `providers.profiles.*.timeoutMs` | One provider catalog or generation request | Optional positive override; defaults to `300000` remotely and `900000` on loopback |
| `sandbox.timeoutMs` | One permit-bearing effect and cleanup | `30000`; backend minimums apply |
| `mcp.servers.*.timeoutMs` | One MCP operation | When present, positive and no greater than sandbox timeout |
| Search or semantic-memory `timeoutMs` | One adapter request | Positive; see the owning page |
| `storage.postgres.statementTimeoutMs` | One PostgreSQL statement and lock acquisition | `30000`; `100..=300000` |

`agent.maxTurns` counts model rounds and is not a timeout. A run can contain provider
requests, approvals, tool effects, and context work with separate time bounds. If one
operation is timing out, adjust that operation only after confirming the service and
cleanup behavior are healthy.

See [MCP server configuration](mcp.md#selection-and-bounds),
[Search configuration](search.md), and [Storage configuration](storage.md#postgresql-fields)
for adapter-specific limits.

## Consolidated numeric reference

| Field | Valid value | Default |
| --- | --- | ---: |
| `agent.maxTurns` | `1..=100` | `100` |
| `subagents.maxConcurrent` | At least `1` | `10` |
| `models.profiles.*.contextWindowTokens` | At least `1024` | Profile-specific |
| `models.profiles.*.maxOutputTokens` | Positive with remaining input budget | Profile-specific |
| `context.targetPercent` | `1..99`, below compaction threshold | `45` |
| `context.compactAtPercent` | `1..99` | `70` |
| `context.preserveRecentMessages` | `0..=1024` | `8` |
| `memory.retrievalLimit` | `1..=100` | `6` |
| `research.maxSources` | `1..=100` | `20` |
| `research.maxWorkers` | `1..=16` | `4` |
| `sandbox.timeoutMs` | Positive, plus backend minimum | `30000` |
| `sandbox.maxOutputBytes` | At least `1024` | `1048576` |
| `sandbox.maxProcesses` | Positive | `16` |
| `sandbox.maxMemoryBytes` | Positive | `268435456` |
| `sandbox.maxConcurrency` | Positive | `1` |
| `storage.postgres.statementTimeoutMs` | `100..=300000` | `30000` |

Adapter protocols also impose non-configurable size, pagination, recursion, and retry
bounds. Raising a value in this table does not remove those hard limits.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| A run stops with `agent.max_turns` | The model used every turn; narrow the task or deliberately raise the turn bound |
| A run still times out after increasing `maxTurns` | Change the provider or effect timeout that is actually expiring |
| Provider/model configuration is rejected | `maxOutputTokens` plus the safety margin must leave a positive input budget |
| Compaction happens earlier than expected | Percentages apply to the derived input budget, not the advertised context window |
| Context still cannot fit after compaction | Reduce preserved messages, retrieved material, tool output, or output reservation |
| Child work remains queued | Check `agents status`, worker readiness, and `subagents.maxConcurrent` |
| Increasing child concurrency does not increase tool parallelism | `sandbox.maxConcurrency` is a separate per-actor/run effect ceiling |
| Research creates too much external traffic | Lower `research.maxWorkers`; it is independent from child concurrency |
| MCP configuration exceeds policy | Its timeout and byte cap may only narrow the sandbox values |
| A sandbox timeout is rejected at startup | OCI, networked OCI, and Windows Job Object require cleanup-safe minimums |
| Output is truncated despite a larger token budget | Tool output is byte-bounded separately from model generation tokens |
| Setting a field to zero fails | Numeric limits are not feature toggles; use the owning feature's enable/disable setting |

## Validate the result

Parse the complete configuration and print its resolved defaults without resolving
credential values:

```bash
colossus --config .colossus/config.yaml config show
```

Then exercise only the affected boundary:

```bash
colossus --config .colossus/config.yaml models profiles
colossus --config .colossus/config.yaml models doctor MODEL_PROFILE
colossus --config .colossus/config.yaml agents status
colossus --config .colossus/config.yaml sandbox doctor
colossus --config .colossus/config.yaml state doctor
```

`config show` proves that relationships and numeric ranges are accepted. Doctor and
status commands test the provider, scheduler, sandbox, or storage boundary that will
actually enforce the value. A syntactically valid high limit does not prove that the
provider quota, database, or host can sustain it.

For fixed protocol and UI caps that are not part of YAML, see
[Output, environment, and limits](../output-environment-limits.md#important-hard-bounds).

Return to the [configuration overview](../configuration.md).
