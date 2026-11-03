---
title: Context, memory, and research configuration
description: Configure long-session compaction, durable memory retrieval, a disposable local index, and bounded research.
audience: operator
type: reference
---

# Context, memory, and research configuration

These three configuration groups affect what supporting information Colossus prepares
for model work, but they own different state and lifecycles:

| Group | Purpose | Source of truth |
| --- | --- | --- |
| `context` | Builds a bounded model-visible view of a durable session | Append-only session messages and immutable context snapshots in the journal |
| `memory` | Retrieves reusable scoped background context | Canonical memory records in the journal |
| `research` | Produces durable source-backed investigations | Research runs, released sources, claims, and reports in the journal |

Context snapshots do not delete transcript messages. Memory indexes do not own memory
lifecycle state. Research evidence is not automatically promoted into general memory.

For user workflows, see [Sessions](../../use/sessions.md) and
[Context and snapshots](../../use/sessions-context.md),
[Memories](../../use/memories.md), and [Deep research](../../use/deep-research.md).

## Choose a starting point

| Scenario | Configuration guidance |
| --- | --- |
| Normal local or hosted-model use | Keep the defaults: automatic context compaction, local Tantivy memory, and bounded research |
| Deterministic or air-gapped operation | Set `context.modelAssisted: false` and use repository-only research |
| Long sessions with a dedicated summarizer | Route `context_summarizer` to a reviewed model and keep recent-message preservation explicit |
| Web-backed research | Configure the top-level `search.roles.research` route |
| MCP-backed research | Enable the server's ordinary `allowedTools`; optionally override its research calls with `researchTools` |

Omitting all three blocks selects these defaults:

```yaml
context:
  autoCompaction: true
  compactAtPercent: 85
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: true
memory:
  indexEnabled: true
  indexPath: null
  retrievalLimit: 6
research:
  maxSources: 20
  maxWorkers: 4
```

When a block is present, you may specify only the fields you want to override. Omitted
fields keep the defaults above, while unknown fields are rejected.

## Context configuration

Context configuration controls when older messages are replaced by an immutable summary
in the next provider request. The complete encrypted message history remains available
through session commands and audit state.

### Context fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `autoCompaction` | Create a snapshot automatically after the threshold is crossed | Boolean | `true` |
| `compactAtPercent` | Percentage of the effective model input budget that triggers compaction | `1..99` and above `targetPercent` | `85` |
| `targetPercent` | Desired prepared-context size after compaction | `1..99` and below `compactAtPercent` | `45` |
| `preserveRecentMessages` | Newest canonical messages not summarized automatically | `0..=1024` | `8` |
| `modelAssisted` | Prefer a bounded summarizer-model result before deterministic fallback | Boolean | `true` |

All percentages apply to the selected model profile's effective input budget, not its
advertised context window:

```text
safety margin = max(ceil(context window / 10), 512)
input budget  = context window - max output - safety margin
threshold     = input budget × compactAtPercent / 100
target        = input budget × targetPercent / 100
```

Instructions, tool schemas, binding decisions, relevant memories, snapshots, and recent
messages all contribute to the prepared-request estimate. See
[Runtime limits configuration](limits.md#model-and-context-budgets) for a worked token
example.

### Automatic compaction

When `autoCompaction` is enabled and the original context estimate exceeds the threshold,
Colossus creates a snapshot only when there is no useful active snapshot or the active
prepared view also exceeds the threshold. It summarizes an older message range and
preserves the newest configured messages.

Compaction tries to reach `targetPercent`, but preservation and binding context take
precedence. If the newest logical turn, instructions, tool definitions, decisions, or
preserved messages cannot fit in the effective input budget, Colossus fails explicitly
instead of discarding them. This means compaction may have triggered even when the turn
ends with a preservation error: the protected tail alone was too large to summarize.
Before this check, provider-visible tool results are projected to at most 64 KiB each and
256 KiB across the complete user logical turn, including sequential assistant/tool
cycles. Built-in repository file summaries are byte-bounded as well as line-bounded so
one generated or minified preview cannot create that condition by itself.

Setting `autoCompaction: false` disables threshold-triggered snapshots. Manual compaction
remains available, and omitted context fields keep their defaults. The materialized
values are still validated together:

```yaml
context:
  autoCompaction: false
  compactAtPercent: 85
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: false
```

Use this only when an operator or application will monitor context and compact
deliberately.

### Model-assisted versus deterministic snapshots

With `modelAssisted: true`, Colossus resolves the `context_summarizer` model role. An
unconfigured specialized role follows normal model routing and falls back to `primary`.
When the resolved provider is usable, Colossus may send bounded historical messages to
that model with no tools and use a valid result as a `hybrid_model` snapshot.

If the route is offline `echo`, unavailable, too small, returns no usable final text, or
fails, Colossus creates a deterministic snapshot instead. Compaction failure never
causes canonical session messages to be deleted.

Set `modelAssisted: false` when historical messages must not be sent through a separate
summarization call or when fully deterministic snapshots are preferred:

```yaml
context:
  autoCompaction: true
  compactAtPercent: 65
  targetPercent: 40
  preserveRecentMessages: 12
  modelAssisted: false
```

If model assistance is enabled, review the provider and model selected by the
`context_summarizer` role. A dedicated route can use a smaller model or a different
trust boundary than `primary`.

### Snapshot lifecycle

Snapshots are immutable journal records with a source message range and either the
`deterministic` or `hybrid_model` strategy. One snapshot is active for future turns;
restoring an older snapshot changes that active pointer without deleting later messages
or snapshots.

Inspect and manage the lifecycle with:

```bash
colossus --config .colossus/config.yaml context status SESSION_ID --role primary
colossus --config .colossus/config.yaml context compact SESSION_ID --role primary
colossus --config .colossus/config.yaml context list SESSION_ID
colossus --config .colossus/config.yaml context restore SESSION_ID SNAPSHOT_ID
```

`context status` reports the resolved model profile, raw and prepared estimates, output
and safety reservations, input budget, threshold, target, and active snapshot. Manual
compact and restore are independently authorized state transitions.

## Memory configuration

Memory records are durable, scoped, non-secret background context. Active key decisions
apply across sessions in the same workspace as binding context and take precedence over
memories; memories are explicitly presented to the model as background rather than
instructions.

When upgrading an existing configuration, remove the entire obsolete `memory.semantic`
block, including `kind: disabled`. The strict parser rejects that removed field.
Canonical memory records and local Tantivy retrieval remain supported.
Desktop automatically removes the retired override from saved settings and setup snapshots.

### Memory fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `indexEnabled` | Enable disposable local Tantivy indexing | Boolean | `true` |
| `indexPath` | Local Tantivy directory; workspace-relative, absolute, or `null` | Path or `null` | Derived beside `storage.path` |
| `retrievalLimit` | Maximum relevant canonical records composed into one model turn | `1..=100` | `6` |

`retrievalLimit` does not change the CLI `memories search --limit` argument. It bounds
automatic memory composition during model-context preparation.

### Local Tantivy index

When `indexEnabled` is true, Colossus opens a local Tantivy lexical index. The index
stores candidate IDs and disposable search fields; after searching, Colossus reloads
each result from the canonical journal and rechecks status, expiry, session scope, and
repository scope before release.

If `indexPath` is `null`, the path is derived from the local `storage.path`. This remains
local derived state when PostgreSQL owns the canonical journal. Set an explicit path
when local state placement or volume management requires it:

```yaml
memory:
  indexEnabled: true
  indexPath: .colossus/indexes/memory-tantivy
  retrievalLimit: 8
```

Index updates are queued from canonical journal events and applied in order. Index-open,
sync, or search failure does not destroy memories; Colossus can fall back to a bounded
canonical term match and exposes index lag and errors through status.

Disable the index while preserving canonical memory operations with:

```yaml
memory:
  indexEnabled: false
  indexPath: null
  retrievalLimit: 6
```

### Index operations

```bash
colossus --config .colossus/config.yaml memories index status
colossus --config .colossus/config.yaml memories index sync
colossus --config .colossus/config.yaml memories index rebuild
```

`status` reports each consumer's readiness, journal position, lag, retry state, and
adapter status. `sync` retries queued journal-to-index work. `rebuild` resets disposable
index data and recreates it from canonical active records; it does not recreate or edit
memory records.

## Research configuration

Research configuration sets run-wide evidence bounds. The caller selects depth and
evidence lanes for each run; the model
cannot add an unrequested lane or choose a new backend.

```yaml
research:
  maxSources: 20
  maxWorkers: 4
```

### Research fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `maxSources` | Maximum canonical evidence sources saved in one research run | `1..=100` | `20` |
| `maxWorkers` | Maximum query/lane collection jobs attempted in one research run | `1..=16` | `4` |

Despite its name, `maxWorkers` is a total work-item budget in the current runtime, not a
promise of parallel execution. Each planned query combined with each selected lane is
one potential collection job.

Depth determines the planned-query ceiling:

| Depth | Maximum planned queries |
| --- | ---: |
| `quick` | 1 |
| `standard` | 3 |
| `deep` | 6 |

For example, `standard` research over `repo,web,mcp` can plan up to nine query/lane jobs.
With the default `maxWorkers: 4`, Colossus attempts the first four and records the rest
as skipped limitations. Source exhaustion can stop collection earlier.

To allow every potential lane for a standard three-lane run while retaining a 30-source
ceiling:

```yaml
research:
  maxSources: 30
  maxWorkers: 9
```

Deep three-lane research can plan 18 jobs, but the hard `maxWorkers` maximum is 16. Split
an unusually broad question or select fewer lanes instead of assuming every combination
will run.

### Evidence lanes

| Lane | Backend requirement | Data behavior |
| --- | --- | --- |
| `repo` | Readable selected workspace and normal filesystem authorization | Reads bounded repository evidence |
| `web` | Exact top-level `search.roles.research` route | Sends planned queries and saves released normalized results |
| `mcp` | Enabled MCP tools and a tool-capable research model, or explicit `researchTools` projections | Selects calls from the live allowed catalog; nonempty projections override inheritance for their server |

Every collection is an ordinary authorized effect. A denied, unavailable, failed, or
budget-skipped lane becomes a durable limitation while other released evidence can still
produce a report.

Configure web search through [Search configuration](search.md). Configure MCP evidence
through [MCP research templates](mcp.md#research-templates).

### Research model roles and fallback

Research uses the fixed `research_planner`, `research_worker`, and
`research_synthesizer` model roles for query planning, MCP call selection and claim
extraction, and final report synthesis. Unconfigured specialized roles fall back to
`primary`. Automatic MCP selection requires tool-call support on `research_worker`;
explicit projections do not need a selection model.

Model output is accepted only after strict phase-specific validation. If planning,
extraction, or synthesis fails or returns invalid output, Colossus records the fallback
and continues with deterministic queries, source sentences, or citation-safe report
generation. Model assistance never weakens the configured source, lane, or worker
bounds.

### Web search route

Use named top-level search profiles and an explicit `research` route:

```yaml
research:
  maxSources: 20
  maxWorkers: 4
search:
  profiles:
    internal:
      kind: searxng
      endpoint: https://search.example.com/search
      credentialReference: env:SEARCH_TOKEN
      authHeader: X-Searxng-Key
      userAgent: colossus/0.10
      timeoutMs: 30000
  roles:
    research: internal
sandbox:
  networkDestinations:
    - https://search.example.com
```

Top-level search supports credentials, independent profiles, and explicit `agent` and
`research` routes. Its credential is resolved in-process and does not need a sandbox
environment grant. See [Search configuration](search.md) for transport and authority
requirements.

## Data disclosure and trust boundaries

| Feature | Potential external disclosure |
| --- | --- |
| Model-assisted context | Bounded older session messages sent to the resolved context summarizer |
| Web research | Planned queries sent to the configured search service; released results persisted in the journal |
| MCP research | Templated queries sent to configured MCP tools; released results persisted as research sources |

Credentials remain references in YAML and are resolved only inside permit-bearing
adapters. Colossus-owned search clients use the shared CA bundle for HTTPS, declared or
ambient request-bound network authorization, DNS pinning, redirect
rejection, response bounds, and quarantine.

The index cannot directly make a memory visible. Colossus always rechecks canonical
lifecycle and scope before composing memory context.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| Automatic compaction never happens | Confirm `autoCompaction` is true and inspect the role-specific threshold with `context status` |
| Context overflows even after compaction | Reduce preserved messages, tool surface, retrieved memory, or model output reservation |
| A different provider receives compaction text | Inspect the `context_summarizer` route and its fallback to `primary` |
| Turning off model assistance disables compaction | It does not; deterministic snapshots remain available |
| A restored snapshot appears to lose later messages | Restore changes the derived active view only; inspect canonical messages with `sessions messages` |
| Memory search returns no indexed result | Check scope, lifecycle status, and expiry with `colossus memories show MEMORY_ID`, then inspect index lag/errors with `colossus memories index status`; canonical fallback may still return bounded matches |
| Web research is disabled | Configure the exact top-level `search.roles.research` route |
| Research lanes are unexpectedly skipped | `maxWorkers` counts query/lane jobs; compare depth × selected lanes with the configured bound |
| Research reaches the source limit early | `maxSources` applies across every query and lane in the run |
| MCP research releases no sources | Inspect the MCP lane status, allowed tool selection, credentials, and policy; automatic selection also needs a tool-capable `research_worker` model, or configure explicit projections |

## Validate the result

Parse all relationships and bounds without resolving credential values:

```bash
colossus --config .colossus/config.yaml config show
```

Then exercise the configured boundaries:

```bash
colossus --config .colossus/config.yaml models route context_summarizer
colossus --config .colossus/config.yaml context status SESSION_ID --role primary
colossus --config .colossus/config.yaml memories index status
colossus --config .colossus/config.yaml search profiles
colossus --config .colossus/config.yaml search query \
  "deployment evidence" --role research --limit 3
colossus --config .colossus/config.yaml mcp tools --server SERVER
```

Run only the commands relevant to enabled features. `config show` proves the strict
shape and numeric relationships; status and live queries prove that local indexes,
model routes, network policy, credentials, and remote services are actually usable.

Return to the [configuration overview](../configuration.md).
