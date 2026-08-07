---
title: Sandbox configuration
description: Configure sandbox profiles, isolation backends, resource grants, and limits with practical examples.
audience: operator
type: reference
---

# Sandbox configuration

`sandbox` defines the isolation backend and the maximum resources an effect may use. It
does not grant permission to perform an action by itself: `access` and `policy` still
decide whether the action is allowed or requires approval.

Use this page to construct the YAML. For host preparation and verification procedures,
see the [Sandbox administration guide](../../admin/sandbox.md).

## Choose a starting point

| Scenario | Profile | Backend | Grants |
| --- | --- | --- | --- |
| Interactive work in one repository | `workspace-development` | `native` or `windows_job` | Derived workspace and shell grants, plus explicit additions |
| Automation or a durable workflow | `offline-default` or a custom label | Supported isolating backend | Explicit least-privilege grants |
| Reproducible container execution | Any nonempty profile | `oci` | Explicit mounts, image executables, and optional network origins |
| Coder/Kubernetes with a separately managed isolation boundary | `offline-default` or a custom label | `external` | Explicit acknowledgement of the external boundary |
| Intentionally unrestricted process execution | `offline-default` or a custom label | `danger_full_access` | Explicit danger acknowledgement |
| Externally brokered execution | Any profile except `workspace-development` | `broker` | Explicit acknowledgement; no Colossus process isolation |

Start with `workspace-development` for local interactive use. Use explicit grants for
workflows, shared workers, and production automation so their authority does not depend
on an interactive development preset.

## Complete default shape

This block shows every sandbox field. The platform default for `backend` is `native` on
macOS and Linux, `windows_job` on Windows, and `oci` on other supported platforms.

```yaml
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

## Profile

`sandbox.profile` is a nonempty policy identity and defaults to `offline-default` when
omitted. Two names have built-in meaning:

| Value | Behavior |
| --- | --- |
| `offline-default` | Adds no interactive workspace authority; configure needed resources explicitly |
| `workspace-development` | Derives a writable selected workspace, trusted shell, Git when available, read-only system command/runtime roots, isolated `HOME` and temporary directories, and a sanitized `PATH` |
| Any other nonempty value | Acts as a custom policy label and derives no extra grants |

The development preset applies only to terminal users and agents without workflow
lineage. It is rejected with `policy.kind: opa`, because OPA must return complete
filesystem, executable, environment, network, and limit obligations. It is also rejected
with `backend: broker`, `external`, or `danger_full_access`, which cannot enforce the
protected workspace control paths.

Explicit grants remain additive under `workspace-development`:

```yaml
sandbox:
  backend: native
  profile: workspace-development
  filesystem:
    - root: /opt/company/reference-data
      mode: read
  executables:
    - /usr/local/bin/company-linter
```

Colossus creates and protects the selected workspace's `.colossus` control directory
before deriving development grants. Shell processes cannot read or modify that directory.

## Isolation backend

### `backend`

| Value | When to use it |
| --- | --- |
| `native` | Normal macOS and Linux execution using host-native isolation |
| `windows_job` | Windows execution using AppContainer and Job Object isolation |
| `oci` | A preloaded Docker or Podman image with a read-only root and exact bind mounts |
| `external` | Supervised direct execution when Coder, Kubernetes, or another trusted host owns isolation |
| `danger_full_access` | Supervised direct execution with no asserted isolation boundary |
| `broker` | An explicitly accepted downgrade where another boundary owns execution |

### Direct-execution acknowledgements

`external` and `danger_full_access` are explicit modes; Colossus never falls back to
them when another backend is unavailable. Both retain authenticated helper execution,
environment filtering, exact executable validation, time/output bounds, resource
supervision where supported, the effect gateway, audit, policy decisions, and approval
obligations. Neither mode supplies Colossus filesystem or network isolation.

For a Coder or Kubernetes workload whose pod/container boundary is managed separately,
edit the existing sandbox block:

```yaml
sandbox:
  backend: external
  acknowledgeExternalBoundary: true
```

`acknowledgeExternalBoundary` defaults to `false`. An interactive TUI then presents the
same bottom-docked, fail-closed decision flow used for effect approvals and requires a
process-local session acknowledgement before any process permit can be minted. A headless
runtime fails process effects closed unless the field is explicitly `true`.

Use unrestricted execution only when ambient runtime access is intentional:

```yaml
sandbox:
  backend: danger_full_access
  acknowledgeDangerFullAccess: true
```

`acknowledgeDangerFullAccess` follows the same TUI/headless behavior and defaults to
`false`. Selecting either direct backend does not change approval mode and does not
auto-approve any policy obligation. The two acknowledgement fields are valid only with
their matching backend, which prevents a stale acknowledgement from silently applying
after a backend change.

In direct modes, `filesystem` and `networkDestinations` remain policy/audit declarations
and continue to constrain Colossus-owned filesystem and HTTP adapters. They are not an
OS-enforced allowlist for arbitrary child-process access; the external platform owns
that enforcement for `external`, and no such enforcement is asserted for
`danger_full_access`. Process working directories and path-like arguments therefore do
not require matching `filesystem` entries in either direct mode. Exact executable and
environment-name grants, approval decisions, time/output bounds, and audit still apply.

### `allowBrokerFallback`

Keep this `false` when Colossus must provide process isolation. Set it to `true` only
when brokered execution is an acceptable security boundary. Selecting `backend: broker`
requires `allowBrokerFallback: true`; broker mode is not represented as sandbox
isolation and cannot use `workspace-development`.

### `helperPath`

Most deployments leave `helperPath: null`, which uses the current Colossus executable as
the trusted sandbox helper. Embedded or packaged applications may provide an exact
helper executable path. Colossus canonicalizes that path before use.

### OCI-only fields

| Field | Meaning |
| --- | --- |
| `ociRuntime` | Absolute path to `docker`, `podman`, or `podman-remote` |
| `ociImage` | Preloaded workload image pinned as `REPOSITORY@sha256:DIGEST` or `sha256:DIGEST` |
| `ociProxyImage` | Preloaded immutable allowlist-proxy image; required when `networkDestinations` is nonempty |

Colossus does not pull images at execution time. Both image references must contain a
64-character SHA-256 digest. Networked OCI execution also requires at least a
10,000-millisecond timeout so proxy and container cleanup can be confirmed.

## Filesystem grants

Each `sandbox.filesystem` entry contains an absolute `root` and one access `mode`:

```yaml
sandbox:
  profile: offline-default
  filesystem:
    - root: /srv/colossus/project
      mode: write
    - root: /srv/colossus/reference-data
      mode: read
    - root: /srv/colossus/cache
      mode: metadata
```

| Mode | What it authorizes |
| --- | --- |
| `metadata` | Metadata operations under the root |
| `read` | File content reads and metadata operations under the root |
| `write` | Reads, metadata operations, and mutations under the root |
| `execute` | One exact executable identity; normally declare it with `executables` instead |

Roots must be absolute. Colossus canonicalizes existing roots, rejects symbolic-link
targets at the effect boundary, and checks the requested path against the granted root.
A write grant is intentionally broader than a read grant, so prefer separate read and
write roots when outputs can be isolated.

The workflow repository and user roots receive read access separately during runtime
composition. A filesystem grant still does not allow an action denied by access or
policy.

## Executables

`sandbox.executables` contains exact absolute executable paths—not command names and not
shell expressions:

```yaml
sandbox:
  executables:
    - /usr/bin/git
    - /usr/bin/rg
    - /usr/local/bin/company-linter
```

For native execution, each path must resolve to a regular host file. For OCI execution,
the path names the executable inside the pinned workload image. No `PATH` lookup widens
this list. Shell command mode needs either `workspace-development` or one explicitly
granted platform shell; argument-vector mode can call another exact granted executable.

## Environment variables

`sandbox.environment` contains variable names that an authorized effect may receive.
It never contains values:

```yaml
sandbox:
  environment:
    - CI
    - BUILD_MODE
    - TOOL_CACHE_DIR
```

Names use POSIX variable syntax. This list authorizes variables exposed to sandboxed
child processes and configuration fields that explicitly require a sandbox environment
grant. In-process provider credentials do not need an entry merely because their
provider profile uses `env:VARIABLE`. The owning configuration page states when both a
credential reference and an environment grant are required.

## Network destinations

Each `sandbox.networkDestinations` entry is either a canonical HTTP(S) origin or `"*"`:

```yaml
sandbox:
  networkDestinations:
    - https://api.openai.com
    - https://splunk.example.com
    - http://127.0.0.1:8888
```

An origin contains only the scheme, host, and non-default port when needed. Put endpoint
paths in the provider, search, MCP, integration, or audit configuration—not in the
sandbox grant. Entries must be unique.

`"*"` authorizes public `http` and `https` origins only:

```yaml
sandbox:
  networkDestinations:
    - "*"
    - http://127.0.0.1:8888
```

Loopback, private, link-local, and metadata destinations never match the wildcard and
must be listed exactly. The wildcard does not authorize raw sockets, non-HTTP protocols,
credentials, actions, or a sandbox bypass. Network effects retain DNS pinning, TLS
authority checks, disabled ambient proxies and redirects, bounded connections, and
private-address rejection for wildcard destinations.

## Resource limits

These fields are ceilings. A policy decision, permit, server declaration, or individual
tool request may narrow them but cannot widen them.

| Field | Values / constraint |
| --- | --- |
| `timeoutMs` | Maximum wall time for the complete effect and confirmed cleanup; must be positive |
| `maxOutputBytes` | Request/result and captured-output ceiling in bytes; at least `1024` |
| `maxProcesses` | Maximum process-tree count where the backend supports it; must be positive |
| `maxMemoryBytes` | Maximum process-tree memory in bytes where supported; must be positive |
| `maxConcurrency` | Maximum concurrent effects per actor/run; must be positive |

Minimum `timeoutMs` values are 5,000 for OCI, 10,000 for networked OCI, and 10,000 for
`windows_job`. Native execution has no additional configured minimum. Increasing
`maxConcurrency` can multiply the effective process and memory demand, so raise it only
after sizing the host and worker workload.

For a two-minute build with a 4 MiB output ceiling and up to two concurrent effects:

```yaml
sandbox:
  timeoutMs: 120000
  maxOutputBytes: 4194304
  maxProcesses: 32
  maxMemoryBytes: 1073741824
  maxConcurrency: 2
```

## Advanced examples

### Least-privilege native automation

This Linux example can read one repository, write only to a separate output directory,
run two exact tools, and contact one public API:

```yaml
sandbox:
  backend: native
  profile: offline-default
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: /srv/colossus/repository
      mode: read
    - root: /srv/colossus/output
      mode: write
  executables:
    - /usr/bin/git
    - /usr/bin/rg
  environment:
    - CI
  networkDestinations:
    - https://api.github.com
  timeoutMs: 120000
  maxOutputBytes: 4194304
  maxProcesses: 8
  maxMemoryBytes: 536870912
  maxConcurrency: 1
```

Resolve executable paths on the target host rather than copying these Linux paths to
macOS or Windows. Configure the matching action decision and credential reference
separately.

### Interactive development with public and local services

The public wildcard covers hosted model APIs. The local search service remains an exact
loopback origin:

```yaml
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
  networkDestinations:
    - "*"
    - http://127.0.0.1:8888
  timeoutMs: 120000
  maxOutputBytes: 4194304
  maxProcesses: 16
  maxMemoryBytes: 536870912
  maxConcurrency: 2
```

### Networked OCI execution

This example uses a preloaded workload and proxy image. The executable path is inside
the workload image; the filesystem root is mounted at the same absolute path:

```yaml
sandbox:
  backend: oci
  profile: offline-default
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: /usr/bin/docker
  ociImage: example.com/colossus/workload@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  ociProxyImage: example.com/colossus/proxy@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
  filesystem:
    - root: /srv/colossus/project
      mode: write
  executables:
    - /usr/bin/python3
  environment:
    - API_TOKEN
  networkDestinations:
    - https://api.example.com
  timeoutMs: 30000
  maxOutputBytes: 4194304
  maxProcesses: 16
  maxMemoryBytes: 536870912
  maxConcurrency: 1
```

OCI execution performs no image pull, uses a read-only root, drops capabilities, and
creates exact bind mounts. Replace the example image identities with digests from your
preloaded images.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| A path is rejected | Use a canonical absolute root and grant the required mode |
| A command is unavailable | Add its exact executable path; do not rely on `PATH` lookup |
| A child-process variable is unavailable | Add only its name here; keep values outside YAML |
| A remote endpoint is denied | Grant its origin without a path, query, fragment, or credentials |
| A local service is denied with `"*"` | Add the exact loopback or private origin |
| OCI configuration is rejected | Use preloaded immutable image digests and reserve the required cleanup timeout |
| A granted operation is still denied | Configure the matching `access` and `policy` decision; sandbox grants are not action permission |

## Validate the result

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml sandbox doctor
```

`config show` confirms the strict parsed values. `config effective` shows explicit and
derived grants, protected paths, resolved shell, and unmet prerequisites without
resolving credentials. `sandbox doctor` verifies whether the selected backend can
enforce the configured isolation on the current host.

Return to the [configuration overview](../configuration.md).
