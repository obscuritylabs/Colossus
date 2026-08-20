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
| Intentionally unrestricted host resource access | `offline-default` or a custom label | `danger_full_access` | Schema-default danger acknowledgement; explicit `false` blocks effects |
| Externally brokered execution | Any profile except `workspace-development` | `broker` | Explicit acknowledgement; no Colossus process isolation |

Omitting `sandbox` selects acknowledged `danger_full_access`. This intentionally unsafe
pre-1.0 default makes local work immediately usable across CLI, TUI, Desktop Managed
Local, SDK hosts, workflows, and background effects. Start with
`workspace-development`, `offline-default`, or another explicit isolating boundary when
ambient host authority is inappropriate.

## Complete default shape

This block shows every resolved sandbox field. The schema default is full access on all
platforms; platform-isolating presets explicitly choose `native` on macOS/Linux,
`windows_job` on Windows, or another supported backend.

```yaml
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
```

## Profile

`sandbox.profile` is a nonempty policy identity and defaults to `offline-default` when
omitted. Two names have built-in meaning:

| Value | Behavior |
| --- | --- |
| `offline-default` | Adds no interactive workspace authority; configure needed resources explicitly |
| `workspace-development` | Derives a writable selected workspace, trusted shell, Git when available, read-only system command/runtime roots, isolated `HOME` and temporary directories, and a sanitized `PATH` |
| Any other nonempty value | Acts as a custom policy label and derives no extra grants |

The `offline-default` profile name is not an air-gap guarantee. Desktop Managed Local's
**Offline isolated** boundary combines that resource posture with platform isolation and
hides the generic model-visible `network.http`, `web.fetch`, and `docs.fetch` tools, but
retains the configured provider's exact service and authentication/refresh destinations.
Those retained destinations do not make the generic fetch tools visible. Use the
[offline and air-gapped operation guide](../../admin/offline-airgap.md) when remote
provider transport must also be absent.

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

`external` and `danger_full_access` are direct backend values; Colossus never falls back
to them when another backend is unavailable, although the sparse schema selects
acknowledged danger mode by default. Both retain authenticated helper execution,
time/output bounds, resource supervision where supported, the effect gateway, audit,
policy decisions, and approval obligations. `external` also retains exact executable
and environment-name validation. `danger_full_access` deliberately drops those process
allowlists and inherits the runtime environment after a process permit is minted.
Neither mode supplies Colossus filesystem or network isolation.

On Unix, direct-mode timeout and output bounds cover the supervised request and attached
process group. Process-count, memory, whole-tree termination, and cleanup are
best-effort for descendants that deliberately escape with `setsid`, double-forking, or
reparenting. Such a descendant may outlive the effect and its later activity is outside
that effect's audit record. Strict containment requires native or OCI isolation, a
Windows Job Object, or an external host boundary that owns the complete process
namespace/job.

For a Coder or Kubernetes workload whose pod/container boundary is managed separately,
edit the existing sandbox block:

```yaml
sandbox:
  backend: external
  acknowledgeExternalBoundary: true
```

`acknowledgeExternalBoundary` defaults to `false`. An interactive TUI then presents the
same bottom-docked, fail-closed decision flow used for effect approvals and requires a
session acknowledgement before any process permit can be minted. Embedded mode keeps that
acknowledgement process-local. Worker-backed mode issues an opaque capability to the attached
TUI client and accepts it only for that session's interactive operations; ordinary worker API
calls and other clients remain blocked. A headless runtime fails process effects closed unless
the field is explicitly `true`.

Use unrestricted execution only when ambient runtime access is intentional:

```yaml
sandbox:
  backend: danger_full_access
  acknowledgeDangerFullAccess: true
```

`acknowledgeDangerFullAccess` defaults to `true` only when the effective backend is the
default `danger_full_access`. A partial block that explicitly selects an isolating
backend contextually defaults the acknowledgement to `false`; an explicitly stale
acknowledgement is rejected. Selecting a backend does not itself change approval mode,
though the separate default `access.profile: allow_all` allows registered actions.

In `external`, configured filesystem and network declarations continue to constrain
Colossus-owned adapters while the external platform owns child-process containment.
Acknowledged `danger_full_access` instead supplies explicit ambient resource authority
to every eligible effect. Structured filesystem, repository, patch, trace, process,
and related tools may use absolute host paths and relative paths that traverse outside
`-w`, including `.git`, `.colossus`, live state, configuration, and credential files.
Structured network tools may reach any canonical HTTP(S) origin, including loopback,
private, link-local, and cloud-metadata destinations, without duplicate
`networkDestinations` entries. It resolves executables from absolute paths or ambient
`PATH`, permits outside working directories, inherits ambient environment variables,
and leaves child-process networking unrestricted. Internal helper-control variables
are never inherited.

Ambient authority does not invent a capability. Provider/model routing, credential
references, configured MCP servers and `allowedTools`, connected integration schemas,
pack signatures and trust, known action identities, strict request validation,
authenticated one-use permits, durable audit, quarantine and post-effect release,
transport validation, and configured resource bounds remain mandatory. Configured `*`
retains its public-only meaning; ambient authority is represented separately.

Enabled pack tools and pack-declared stdio MCP servers are rejected under
`danger_full_access`. Direct ambient execution cannot enforce their manifest resource
and credential ceilings; select an isolating boundary instead.

For HTTPS, certificate and hostname validation still apply. Ambient authority also
accepts canonical plaintext HTTP outside loopback. That transport provides no TLS
confidentiality or server authentication and can expose request bodies and credentials;
select an isolating boundary when non-loopback HTTP must remain invalid.

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
The acknowledged `danger_full_access` backend is the intentional exception: `shell.run`
uses an ambient platform shell, resolves command names on the runtime `PATH`, and does
not require `sandbox.executables` entries.

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

An acknowledged `danger_full_access` process instead inherits the runtime environment
after authorization and may override names through `shell.run.env` without listing them
here. Colossus keeps its private helper-control variables out of that inherited map.

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

`"*"` matches public HTTP(S) origins, but under a declared or isolating boundary the
permit-bearing adapters still require HTTPS outside exact loopback development:

```yaml
sandbox:
  networkDestinations:
    - "*"
    - http://127.0.0.1:8888
```

Loopback, private, link-local, and metadata destinations never match the wildcard and
must be listed exactly. An exact declaration authorizes the destination, but does not
weaken the transport: non-loopback plaintext HTTP still requires acknowledged ambient
authority. The wildcard does not authorize raw sockets, non-HTTP protocols, credentials,
actions, or a sandbox bypass. Network effects retain DNS pinning, TLS authority checks
for HTTPS, disabled ambient proxies and redirects, bounded connections, and
private-address rejection for wildcard destinations.

These destinations constrain Colossus-owned HTTP adapters under configured and external
boundaries. Acknowledged `danger_full_access` instead binds each requested canonical
HTTP(S) origin into its one-use permit, including non-public origins. Raw child-process
networking is unrestricted in that mode. Adding the list does not narrow ambient
authority; choose an isolating backend before treating it as an allowlist.

## Resource limits

These fields are ceilings. A policy decision, permit, server declaration, or individual
tool request may narrow them but cannot widen them.

| Field | Values / constraint |
| --- | --- |
| `timeoutMs` | Maximum wall time for the supervised effect and attached-group cleanup; isolating backends confirm whole-tree cleanup; must be positive |
| `maxOutputBytes` | Request/result and captured-output ceiling in bytes; at least `1024` |
| `maxProcesses` | Maximum process-tree count where the backend supports it; must be positive |
| `maxMemoryBytes` | Maximum process-tree memory in bytes where supported; must be positive |
| `maxConcurrency` | Maximum concurrent effects per actor/run; must be positive |

Minimum `timeoutMs` values are 5,000 for OCI, 10,000 for networked OCI, and 10,000 for
`windows_job`. Native execution has no additional configured minimum. Increasing
`maxConcurrency` can multiply the effective process and memory demand, so raise it only
after sizing the host and worker workload.

The default output ceiling is 4 MiB (`4194304` bytes). Provider streaming counts the
complete raw SSE body against this ceiling, including protocol framing, reasoning and
usage events, tool-call arguments, and visible output. This byte bound is independent
of model `maxOutputTokens`; policies and provider-specific declarations may narrow it.

The default memory ceiling is 1 GiB (`1073741824` bytes) and is not reserved when
Colossus starts. Native supervision measures observed process-tree resident memory, OCI
uses the effective value as its container memory cap, and Windows applies process and
job limits. Explicit configuration or external policy can retain stricter deployment
values.

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
| A path is rejected under an isolating boundary | Use a canonical absolute root and grant the required mode |
| A command is unavailable | Add its exact executable path; only acknowledged `danger_full_access` relies on ambient `PATH` |
| A child-process variable is unavailable | Add only its name here; acknowledged `danger_full_access` instead inherits ambient names and accepts explicit overrides |
| A remote endpoint is denied under an isolating boundary | Grant its origin without a path, query, fragment, or credentials |
| A local service is denied with `"*"` | Add the exact loopback origin; for a private non-loopback service, use HTTPS and add its exact origin |
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
