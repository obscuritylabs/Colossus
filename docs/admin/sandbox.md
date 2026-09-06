---
title: Sandbox
description: Constrain filesystem, process, environment, and network effects with explicit sandbox grants.
audience: operator
type: how-to
---

# Sandbox

## Goal

Select and verify the intended execution boundary for repository work. Sparse
schema-version-3 configuration intentionally defaults to acknowledged
`danger_full_access`; use this guide to opt down to named resources and platform
isolation whenever ambient host authority is inappropriate.

## Prerequisites

- A canonical workspace selected with global `-w, --workspace`.
- Canonical absolute paths for explicit roots and executables.
- Exact private origins, if private or loopback network access is required.
- A supported native backend or a preloaded immutable OCI image.
- On Ubuntu 24.04 or later with restricted unprivileged user namespaces, the
  exact-path AppArmor profile from the Linux release archive.

## Steps

1. For interactive repository development, select the workspace and use the development
   preset:

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
      timeoutMs: 120000
      maxOutputBytes: 4194304
      maxProcesses: 16
      maxMemoryBytes: 1073741824
      maxConcurrency: 1
    ```

   These are the compiled 4 MiB output and 1 GiB process-tree memory defaults. Provider
   raw SSE bytes are counted separately from model output tokens, and process memory is
   observed or capped by the selected backend rather than preallocated at startup.
   Set explicit lower values when a deployment requires stricter ceilings.

   Then run from any directory:

    ```bash
    colossus -w /absolute/path/to/repository \
      --config .colossus/config.yaml \
      --approval-mode ask tui
    ```

   Relative config, state, workflow, standalone MCP, and tool paths resolve against the
   canonical selected workspace.

2. Keep explicit `filesystem`, `executables`, and environment variable *names* as
   additive grants. Never put environment values in YAML.

3. Add network destinations as canonical origins such as
   `https://api.example.com`. `*` matches public HTTP(S) destinations; declared
   execution still requires HTTPS outside exact loopback development:

    ```yaml
    sandbox:
      networkDestinations:
        - "*"
        - http://127.0.0.1:8888
    ```

   Loopback, private, link-local, and metadata destinations never match `*`; list each
   required origin exactly. The wildcard does not grant raw TCP/UDP, SSH, other
   protocols, credentials, actions, or a sandbox bypass.

4. Run:

    ```bash
    colossus --config .colossus/config.yaml sandbox doctor
    colossus --config .colossus/config.yaml config effective
    ```

   On affected Ubuntu hosts, `protected_path_exclusions_supported: false` and an
   AppArmor message mean the host admitted the user namespace but denied the mount
   capability needed to mask `.colossus`. Install Colossus at a root-owned path and
   load the archive's exact-path profile:

    ```bash
    sudo ./install.sh --prefix /usr/local
    sudo ./install-apparmor.sh /usr/local/bin/colossus
    /usr/local/bin/colossus -w /path/to/repository sandbox doctor
    ```

   The installer rejects symlinks, user-replaceable binaries, user-replaceable parent
   directories, and AppArmor metacharacters. The profile grants `userns` only to that
   exact executable; Colossus still drops and locks namespace capabilities before
   starting the requested command. Do not attach the profile to a user-local
   installation. Use OCI when a root-owned installation is not available.

5. Exercise each effect in a disposable repository before granting it in production.

`shell.run` accepts exactly one of `command` or `argv`. `command` runs a bounded
non-interactive script through the resolved platform shell without startup profiles;
`argv` retains exact execution semantics. Both use a workspace-relative `cwd`, isolated
`HOME`/temp directory, sanitized absolute `PATH`, bounded environment, timeout, process
tree, and output. Persistent PTYs, background sessions, and interactive stdin are not
provided.

The development grant applies only to terminal users, main agents, and child agents
without workflow lineage. The selected workspace is writable, but `.colossus` control
state is created and hidden from the shell before the first command can run. macOS uses
Seatbelt deny rules, Linux masks the path in a rootless mount namespace before Landlock,
Windows uses protected AppContainer ACL targets, and OCI masks it with a read-only
inaccessible mount. Runtime construction fails when the selected backend cannot enforce
the exclusion.

## Backend choices

| Backend | Use |
| --- | --- |
| `native` | Host-native isolation selected explicitly or by an isolating preset |
| `oci` | Preloaded Docker or Podman image pinned by full `@sha256:` digest |
| `windows_job` | AppContainer and Job Object isolation on Windows |
| `external` | Direct execution inside an operator-asserted Coder/Kubernetes/host boundary |
| `danger_full_access` | Ambient process plus structured filesystem and HTTP(S) authority with no asserted isolation boundary; selected and acknowledged by the sparse schema default |
| `broker` | Explicitly acknowledged downgrade only |

Broker mode requires `allowBrokerFallback: true`, is not represented as sandbox
isolation, and cannot supply `workspace-development`. OCI uses no pull, a read-only
root, dropped capabilities, bounded resources, and exact bind mounts. Networked OCI work
also requires the immutable proxy image.

`external` is the intended choice when the Colossus process is already contained by a
trusted platform boundary and native kernel sandboxing is unavailable, as can happen in
Coder workspaces running in Kubernetes:

```yaml
sandbox:
  backend: external
  acknowledgeExternalBoundary: false
```

With the default `false`, the TUI shows a warning and requires a session-scoped
acknowledgement. Set it to `true` only in operator-managed configuration when the same
external boundary is guaranteed for headless runs. Use `danger_full_access` plus
`acknowledgeDangerFullAccess` only when ambient access is intentional. Neither direct
mode disables normal policy decisions or approval obligations, and neither may use
`workspace-development` because Colossus cannot hide `.colossus` from the child process.

Direct modes keep authenticated helper execution and resource/output supervision, but
they do not enforce child-process filesystem or network allowlists. With `external`,
configured structured-adapter grants still apply while the platform boundary owns
filesystem and network enforcement. With acknowledged `danger_full_access`, no
resource grants are required: structured path tools may use host paths outside the
workspace, structured HTTP may reach public or non-public HTTP(S) origins, executables
resolve from absolute paths or ambient `PATH`, the child inherits the runtime
environment, working directories may be outside the workspace, and child networking is
unrestricted. No isolation boundary is asserted, but configured capability identity,
policy, configured effect bounds, permits, quarantine, transport validation, and audit
remain active.
HTTPS still validates certificates and hostnames. Canonical non-loopback plaintext HTTP
is also accepted in this mode and has no TLS confidentiality or server authentication.

On Unix direct backends, the configured timeout and output ceiling bind the supervised
request and attached process group. Process-count, memory, whole-tree termination, and
cleanup are best-effort when hostile code deliberately escapes that group with
`setsid`, double-forking, or reparenting. Such a descendant can outlive the reported
effect, and its later activity is outside that effect's audit evidence. Use native or
OCI containment, a Windows Job Object, or an `external` host boundary that contains the
entire process namespace/job when strict descendant cleanup and resource enforcement
are required.

`danger_full_access` cannot provide Colossus-owned isolation for scripts referenced by
Agent Skills or plugin stdio MCP servers. Select an isolating boundary when those
components require enforced process containment; plugin metadata never widens authority.

## Expected result

The doctor command reports the selected backend as available and the effective catalog
shows only tools whose static obligations are met.

## Verification

For an isolating boundary, confirm that a workspace write succeeds, an
outside-workspace write fails, and `.colossus` remains unchanged. If wildcard network
is enabled, confirm a public HTTPS origin succeeds, metadata/private origins fail, and
an explicitly listed loopback origin succeeds. For full access, run only
non-destructive probes and confirm `config effective` reports ambient filesystem,
network, process, and environment authority. Do not interpret a direct-backend process
result as proof that deliberately detached Unix descendants terminated. Process results
include a bounded `observed_origins` list for allowed proxy connections. Retain the
diagnostic and audit evidence.

## Failure path

Treat unavailable native isolation, a failed protected-path probe, an invalid helper,
an unpinned OCI image, unexpected cleanup uncertainty under an isolating boundary, or
Windows isolation setup failure as a blocked effect. Colossus fails closed; it does not
silently fall back to broker or direct execution. Deliberately select and acknowledge
`external`, or retain the acknowledged `danger_full_access` default, only when that
behavior is intended.

## Next step

Review [Storage and worker](storage-worker.md) before enabling concurrent clients or
scheduled work. Use [Sandbox configuration](../reference/configuration/sandbox.md) for
the field-by-field reference and advanced YAML examples.
