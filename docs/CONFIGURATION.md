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

Configurations created before unified access profiles are intentionally rejected even
though `schemaVersion` remains `1`. Migrate to a new path:

```bash
colossus --config .colossus/config.yaml config migrate \
  --output .colossus/config.migrated.yaml
```

Migration defaults to live `development` inheritance and transfers legacy action
choices. Use `--access-profile pinned` to transfer the old exact tool list instead.
Migration never overwrites its source or an existing output. Inspect the result with
`config effective` before making it active.

## Isolated Source Development

Source builds can avoid platform credential-store prompts by using the development
launcher:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

On first use, the launcher creates `.colossus/config.dev.yaml` and a mode-0600
`.colossus/dev-keys.env`. If `.colossus/config.yaml` exists, its non-storage settings
are strictly parsed and cloned. The development config always replaces the complete
storage section with a fresh environment-key identity, `.colossus/state.dev.redb`, and
`.colossus/secure-anchor.dev.json`; it never opens or imports the source state.

The equivalent initializer is:

```bash
cargo run --offline -q -p colossus-cli --bin colossus -- \
  --config .colossus/config.dev.yaml \
  config init --development --from .colossus/config.yaml
```

`--from` is accepted only with `--development`. The initializer writes key references,
not key values; the launcher generates and loads two independent 32-byte hexadecimal
keys without evaluating the key file as shell code. It compiles before loading those
keys, then executes `target/debug/colossus` directly so Cargo and dependency build
scripts never receive them. The development config, keys, state, and anchor are ignored
by Git. Do not point the development config at an existing platform-keyed or production
journal.

## Minimal Offline Configuration

The generated file uses platform-managed keys. A headless equivalent can use explicit
environment references:

<!-- rust-config-example:start -->
```yaml
schemaVersion: 1
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
    journal_key_id: journal-production-v1
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
      model: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 120000
  roles:
    primary: echo
agent:
  maxTurns: 24
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

### PostgreSQL Journal And Projections

Redb remains the default when `storage.adapter` is omitted. A multi-process deployment
can select PostgreSQL while retaining `storage.path` as its local instance and worker-IPC
identity:

```yaml
storage:
  path: .colossus/instance.redb
  adapter: postgres
  postgres:
    connectionVariable: COLOSSUS_DATABASE_URL
    schema: colossus_production
    tls:
      kind: webpki_roots
    statementTimeoutMs: 30000
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production-v1
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
```

Changing `storage.adapter` does not migrate, import, or delete existing redb state.
Provision and verify the PostgreSQL target independently, retain the original redb files
and key material, and treat any intentional data transition as an explicit reviewed
operation. Colossus never merges the two canonical journals silently.

`COLOSSUS_DATABASE_URL` may contain a libpq URL or key/value string; its value is resolved
only by the adapter and is never rendered by `config show` or `state doctor`. The pinned
Mozilla WebPKI root set is the default TLS policy. A private deployment can instead use
`kind: custom_ca` plus `caPemPath`; that PEM bundle becomes the complete database trust set.
`kind: disabled` is rejected unless every target is loopback or a Unix socket.

## Provider Profiles And Roles

Supported kinds are `echo`, `open_ai_responses`, and `open_ai_compatible`. Network
providers need a version base URL, an exact origin grant, and an access decision.
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
    risk_evaluator: openrouter
    context_summarizer: openrouter
    subagent_default: openrouter
    research_planner: openrouter
    research_worker: openrouter
    research_synthesizer: openrouter
sandbox:
  networkDestinations:
    - https://openrouter.ai
```

The `sandbox` fragment supplements the other required sandbox fields; do not replace the
whole section with that fragment. For OpenAI Responses use `open_ai_responses`,
`https://api.openai.com/v1`, and `env:OPENAI_API_KEY`. The `development` and `minimal`
profiles allow configured provider calls; `pinned` requires exact provider action
overrides.

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

## Access And Policy

The required `access` block selects the model-visible tool surface and built-in action
decisions. The `policy` block selects the decision engine and retains obligations such
as post-effect authorization:

```yaml
access:
  profile: development
  tools:
    include: []
    exclude:
      - shell.run
  actions:
    allow:
      - filesystem.write
    requireApproval:
      - context.restore
    deny:
      - integration.invoke
policy:
  kind: built_in
  require_post_effect: true
```

Tool inclusion changes visibility only; it never grants the tool's effect action.
`deny`, `requireApproval`, and `allow` must contain exact, non-overlapping action names.
Use `profile: allow_all` instead of an action wildcard. An approval decision is not a
sandbox grant: the action still needs matching roots, executables, destinations, trust,
and one-use permits. Approval modes can satisfy an approval obligation but cannot add
those authorities.

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

With OPA, the access profile still selects tools, but OPA is the sole action decision
point. `access.actions` must be empty.

Remote OPA requires HTTPS, pinned trust, mTLS, a fixed decision path, disclosure
acknowledgement, and verified decision-log masking. Diagnose it with:

```bash
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml config effective
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

Backends are `native`, `oci`, Windows AppContainer-backed `windows_job`, and explicitly
downgraded `broker`. `windows_job` supports network-free effects and exact-origin network
destinations through Colossus's authenticated proxy-only AppContainer transport. It
requires permission to create the temporary loopback exemption and dynamic WFP filters;
failure is fail-closed and never becomes broker execution. Broker mode requires
`allowBrokerFallback: true`. OCI images must be preloaded immutable `@sha256:` references
and use an exact Docker or Podman executable.
Run `sandbox doctor` before enabling process effects.

## Access Profiles And Agent Tools

New configurations default to `development`, which inherits applicable first-party
tools and configured, trusted extensions. `minimal` exposes pure support tools.
`allow_all` allows all registered trusted actions but does not bypass safety or sandbox
enforcement. `pinned` exposes only exact includes and denies actions except
`provider.echo` unless overridden.

```yaml
access:
  profile: development
  tools:
    include: []
    exclude:
      - shell.run
  actions:
    allow: []
    requireApproval: []
    deny: []

agent:
  maxTurns: 24
```

`tools.include: ["*"]` selects every applicable trusted tool without granting its
actions. An exact include with a missing static prerequisite is a configuration error;
an inherited tool with the same missing prerequisite is hidden and explained by
diagnostics. Git tools require exactly one configured executable whose filename is `git`
(or `git.exe`), while `shell.run` requires an exact executable. Inspect active and hidden
resolution with `config effective`, and active schemas with `tools list`. See
[Unified Access Profiles](ACCESS_PROFILES.md).

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

Chroma and remote embedding origins also need exact network grants and access decisions.
Canonical records remain available when an index is unavailable; use
`memories index status|sync|rebuild`.

Provider-neutral search uses named profiles plus explicit `agent` and `research` routes:

```yaml
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://127.0.0.1:8888/search
      timeoutMs: 30000
    paid:
      kind: serp_api
      endpoint: https://serpapi.com/search.json
      credentialReference: env:SERPAPI_API_KEY
      timeoutMs: 30000
  roles:
    agent: local
    research: local
```

Every profile origin must be present in `sandbox.networkDestinations`. Provider choice
never appears in model arguments, and routes do not fall back or retry. The v0.8
`research.search.kind: searxng` form remains a deprecated research-only fallback when
top-level `search` is absent; configuring both forms is rejected. See
[Provider-Neutral Web Search](SEARCH.md) for SearXNG, SerpAPI, policy, and diagnostics.
`development` and `allow_all` inherit `web.search` when the `agent` route is valid.
`pinned` requires `web.search` in `access.tools.include`; visibility still does not grant
the action.

## Workflows, Skills, Packs, And MCP

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

The command, working directory, environment names, and tool must also fit access
decisions and sandbox obligations. MCP output is quarantined and hard-secret redacted
before release.

## Audit Export

Canonical evidence remains in redb by default. An existing directory can receive
ciphertext-free evidence through the gateway:

```yaml
audit:
  exporter:
    kind: directory
    path: /absolute/path/to/audit-evidence
```

Set the intended `audit.export.write` access decision and a matching filesystem write
root.
Use `audit exporter-status`, `audit exporter-drain`, and operator-authorized
`audit exporter-reset` for durable queue management.

A retention-locked service exposing create-only object PUTs can receive the same evidence:

```yaml
audit:
  exporter:
    kind: worm_http
    endpoint: https://worm.example/v1/retained-audit/
    credentialReference: env:COLOSSUS_WORM_TOKEN
sandbox:
  environment: [COLOSSUS_WORM_TOKEN]
  networkDestinations: [https://worm.example]
```

Set the intended `audit.export.worm.write` access decision. The endpoint must end in `/`, contain no
credentials/query/fragment, and enforce retention or object lock independently. Colossus
uses deterministic content-hashed names and create-only HTTP semantics; it does not infer
WORM durability from a successful HTTP response.

## Workspace And Path Resolution

The process working directory is the workspace identity. Start Colossus from the target
repository and pass an absolute config path when config lives elsewhere. Relative paths
in YAML resolve from the process environment as documented by each adapter; security
roots and executable identities must be absolute. Changing directories never widens
policy.

After every edit, run:

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
```
