# Configuration

The Rust runtime reads one strict YAML file selected by global `--config` (default
`.colossus/config.yaml`). Unknown fields, invalid enum values, unsafe paths, incomplete
provider profiles, and inconsistent sandbox grants fail before runtime construction.

```bash
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml config show
```

`config init` refuses to overwrite. Use source control or an explicit backup before
replacing configuration; there is intentionally no force flag.

## Minimal Offline Configuration

The generated file uses platform-managed keys. A headless equivalent can use explicit
environment references:

<!-- rust-config-example:start -->
```yaml
schemaVersion: 1
storage:
  path: .colossus/state.redb
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production-v1
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
policy:
  kind: built_in
  allow_actions: []
  approval_actions: []
  require_post_effect: false
workflows:
  repository: .colossus/workflows
  user: workflows
providers:
  profiles:
    echo:
      kind: echo
      model: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 120000
  roles:
    primary: echo
agent:
  maxTurns: 24
  tools: [echo]
subagents:
  maxConcurrent: 10
sandbox:
  backend: native
  profile: offline-default
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

Journal and signing variables must contain separately managed 32-byte key material. Do
not put key values in YAML. Platform mode instead uses `kind: platform` with `service`,
`journal_key_id`, and `signing_key_id`; Keychain, DPAPI, or Secret Service stores the
material and secure anchor.

## Provider Profiles And Roles

Supported kinds are `echo`, `open_ai_responses`, and `open_ai_compatible`. Network
providers need a version base URL, an exact origin grant, and an explicit policy action.
Credential values are resolved only after a permit; configuration contains only
`env:VARIABLE` references.

OpenRouter-compatible example:

```yaml
providers:
  profiles:
    openrouter:
      kind: open_ai_compatible
      model: openrouter/free
      baseUrl: https://openrouter.ai/api/v1
      credentialReference: env:OPENROUTER_API_KEY
      timeoutMs: 120000
  roles:
    primary: openrouter
    context_summarizer: openrouter
    subagent_default: openrouter
    research_planner: openrouter
    research_worker: openrouter
    research_synthesizer: openrouter
policy:
  kind: built_in
  allow_actions:
    - provider.openai.chat
    - provider.models
  approval_actions: []
  require_post_effect: true
sandbox:
  networkDestinations:
    - https://openrouter.ai
```

The `sandbox` fragment supplements the other required sandbox fields; do not replace the
whole section with that fragment. For OpenAI Responses use `open_ai_responses`,
`https://api.openai.com/v1`, `env:OPENAI_API_KEY`, and policy action
`provider.openai.responses`.

Role resolution is visible without secrets:

```bash
colossus --config .colossus/config.yaml provider profiles
colossus --config .colossus/config.yaml models routes
colossus --config .colossus/config.yaml models route primary
colossus --config .colossus/config.yaml provider doctor openrouter
colossus --config .colossus/config.yaml provider models openrouter
```

Specialized roles fall back to `primary` when not mapped. A provider origin must match
`networkDestinations` exactly by scheme, host, and effective port; URL paths belong only
in `baseUrl`.

## Policy

Built-in policy is deny by default:

```yaml
policy:
  kind: built_in
  allow_actions:
    - filesystem.read
    - provider.openai.chat
  approval_actions:
    - filesystem.write
    - shell.run
  require_post_effect: true
```

Approval actions are not grants: an action also needs the matching resource obligation
from `sandbox`. `--approval-mode ask`, `risk-auto`, or `full-access` can satisfy an
approval obligation but cannot add roots, executables, destinations, or actions.

OPA configuration uses strict disclosure and TLS fields:

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

Remote OPA requires HTTPS, pinned trust, mTLS, a fixed decision path, disclosure
acknowledgement, and verified decision-log masking. Diagnose it with:

```bash
colossus --config .colossus/config.yaml policy doctor
```

## Sandbox And Capabilities

Filesystem roots and executables must be absolute. Network entries are canonical origins,
not wildcard URLs.

```yaml
sandbox:
  backend: native
  profile: repository-development
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: /absolute/path/to/repository
      mode: write
  executables:
    - /usr/bin/git
  environment:
    - CI
  networkDestinations:
    - https://api.openai.com
  timeoutMs: 120000
  maxOutputBytes: 1048576
  maxProcesses: 16
  maxMemoryBytes: 268435456
  maxConcurrency: 1
```

Backends are `native`, `oci`, reserved fail-closed `windows_job`, and explicitly
downgraded `broker`. Broker mode requires `allowBrokerFallback: true`. OCI images must
be preloaded immutable `@sha256:` references and use an exact Docker or Podman executable.
Run `sandbox doctor` before enabling process effects.

## Agent Tools

`agent.tools` is the exact model-visible built-in catalog. Unknown names fail startup.
The default is only `echo`; add capabilities deliberately and pair effectful tools with
policy and sandbox grants.

```yaml
agent:
  maxTurns: 24
  tools:
    - echo
    - filesystem.list
    - filesystem.read
    - filesystem.search
    - git.status
    - git.diff
    - repo.map
    - tool.search
```

Git tools require exactly one configured executable whose filename is `git` (or
`git.exe`). `shell.run` requires at least one exact executable. Inspect the resolved
model schemas with `colossus --config .colossus/config.yaml tools list`.

## Context, Memory, And Research

Omitted sections receive strict defaults:

```yaml
context:
  autoCompaction: true
  contextWindowTokens: 32768
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

Tantivy is the disposable offline lexical index. Chroma is optional and never canonical:

```yaml
memory:
  indexEnabled: true
  indexPath: .colossus/memory-index
  retrievalLimit: 8
  semantic:
    kind: chroma
    baseUrl: https://chroma.internal.example
    tenant: default_tenant
    database: default_database
    collection: colossus_memories
    credentialReference: env:CHROMA_TOKEN
    timeoutMs: 30000
    positionPath: .colossus/chroma-position.json
    embedding:
      kind: local
      dimensions: 384
```

Chroma and remote embedding origins also need exact network and policy grants. Canonical
records remain available when an index is unavailable; use `memories index status|sync|rebuild`.

SearXNG research uses an exact `/search` endpoint and `network.http` authorization:

```yaml
research:
  maxSources: 20
  maxWorkers: 4
  search:
    kind: searxng
    endpoint: https://search.internal.example/search
    userAgent: colossus-rust/0.6
```

## Workflows, Skills, Packs, And MCP

```yaml
workflows:
  repository: .colossus/workflows
  user: workflows
skills:
  enabled: true
  allowUserOverrides: false
  bundled: rust/bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
packs:
  installRoot: .colossus/packs
mcp:
  servers: {}
```

Configured MCP servers are stdio-only, exact-executable, exact-tool allowlists:

```yaml
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

The command, working directory, environment names, and tool must also fit sandbox and
policy obligations. MCP output is quarantined and hard-secret redacted before release.

## Audit Export

Canonical evidence remains in redb by default. An existing directory can receive
ciphertext-free evidence through the gateway:

```yaml
audit:
  exporter:
    kind: directory
    path: /absolute/path/to/audit-evidence
```

Grant `audit.export.write`, approval if required, and a matching filesystem write root.
Use `audit exporter-status`, `audit exporter-drain`, and operator-authorized
`audit exporter-reset` for durable queue management.

## Workspace And Path Resolution

The process working directory is the workspace identity. Start Colossus from the target
repository and pass an absolute config path when config lives elsewhere. Relative paths
in YAML resolve from the process environment as documented by each adapter; security
roots and executable identities must be absolute. Changing directories never widens
policy.

After every edit, run:

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
```
