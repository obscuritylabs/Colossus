---
title: Configuration fields
description: Strict YAML field groups, defaults, and constraints for Colossus configuration.
audience: operator
type: reference
---

# Configuration fields

Colossus reads one strict YAML document selected by global `--config`. Global
`-w, --workspace` defaults to the current directory, is canonicalized once, and is the
base for relative config and workspace-owned paths. Unknown fields, invalid enum values,
unsafe paths, incomplete profiles, and inconsistent grants fail before runtime
construction. Field names are case-sensitive.

The current schema is exactly `2`. Schema `1` configurations are rejected rather than
silently migrated because provider connections and model profiles now have separate
authority and metadata; generate a fresh configuration with `colossus --config PATH
config init` and reapply the intended settings explicitly.

## Complete baseline

This credential-free baseline is parser-backed by the documentation contract:

<!-- rust-config-example:start -->
```yaml
schemaVersion: 2
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
storage:
  path: .colossus/state.redb
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
providers:
  profiles:
    echo:
      kind: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 120000
models:
  profiles:
    echo:
      providerProfile: echo
      model: echo
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: echo
agent:
  maxTurns: 24
subagents:
  maxConcurrent: 10
sandbox:
  backend: native
  profile: workspace-development
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 30000
  maxOutputBytes: 1048576
  maxProcesses: 16
  maxMemoryBytes: 268435456
  maxConcurrency: 1
```
<!-- rust-config-example:end -->

Environment-key deployments require independent 32-byte journal and signing values at
process launch. YAML contains names and identities, never the values.

## Top-level groups

| Field | Required | Purpose |
| --- | --- | --- |
| `schemaVersion` | Yes | Strict configuration schema identity |
| `access` | Yes | Tool selection and built-in action overrides |
| `storage` | Yes | Journal adapter, key provider, and anchor |
| `policy` | Yes | Built-in or OPA action decisions |
| `workflows` | Yes | Repository and user workflow roots |
| `providers` | No | Named provider connections; defaults to `echo` |
| `models` | No | Named model limits/capabilities and role routes; defaults to `echo` |
| `agent` | No | Agent turn bound; defaults to `24` |
| `subagents` | No | Child concurrency bound; defaults to `10` |
| `sandbox` | No | Resource obligations and platform isolation defaults |
| `context` | No | Long-session compaction controls |
| `memory` | No | Lexical and optional semantic indexes |
| `research` | No | Research bounds and compatibility search settings |
| `search` | No | Named provider-neutral search profiles and routes |
| `skills` | No | Skill roots, overrides, and disabled names |
| `packs` | No | Pack installation root |
| `mcp` | No | Exact stdio server declarations |
| `audit` | No | External evidence exporter |

## Access

| Field | Values / constraint |
| --- | --- |
| `access.profile` | `minimal`, `development`, `allow_all`, `pinned` |
| `access.tools.include` | Exact tool names; `*` allowed only as the sole wildcard selector |
| `access.tools.exclude` | Exact tool names; no wildcard |
| `access.actions.allow` | Exact action names |
| `access.actions.requireApproval` | Exact action names |
| `access.actions.deny` | Exact action names |

Include/exclude entries cannot overlap. The three action lists cannot overlap. With
`policy.kind: opa`, all action override lists are empty.

## Providers, models, and roles

| Field | Values / constraint |
| --- | --- |
| `providers.profiles.NAME.kind` | `echo`, `open_ai_responses`, `open_ai_compatible` |
| `.baseUrl` | URL including API path; remote endpoints use HTTPS |
| `.credentialReference` | `env:VARIABLE`, injected `host:IDENTIFIER`, or `null` when supported |
| `.timeoutMs` | Positive bounded duration |
| `models.profiles.NAME.providerProfile` | Existing provider connection profile |
| `.model` | Non-empty provider model identifier |
| `.contextWindowTokens` | Total model context window; at least `1024` |
| `.maxOutputTokens` | Positive output reservation smaller than the effective window |
| `.capabilities.toolCalls` | Whether tool definitions/history may be sent |
| `.capabilities.streaming` | Whether the adapter uses streaming transport |
| `models.roles.primary` | Required model profile name |
| Other role fields | Optional profile name; fall back to `primary` |

Known specialized roles are `risk_evaluator`, `context_summarizer`,
`subagent_default`, `research_planner`, `research_worker`, and
`research_synthesizer`.

The effective input budget is the context window minus the output reservation and a
safety reserve of `max(10% of the context window, 512 tokens)`. Colossus uses a
conservative byte-based estimator and compacts against this model-specific input
budget. A request may only narrow `maxOutputTokens`; it cannot enlarge the configured
limit.

With the built-in policy, each provider connection's `timeoutMs` bounds its own catalog
and generation transport independently of `sandbox.timeoutMs`. The adapter still
enforces the exact selected connection's timeout. OPA deployments may return a stricter
timeout obligation.

For `open_ai_compatible` profiles, provider-facing tool schemas omit `maxLength`
annotations to interoperate with Chat Completions servers that compile tool definitions
into bounded grammars. The canonical Colossus tool schema remains unchanged and is
validated in full before execution.

`host:` references are resolved only by an application-managed runtime through its
in-memory credential resolver. The standard CLI and daemon composition remain
environment-backed; they never interpret a `host:` identifier as a secret value.

## Storage

| Field | Values / constraint |
| --- | --- |
| `storage.path` | Local state or instance path |
| `storage.adapter` | Omitted/`redb`, or `postgres` |
| `storage.keys.kind` | `platform` or `environment` |
| Environment keys | Variable names, key identity, and separate anchor path |
| Platform keys | Service plus journal/signing key identities |
| `storage.postgres.connectionVariable` | Environment variable containing libpq URL or key/value string |
| `storage.postgres.schema` | Deployment-owned schema |
| `storage.postgres.tls.kind` | `webpki_roots`, `custom_ca`, or narrowly permitted `disabled` |
| `storage.postgres.statementTimeoutMs` | Positive query bound |

Disabled database TLS is accepted only for loopback or Unix-socket targets.

## Policy and audit

The built-in decision point accepts only:

```yaml
policy:
  kind: built_in
  require_post_effect: false
```

Remote OPA uses the complete field set below. Remote deployments require the CA and
client identity paths; acknowledgements are explicit because OPA receives bounded
logical request content after hard-secret replacement.

```yaml
policy:
  kind: opa
  base_url: https://opa.internal.example
  decision_path: /v1/data/colossus/effect
  ca_pem_path: /etc/colossus/opa-ca.pem
  identity_pem_path: /etc/colossus/opa-client.pem
  full_content_disclosure_acknowledged: true
  decision_log_masking_verified: true
  timeout_ms: 5000
```

`audit.exporter.kind` is `disabled` by default. The other exact variants are:

```yaml
audit:
  exporter:
    kind: directory
    path: /var/lib/colossus/audit-export
```

```yaml
audit:
  exporter:
    kind: worm_http
    endpoint: https://evidence.example/colossus/
    credentialReference: env:COLOSSUS_AUDIT_TOKEN
```

A WORM endpoint is credential-free, HTTPS, and ends with `/`. Its origin must appear in
`sandbox.networkDestinations`; a credential reference also requires the variable name
in `sandbox.environment`.

## Sandbox

| Field | Values / constraint |
| --- | --- |
| `backend` | `native`, `oci`, `windows_job`, `broker` |
| `profile` | `offline-default` or `workspace-development` |
| `allowBrokerFallback` | Explicit downgrade acknowledgement |
| `helperPath` | Exact helper path when configured |
| `ociRuntime` | Exact Docker, Podman, or supported client executable |
| `ociImage`, `ociProxyImage` | Preloaded immutable `@sha256:` references |
| `filesystem` | Absolute roots with `read`, `write`, `metadata`, or `execute` mode |
| `executables` | Absolute executable paths |
| `environment` | Environment variable names |
| `networkDestinations` | Canonical origins, or `*` for public HTTP(S) only |
| `timeoutMs` | Effect timeout |
| `maxOutputBytes` | Combined released-output bound |
| `maxProcesses` | Process-tree bound |
| `maxMemoryBytes` | Process-tree memory bound |
| `maxConcurrency` | Concurrent sandbox work bound |

`workspace-development` derives a write grant for the canonical selected workspace, a
trusted platform shell, Git when available, read-only system command/runtime roots, an
isolated `HOME` and temp directory, and a sanitized `PATH`. Explicit sandbox entries are
additive. `.colossus` and canonical runtime control paths are protected from shell
access; the control directory is created before sandbox obligations are derived,
including on a fresh workspace. These derived grants apply only to terminal users and
agents without workflow lineage and are rejected with OPA.

`*` matches public `http` and `https` origins only. Loopback, private, link-local, and
metadata destinations require exact canonical origins. It does not authorize raw
sockets, non-HTTP protocols, credentials, actions, or sandbox bypass. All network paths
retain proxy-only process egress, DNS pinning, TLS authority checks, no ambient proxies,
no redirects, bounded connections, and private-address rejection.

## Context, memory, and research defaults

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

Tantivy is a disposable offline lexical index. `memory.semantic.kind: chroma` adds an
optional candidate projection; canonical memory records remain in the journal.

The Chroma variant is:

```yaml
memory:
  indexEnabled: true
  indexPath: null
  retrievalLimit: 6
  semantic:
    kind: chroma
    baseUrl: https://chroma.internal.example
    tenant: colossus
    database: production
    collection: memories
    credentialReference: env:CHROMA_TOKEN
    timeoutMs: 30000
    positionPath: .colossus/chroma-position.json
    embedding:
      kind: local
      dimensions: 384
```

`embedding.kind` is `local` with `dimensions` in `64..=4096`, or
`open_ai_compatible` with `profile`, `model`, `baseUrl`, optional
`credentialReference`, `timeoutMs`, and optional `dimensions`. Every remote origin must
also be a sandbox network destination.

## Search

```yaml
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      credentialReference: null
      timeoutMs: 30000
  roles:
    agent: local
    research: local
```

Kinds are `searxng` and `serp_api`. SerpAPI requires
`credentialReference: env:VARIABLE`; SearXNG may use one and defaults `authHeader` to
`X-Searxng-Key`. Both profiles accept `userAgent` and `timeoutMs`, which default to
`colossus/0.10` and `30000`. The only route names are `agent` and `research`. Every
profile origin must be in `sandbox.networkDestinations`. Routes never silently fall
back.

## OpenRouter plus local SearXNG development example

```yaml
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
providers:
  profiles:
    openrouter:
      kind: open_ai_compatible
      baseUrl: https://openrouter.ai/api/v1
      credentialReference: env:OPENROUTER_API_KEY
      timeoutMs: 120000
models:
  profiles:
    openrouter-primary:
      providerProfile: openrouter
      model: openrouter/free
      contextWindowTokens: 131072
      maxOutputTokens: 16384
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: openrouter-primary
    risk_evaluator: openrouter-primary
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      credentialReference: null
      timeoutMs: 30000
  roles:
    agent: local
    research: local
sandbox:
  backend: native
  profile: workspace-development
  networkDestinations:
    - "*"
    - http://127.0.0.1:8888
```

The wildcard covers public OpenRouter HTTPS. The local SearXNG loopback origin remains
an exact entry. The provider credential stays outside YAML.

## Skills, packs, workflows, and MCP

```yaml
workflows:
  repository: .colossus/workflows
  user: workflows
skills:
  enabled: true
  allowUserOverrides: false
  bundled: bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
packs:
  installRoot: .colossus/packs
mcp:
  servers:
    local-docs:
      command: /absolute/path/to/mcp-server
      args: [--stdio]
      workingDirectory: /absolute/path/to/repository
      environment:
        API_TOKEN: env:MCP_API_TOKEN
      allowedTools: [search_docs]
      researchTools:
        - tool: search_docs
          title: Internal documentation
          arguments:
            query: "{query}"
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

Skill roots are paths; `disabled` accepts unique 1–128 character directory names made
from ASCII letters, digits, `.`, `_`, and `-`. Pack installation uses only
`packs.installRoot`.

MCP supports at most 64 configured servers. `command` is an exact absolute executable
also granted by the sandbox; the working directory needs a containing read or write
grant; child environment names need both an `env:VARIABLE` reference and a sandbox
environment grant. A server allows at most 1,024 unique tools and 64 research templates.
Its optional timeout and output cap may only narrow the sandbox values, and output is at
least 1,024 bytes.

## Numeric constraints

| Field | Constraint |
| --- | --- |
| `agent.maxTurns` | `1..=100`; default `24` |
| `subagents.maxConcurrent` | At least `1`; default `10` |
| `models.profiles.*.contextWindowTokens` | At least `1024` |
| `models.profiles.*.maxOutputTokens` | Positive and leaves room for the safety reserve and input |
| `context.targetPercent` | `1..99` and below `compactAtPercent` |
| `context.compactAtPercent` | `1..99` |
| `context.preserveRecentMessages` | At most `1024` |
| `memory.retrievalLimit` | `1..=100`; default `6` |
| `research.maxSources` | `1..=100`; default `20` |
| `research.maxWorkers` | `1..=16`; default `4` |
| `sandbox.timeoutMs` | Positive; backend-specific minimums may be higher |
| `sandbox.maxOutputBytes` | At least `1024` |
| `sandbox.maxProcesses`, `maxMemoryBytes`, `maxConcurrency` | Positive |
| `storage.postgres.statementTimeoutMs` | `100..=300000`; default `30000` |

## Validation commands

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
```

`config show` is the authority for parsed values. `config effective` adds the canonical
workspace, explicit and derived grants, resolved shell, protected paths, wildcard
meaning, tools, actions, sources, decisions, and unmet prerequisites without resolving
credentials.
