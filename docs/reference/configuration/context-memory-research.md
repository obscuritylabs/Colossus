---
title: Context, memory, and research configuration
description: Configure long-session compaction, durable memory retrieval, disposable indexes, semantic backends, and bounded research.
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

For user workflows, see [Sessions and context](../../use/sessions-context.md),
[Memories](../../use/memories.md), and [Deep research](../../use/deep-research.md).

## Choose a starting point

| Scenario | Configuration guidance |
| --- | --- |
| Normal local or hosted-model use | Keep the defaults: automatic context compaction, local Tantivy memory, and bounded research |
| Deterministic or air-gapped operation | Set `context.modelAssisted: false`, keep semantic memory disabled, and use repository-only research |
| Long sessions with a dedicated summarizer | Route `context_summarizer` to a reviewed model and keep recent-message preservation explicit |
| Meaning-based memory retrieval | Add Chroma with local embeddings before introducing a second remote embedding service |
| Web-backed research | Configure the top-level `search.roles.research` route; keep legacy `research.search` disabled |
| MCP-backed research | Add explicit `mcp.servers.*.researchTools` templates; allowing an MCP tool alone is insufficient |

Omitting all three blocks selects these defaults:

```yaml
context:
  autoCompaction: true
  compactAtPercent: 70
  targetPercent: 45
  preserveRecentMessages: 8
  modelAssisted: true
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

When a block is present, use its complete strict shape. Unknown fields are rejected.

## Context configuration

Context configuration controls when older messages are replaced by an immutable summary
in the next provider request. The complete encrypted message history remains available
through session commands and audit state.

### Context fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `autoCompaction` | Create a snapshot automatically after the threshold is crossed | Boolean | `true` |
| `compactAtPercent` | Percentage of the effective model input budget that triggers compaction | `1..99` and above `targetPercent` | `70` |
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
instead of discarding them.

Setting `autoCompaction: false` disables threshold-triggered snapshots. Manual compaction
remains available, and the other context fields are still required and validated:

```yaml
context:
  autoCompaction: false
  compactAtPercent: 70
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

Memory records are durable, scoped, non-secret background context. Active decisions are
binding context and take precedence over memories; memories are explicitly presented to
the model as background rather than instructions.

### Memory fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `indexEnabled` | Enable disposable Tantivy indexing and any configured semantic index | Boolean | `true` |
| `indexPath` | Local Tantivy directory; workspace-relative, absolute, or `null` | Path or `null` | Derived beside `storage.path` |
| `retrievalLimit` | Maximum relevant canonical records composed into one model turn | `1..=100` | `6` |
| `semantic` | `kind: disabled` or a Chroma projection | Strict tagged block | Disabled |

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
  semantic:
    kind: disabled
```

Index updates are queued from canonical journal events and applied in order. Index-open,
sync, or search failure does not destroy memories; Colossus can fall back to a bounded
canonical term match and exposes index lag and errors through status.

Disable all indexes while preserving canonical memory operations with:

```yaml
memory:
  indexEnabled: false
  indexPath: null
  retrievalLimit: 6
  semantic:
    kind: disabled
```

Chroma cannot be configured when `indexEnabled` is false.

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

An external mutation with an unknown outcome blocks automatic Chroma retries. Inspect
status and use an operator-authorized rebuild to re-establish known projection state.

## Chroma semantic memory

Chroma adds a second candidate index alongside Tantivy. It does not become the memory
source of truth. The Chroma collection and its local position file are disposable
projection state that can be rebuilt from the journal.

This example uses deterministic local embeddings and sends the resulting vectors,
memory text, and bounded metadata to Chroma:

```yaml
memory:
  indexEnabled: true
  indexPath: .colossus/indexes/memory-tantivy
  retrievalLimit: 8
  semantic:
    kind: chroma
    baseUrl: https://chroma.internal.example
    tenant: colossus
    database: production
    collection: memories
    credentialReference: env:CHROMA_TOKEN
    timeoutMs: 30000
    positionPath: .colossus/indexes/chroma-position.json
    embedding:
      kind: local
      dimensions: 384
sandbox:
  networkDestinations:
    - https://chroma.internal.example
```

Merge the sandbox destination into the deployment's complete sandbox block. Chroma and
embedding credentials are resolved by Colossus in-process after authorization; they do
not need `sandbox.environment` grants.

### Chroma fields

| Field | Rule |
| --- | --- |
| `baseUrl` | Credential-free HTTPS origin; exact loopback HTTP is allowed for development |
| `tenant` | Existing Chroma tenant; 1–128 ASCII letters, digits, dots, underscores, or hyphens |
| `database` | Existing Chroma database with the same name constraint |
| `collection` | Colossus-managed disposable collection name with the same constraint |
| `credentialReference` | Optional `env:VARIABLE`; sent as `x-chroma-token` |
| `timeoutMs` | Positive per-operation timeout, capped by permit policy |
| `positionPath` | Optional local projection position/outcome file; defaults beside `storage.path` |
| `embedding` | Required local or OpenAI-compatible embedding profile |

The Chroma `baseUrl` must not contain a path other than `/`, user information, query, or
fragment. Colossus constructs the Chroma v2 API paths and gets or creates the configured
collection.

The Chroma origin must appear in `sandbox.networkDestinations`. Its client uses DNS
pinning, no ambient proxy, no redirects, bounded requests and responses, the permit
timeout, and the shared [network CA bundle](network.md).

Enabling Chroma is an external disclosure decision: memory text, metadata, IDs, and
vectors are sent to that service. The configured collection should be dedicated to the
deployment and protected accordingly.

### Local embeddings

```yaml
embedding:
  kind: local
  dimensions: 384
```

Local embeddings use deterministic token and bigram feature hashing without a model or
network request. `dimensions` must be in `64..=4096`. This provides lightweight lexical
similarity in vector form; it should not be described as model-derived semantic
understanding.

### OpenAI-compatible embeddings

Use a remote embedding endpoint when model-derived vectors are required:

```yaml
embedding:
  kind: open_ai_compatible
  profile: memory-embeddings
  model: text-embedding-model
  baseUrl: https://embeddings.example.com/v1
  credentialReference: env:EMBEDDING_API_KEY
  timeoutMs: 30000
  dimensions: 1536
```

| Field | Rule |
| --- | --- |
| `profile` | Stable 1–128 character name using ASCII letters, digits, dots, underscores, or hyphens |
| `model` | Nonempty provider model ID of at most 256 bytes |
| `baseUrl` | Credential-free HTTPS API base; a path such as `/v1` is allowed |
| `credentialReference` | Optional `env:VARIABLE`; sent as a bearer credential |
| `timeoutMs` | Positive per-request timeout, capped by permit policy |
| `dimensions` | Optional strict response length in `1..=4096`; `null` accepts any valid bounded length |

Colossus appends `/embeddings` to `baseUrl`. The embedding origin and Chroma origin must
both be present in `sandbox.networkDestinations`; list both when they differ:

```yaml
sandbox:
  networkDestinations:
    - https://chroma.internal.example
    - https://embeddings.example.com
```

The embedding service receives memory text during indexing and query text during search.
Changing the embedding model or vector dimensions changes projection compatibility;
rebuild the disposable memory indexes deliberately after the new profile is in place.

## Research configuration

Research configuration sets run-wide evidence bounds and retains one deprecated search
compatibility field. The caller selects depth and evidence lanes for each run; the model
cannot add an unrequested lane or choose a new backend.

```yaml
research:
  maxSources: 20
  maxWorkers: 4
  search:
    kind: disabled
```

### Research fields

| Field | Meaning | Constraint | Default |
| --- | --- | --- | ---: |
| `maxSources` | Maximum canonical evidence sources saved in one research run | `1..=100` | `20` |
| `maxWorkers` | Maximum query/lane collection jobs attempted in one research run | `1..=16` | `4` |
| `search` | Deprecated SearXNG compatibility adapter | `disabled` or legacy `searxng` | Disabled |

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
  search:
    kind: disabled
```

Deep three-lane research can plan 18 jobs, but the hard `maxWorkers` maximum is 16. Split
an unusually broad question or select fewer lanes instead of assuming every combination
will run.

### Evidence lanes

| Lane | Backend requirement | Data behavior |
| --- | --- | --- |
| `repo` | Readable selected workspace and normal filesystem authorization | Reads bounded repository evidence |
| `web` | Exact top-level `search.roles.research` route | Sends planned queries and saves released normalized results |
| `mcp` | At least one explicit MCP `researchTools` template | Calls the configured template for each attempted MCP query |

Every collection is an ordinary authorized effect. A denied, unavailable, failed, or
budget-skipped lane becomes a durable limitation while other released evidence can still
produce a report.

Configure web search through [Search configuration](search.md). Configure MCP evidence
through [MCP research templates](mcp.md#research-templates).

### Research model roles and fallback

Research uses the fixed `research_planner`, `research_worker`, and
`research_synthesizer` model roles for query planning, claim extraction, and final report
synthesis. Unconfigured specialized roles fall back to `primary`.

Model output is accepted only after strict phase-specific validation. If planning,
extraction, or synthesis fails or returns invalid output, Colossus records the fallback
and continues with deterministic queries, source sentences, or citation-safe report
generation. Model assistance never weakens the configured source, lane, or worker
bounds.

### Deprecated `research.search`

New configurations should leave this field disabled and use named top-level search
profiles and an explicit `research` route:

```yaml
research:
  maxSources: 20
  maxWorkers: 4
  search:
    kind: disabled
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
environment grant.

For migration compatibility only, `research.search` also accepts:

```yaml
research:
  maxSources: 20
  maxWorkers: 4
  search:
    kind: searxng
    endpoint: https://search.example.com/search
    userAgent: colossus-rust/0.6
```

The legacy endpoint requires HTTPS except for loopback HTTP, no query or fragment, a
nonempty user agent of at most 256 bytes, and an authorized network origin. It has no
credential field and inherits `sandbox.timeoutMs`. Configuring both legacy
`research.search` and any top-level search profile or role is rejected.

## Data disclosure and trust boundaries

| Feature | Potential external disclosure |
| --- | --- |
| Model-assisted context | Bounded older session messages sent to the resolved context summarizer |
| Chroma with local embeddings | Memory text, metadata, identifiers, and locally generated vectors sent to Chroma |
| Remote embeddings | Memory text and search queries sent to the embedding service, plus projection data sent to Chroma |
| Web research | Planned queries sent to the configured search service; released results persisted in the journal |
| MCP research | Templated queries sent to configured MCP tools; released results persisted as research sources |

Credentials remain references in YAML and are resolved only inside permit-bearing
adapters. Colossus-owned semantic and search clients use the shared CA bundle, exact
network authorization, DNS pinning, redirect rejection, response bounds, and quarantine.

Neither an index nor an external service may directly make a memory visible. Colossus
always rechecks canonical lifecycle and scope before composing memory context.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| Automatic compaction never happens | Confirm `autoCompaction` is true and inspect the role-specific threshold with `context status` |
| Context overflows even after compaction | Reduce preserved messages, tool surface, retrieved memory, or model output reservation |
| A different provider receives compaction text | Inspect the `context_summarizer` route and its fallback to `primary` |
| Turning off model assistance disables compaction | It does not; deterministic snapshots remain available |
| A restored snapshot appears to lose later messages | Restore changes the derived active view only; inspect canonical messages with `sessions messages` |
| Memory search returns no indexed result | Check scope, lifecycle status, and expiry with `memories index status`; canonical fallback may still return bounded matches |
| Chroma is rejected at startup | Set `indexEnabled: true`, use an origin-only base URL, and authorize its exact network origin |
| Chroma works but embedding calls are denied | Authorize the separate embedding origin and verify its credential reference |
| A Chroma retry is blocked after failure | The previous mutation outcome is unknown; inspect status and perform a deliberate rebuild |
| Vector writes fail after changing models | Keep dimensions compatible or rebuild the disposable collection and position state |
| Web research is disabled | Configure the exact top-level `search.roles.research` route |
| Search configuration reports a conflict | Do not combine top-level `search` with deprecated `research.search` |
| Research lanes are unexpectedly skipped | `maxWorkers` counts query/lane jobs; compare depth × selected lanes with the configured bound |
| Research reaches the source limit early | `maxSources` applies across every query and lane in the run |
| MCP research is disabled despite allowed tools | Add explicit `researchTools`; `allowedTools` alone does not create a research template |

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
