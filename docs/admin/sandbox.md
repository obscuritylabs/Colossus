---
title: Sandbox
description: Constrain filesystem, process, environment, and network effects with explicit sandbox grants.
audience: operator
type: how-to
---

# Sandbox

## Goal

Permit only named resources and bounded effects for repository work.

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
      helperPath: null
      ociRuntime: null
      ociImage: null
      ociProxyImage: null
      filesystem: []
      executables: []
      environment: []
      networkDestinations: []
      timeoutMs: 120000
      maxOutputBytes: 1048576
      maxProcesses: 16
      maxMemoryBytes: 268435456
      maxConcurrency: 1
    ```

   Then run from any directory:

    ```bash
    colossus -w /absolute/path/to/repository \
      --config .colossus/config.yaml \
      --approval-mode ask tui
    ```

   Relative config, state, workflow, skill, pack, and tool paths resolve against the
   canonical selected workspace.

2. Keep explicit `filesystem`, `executables`, and environment variable *names* as
   additive grants. Never put environment values in YAML.

3. Add network destinations as canonical origins such as
   `https://api.example.com`. `*` means public HTTP(S) egress only:

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
| `native` | Host-native isolation and the normal default |
| `oci` | Preloaded Docker or Podman image pinned by full `@sha256:` digest |
| `windows_job` | AppContainer and Job Object isolation on Windows |
| `broker` | Explicitly acknowledged downgrade only |

Broker mode requires `allowBrokerFallback: true`, is not represented as sandbox
isolation, and cannot supply `workspace-development`. OCI uses no pull, a read-only
root, dropped capabilities, bounded resources, and exact bind mounts. Networked OCI work
also requires the immutable proxy image.

## Expected result

The doctor command reports the selected backend as available and the effective catalog
shows only tools whose static obligations are met.

## Verification

Confirm that a workspace write succeeds, an outside-workspace write fails, and
`.colossus` remains unchanged. If wildcard network is enabled, confirm a public HTTPS
origin succeeds, metadata/private origins fail, and an explicitly listed loopback origin
succeeds. Process results include a bounded `observed_origins` list for allowed proxy
connections. Retain the diagnostic and audit evidence.

## Failure path

Treat unavailable native isolation, a failed protected-path probe, an invalid helper, an
unpinned OCI image, resource cleanup uncertainty, or Windows isolation setup failure as
a blocked effect. Colossus fails closed; it does not silently fall back to broker
execution.

## Next step

Review [Storage and worker](storage-worker.md) before enabling concurrent clients or
scheduled work. Use [Sandbox configuration](../reference/configuration/sandbox.md) for
the field-by-field reference and advanced YAML examples.
