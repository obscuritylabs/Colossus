---
title: Configuration fields
description: Entry point for the strict Colossus YAML schema and its domain-specific field references.
audience: operator
type: reference
---

# Configuration fields

Colossus reads one strict YAML document. Selection order is explicit `--config PATH`,
`<workspace>/.colossus/config.yaml`, then `$COLOSSUS_HOME/config.yaml`; the documents
are complete replacements and are never merged. A missing explicit path or malformed
higher-priority file fails without fallback. Global `-w, --workspace` defaults to the
current directory, is canonicalized once, and selects repository context, relative-path
anchoring, and state identity—not the Colossus home. Under full access it is not a
maximum resource boundary. Unknown fields, invalid enum values, unsafe paths, incomplete
profiles, and inconsistent grants fail before runtime construction. Field names are
case-sensitive.

The current schema is exactly `2`. Schema `1` configurations are rejected rather than
silently migrated because provider connections and model profiles now have separate
authority and metadata. Generate a fresh configuration with `colossus config init`,
`colossus config init --local`, or `colossus --config PATH config init` as appropriate,
then reapply the intended settings explicitly. See
[Colossus home and workspace resolution](colossus-home.md) for exact selection and
initialization behavior.

Use this page for the complete baseline and top-level map. Each linked domain page owns
the exact fields, defaults, examples, and constraints for that part of the schema.
Operational procedures remain in [Configuration recipes](../admin/configuration.md).

## Minimal authored configuration

Ordinary `config init` writes only the choices that cannot be inferred safely:

```yaml
schemaVersion: 2
storage:
  location: home_workspace
  path: state.redb
```

Only `schemaVersion` and `storage` are required at the root. Ordinary nested blocks are
recursively defaultable, so a user can specify one changed nested field without copying
the rest of the schema. Explicit tagged variants still require `kind`, and unknown
fields remain errors. `config show` expands the complete resolved configuration below;
`config effective` adds resolution and authority metadata.

## Complete resolved baseline

This credential-free baseline is parser-backed by the documentation contract:

<!-- rust-config-example:start -->
```yaml
schemaVersion: 2
access:
  profile: allow_all
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
storage:
  location: home_workspace
  path: state.redb
  adapter: redb
  startupVerification: incremental
  keys:
    kind: none
network:
  caBundlePath: null
audit:
  exporter:
    kind: disabled
observability:
  enabled: false
  serviceName: colossus
  resourceAttributes: {}
  traces:
    enabled: false
    sampleRatio: 1.0
  metrics:
    enabled: false
    exportIntervalMs: 60000
  logs:
    otlp: false
    stdoutJson: false
    journalPayloads: disabled
    acknowledgeSensitiveContent: false
  otlp:
    endpoint: null
    protocol: grpc
    timeoutMs: 10000
    acknowledgeInsecureTransport: false
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
search:
  profiles: {}
  roles: {}
mcp:
  oauthCredentialStore: auto
  servers: {}
skills:
  enabled: true
  allowUserOverrides: false
  bundled: bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
packs:
  installRoot: .colossus/packs
sandbox:
  backend: danger_full_access
  profile: offline-default
  allowBrokerFallback: false
  acknowledgeExternalBoundary: false
  acknowledgeDangerFullAccess: true
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 30000
  maxOutputBytes: 4194304
  maxProcesses: 16
  maxMemoryBytes: 1073741824
  maxConcurrency: 1
agent:
  maxTurns: 100
subagents:
  maxConcurrent: 10
```
<!-- rust-config-example:end -->

Environment-key deployments require independent 32-byte journal and signing values at
process launch. YAML contains names and identities, never the values.

## Top-level groups

| Field | Required | Purpose | Exact reference |
| --- | --- | --- | --- |
| `schemaVersion` | Yes | Strict configuration schema identity | This page |
| `access` | No | Tool selection and built-in action overrides; defaults to `allow_all` | [Access](configuration/access.md) |
| `storage` | Yes | Journal adapter, key provider, and anchor | [Storage](configuration/storage.md) |
| `network` | No | Runtime-wide additional CA certificate bundle | [Network trust](configuration/network.md) |
| `policy` | No | Built-in or OPA action decisions; defaults to built-in | [Policy and audit](configuration/policy-audit.md) |
| `workflows` | No | Repository and user workflow roots | [Skills, packs, and workflows](configuration/extensions.md) |
| `providers` | No | Named provider connections; defaults to `echo` | [Providers and models](configuration/providers-models.md) |
| `models` | No | Named model limits, capabilities, and role routes; defaults to `echo` | [Providers and models](configuration/providers-models.md) |
| `agent` | No | Agent turn bound; defaults to `100` | [Runtime limits](configuration/limits.md) |
| `subagents` | No | Child concurrency bound; defaults to `10` | [Runtime limits](configuration/limits.md) |
| `sandbox` | No | Execution boundary and resource obligations; omission selects acknowledged full access | [Sandbox](configuration/sandbox.md) |
| `context` | No | Long-session compaction controls | [Context, memory, and research](configuration/context-memory-research.md) |
| `memory` | No | Lexical and optional semantic indexes | [Context, memory, and research](configuration/context-memory-research.md) |
| `research` | No | Research bounds and compatibility search settings | [Context, memory, and research](configuration/context-memory-research.md) |
| `search` | No | Named provider-neutral search profiles and routes | [Search](configuration/search.md) |
| `skills` | No | Skill roots, overrides, and disabled names | [Skills, packs, and workflows](configuration/extensions.md) |
| `packs` | No | Pack installation root | [Skills, packs, and workflows](configuration/extensions.md) |
| `mcp` | No | Exact stdio and stateful Streamable HTTP server declarations | [MCP servers](configuration/mcp.md) |
| `audit` | No | External evidence exporter | [Policy and audit](configuration/policy-audit.md) |
| `observability` | No | Opt-in OTLP traces, metrics, logs, and journal log disclosure | [Live observability](configuration/observability.md) |

## Shared rules

- Relative explicit configuration paths and workspace-owned workflow, skill, pack, and
  configured sandbox paths resolve from the canonical selected workspace. Relative storage paths
  use `storage.location`; `home_workspace` confines them to the current CLI workspace
  partition. See [Storage](configuration/storage.md). Security-sensitive explicit
  sandbox roots and executables use the constraints in
  [Sandbox](configuration/sandbox.md).
- Credential fields store `env:VARIABLE`, injected `host:IDENTIFIER`, or `null` when the
  adapter supports unauthenticated operation. They never store a literal secret.
- Under a configured resource boundary, every remote origin must be authorized by the
  matching sandbox network destination. Under acknowledged danger full access, each
  canonical requested HTTP(S) origin is authorized as an exact request-bound resource,
  including loopback, private, link-local, and metadata origins. See
  [Network trust](configuration/network.md) and [Sandbox](configuration/sandbox.md).
- Numeric fields are bounded and fail closed. The consolidated ranges are in
  [Runtime limits](configuration/limits.md), with domain-specific constraints repeated
  on their owning pages.

## Validation commands

```bash
colossus -w /absolute/path/to/repository config show
colossus -w /absolute/path/to/repository config effective
colossus -w /absolute/path/to/repository state doctor
colossus -w /absolute/path/to/repository policy doctor
colossus -w /absolute/path/to/repository sandbox doctor
```

`config show` is the authority for parsed values. `config effective` adds the canonical
workspace, `resolution.configSource`, `resolution.configScope`, the resolved home,
workspace partition ID, resolved state path, explicit and derived grants, resolved
shell, protected paths, resource authority mode and matrix, wildcard meaning, tools,
actions, sources, decisions, and unmet prerequisites without
resolving credentials or private bootstrap material.
