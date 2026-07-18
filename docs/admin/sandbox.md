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

- Canonical absolute paths for the workspace and each executable.
- Exact network origins, if network access is required.
- A supported native backend or a preloaded immutable OCI image.

## Steps

1. Start with a narrow grant:

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
      environment: []
      networkDestinations: []
      timeoutMs: 120000
      maxOutputBytes: 1048576
      maxProcesses: 16
      maxMemoryBytes: 268435456
      maxConcurrency: 1
    ```

2. Add environment variable *names*, not values.

3. Add network destinations as canonical origins such as
   `https://api.example.com`. Wildcards and URL paths are not grants.

4. Run:

    ```bash
    colossus --config .colossus/config.yaml sandbox doctor
    colossus --config .colossus/config.yaml config effective
    ```

5. Exercise each effect in a disposable repository before granting it in production.

Process execution never invokes an implicit shell. A process request names one exact
executable, literal arguments, a working directory, environment names, and resource
limits. Filesystem paths are rechecked against canonical roots; writes reject symlink
leaves and use atomic replacement.

## Backend choices

| Backend | Use |
| --- | --- |
| `native` | Host-native isolation and the normal default |
| `oci` | Preloaded Docker or Podman image pinned by full `@sha256:` digest |
| `windows_job` | AppContainer and Job Object isolation on Windows |
| `broker` | Explicitly acknowledged downgrade only |

Broker mode requires `allowBrokerFallback: true` and is not represented as sandbox
isolation. OCI uses no pull, a read-only root, dropped capabilities, bounded resources,
and exact bind mounts. Networked OCI work also requires the immutable proxy image.

## Expected result

The doctor command reports the selected backend as available and the effective catalog
shows only tools whose static obligations are met.

## Verification

Confirm that one permitted path succeeds and an undeclared sibling path fails. If
network is enabled, confirm the exact approved origin succeeds and another origin fails.
Retain the bounded diagnostic and audit evidence.

## Failure path

Treat unavailable native isolation, an invalid helper, an unpinned OCI image, resource
cleanup uncertainty, or Windows isolation setup failure as a blocked effect. Colossus
fails closed; it does not silently fall back to broker execution.

## Next step

Review [Storage and worker](storage-worker.md) before enabling concurrent clients or
scheduled work.
