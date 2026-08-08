---
title: Configuration fields
description: Entry point for the strict Colossus YAML schema and its domain-specific field references.
audience: operator
type: reference
---

# Configuration fields

Colossus reads one strict YAML document selected by global `--config`. Global
`-w, --workspace` defaults to the current directory, is canonicalized once, and is the
base for relative configuration and workspace-owned paths. Unknown fields, invalid enum
values, unsafe paths, incomplete profiles, and inconsistent grants fail before runtime
construction. Field names are case-sensitive.

The current schema is exactly `2`. Schema `1` configurations are rejected rather than
silently migrated because provider connections and model profiles now have separate
authority and metadata. Generate a fresh configuration with `colossus --config PATH
config init` and reapply the intended settings explicitly.

Use this page for the complete baseline and top-level map. Each linked domain page owns
the exact fields, defaults, examples, and constraints for that part of the schema.
Operational procedures remain in [Configuration recipes](../admin/configuration.md).

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
  startupVerification: incremental
  keys:
    kind: none
network:
  caBundlePath: null
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
  acknowledgeExternalBoundary: false
  acknowledgeDangerFullAccess: false
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

| Field | Required | Purpose | Exact reference |
| --- | --- | --- | --- |
| `schemaVersion` | Yes | Strict configuration schema identity | This page |
| `access` | Yes | Tool selection and built-in action overrides | [Access](configuration/access.md) |
| `storage` | Yes | Journal adapter, key provider, and anchor | [Storage](configuration/storage.md) |
| `network` | No | Runtime-wide additional CA certificate bundle | [Network trust](configuration/network.md) |
| `policy` | Yes | Built-in or OPA action decisions | [Policy and audit](configuration/policy-audit.md) |
| `workflows` | Yes | Repository and user workflow roots | [Skills, packs, and workflows](configuration/extensions.md) |
| `providers` | No | Named provider connections; defaults to `echo` | [Providers and models](configuration/providers-models.md) |
| `models` | No | Named model limits, capabilities, and role routes; defaults to `echo` | [Providers and models](configuration/providers-models.md) |
| `agent` | No | Agent turn bound; defaults to `24` | [Runtime limits](configuration/limits.md) |
| `subagents` | No | Child concurrency bound; defaults to `10` | [Runtime limits](configuration/limits.md) |
| `sandbox` | No | Resource obligations and platform isolation defaults | [Sandbox](configuration/sandbox.md) |
| `context` | No | Long-session compaction controls | [Context, memory, and research](configuration/context-memory-research.md) |
| `memory` | No | Lexical and optional semantic indexes | [Context, memory, and research](configuration/context-memory-research.md) |
| `research` | No | Research bounds and compatibility search settings | [Context, memory, and research](configuration/context-memory-research.md) |
| `search` | No | Named provider-neutral search profiles and routes | [Search](configuration/search.md) |
| `skills` | No | Skill roots, overrides, and disabled names | [Skills, packs, and workflows](configuration/extensions.md) |
| `packs` | No | Pack installation root | [Skills, packs, and workflows](configuration/extensions.md) |
| `mcp` | No | Exact stdio and stateful Streamable HTTP server declarations | [MCP servers](configuration/mcp.md) |
| `audit` | No | External evidence exporter | [Policy and audit](configuration/policy-audit.md) |

## Shared rules

- Relative configuration, state, workflow, skill, pack, and workspace-owned paths
  resolve from the canonical selected workspace. Security-sensitive explicit sandbox
  roots and executables use the constraints in [Sandbox](configuration/sandbox.md).
- Credential fields store `env:VARIABLE`, injected `host:IDENTIFIER`, or `null` when the
  adapter supports unauthenticated operation. They never store a literal secret.
- Every remote origin must be authorized by the matching sandbox network destination.
  Public HTTP(S) `*` and exact private origins have different semantics; see
  [Network trust](configuration/network.md) and [Sandbox](configuration/sandbox.md).
- Numeric fields are bounded and fail closed. The consolidated ranges are in
  [Runtime limits](configuration/limits.md), with domain-specific constraints repeated
  on their owning pages.

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
