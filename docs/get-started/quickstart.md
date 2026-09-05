---
title: Five-minute quickstart
description: Initialize Colossus and complete a deterministic offline agent run in less than five minutes.
audience: user
type: tutorial
---

# Five-minute quickstart

## Goal

Create a fresh configuration, run the deterministic `echo` provider, and verify the
hash-chained audit journal. No storage key, model credential, or network connection is
required.

## Prerequisites

- The installed `colossus` executable. See [Install Colossus](install.md).
- A fresh Colossus home with no global `config.yaml` yet.
- An empty directory to use as the repository workspace.

## Steps

### 1. Create a working directory

=== "macOS and Linux"

    ```bash
    mkdir colossus-quickstart
    cd colossus-quickstart
    ```

=== "Windows PowerShell"

    ```powershell
    New-Item -ItemType Directory colossus-quickstart
    Set-Location colossus-quickstart
    ```

### 2. Initialize strict configuration

```bash
colossus -w . config init
colossus -w . config show
colossus -w . config effective
```

`config init` creates `$COLOSSUS_HOME/config.yaml` and refuses to overwrite an existing
file. The ordinary generated document is intentionally small:

```yaml
schemaVersion: 3
storage:
  location: home_workspace
  path: state.redb
```

Omitted fields use the strict compiled defaults shown by `config show`: the local
deterministic `echo` provider, `access.profile: allow_all`, and acknowledged
`sandbox.backend: danger_full_access`. This is an intentionally unsafe pre-1.0
developer default. Authorized process, filesystem, and HTTP tools can use ambient host
resources outside the selected workspace without approval. Provider routes,
credentials, configured extensions, one-use permits, audit, transport checks, and
configured bounds still apply. On Unix, however, a deliberately detached direct child
can evade process-tree memory/count accounting, outlive the effect, and act outside its
audit record; timeout and output bounds cover the supervised effect. Agent Plugin scripts
and MCP servers remain governed by ordinary tools, explicit MCP enablement, permits,
configured limits, and audit. Select an explicit isolating sandbox preset when ambient
authority or best-effort process cleanup is inappropriate.

The selected workspace is canonicalized once and its state resolves beneath
`workspaces/<partition-id>/cli/`.

Use `config init --local` when this repository needs a complete replacement at
`<workspace>/.colossus/config.yaml`. Configuration files are selected, not merged; see
[Colossus home and workspace resolution](../reference/colossus-home.md).

The default `storage.keys.kind: none` keeps setup dependency-free. Journal payloads
are plaintext, while record hashes, the append-only chain, projections, and full audit
verification remain active. Interactive commands show security posture warnings; the
danger-full-access warning also appears on stderr for noninteractive commands while
JSON stdout stays clean. To start a fresh protected journal instead, pass
`--storage-keys platform` or `--storage-keys environment` during initialization.

### 3. Run the offline smoke

```bash
colossus -w . run "hello from Colossus"
```

On an interactive terminal, Colossus prints only the assistant response. When output is
redirected, it emits the complete stable JSON result.

### 4. Verify the journal

```bash
colossus -w . audit verify
```

## Expected result

The run prints `hello from Colossus`. The JSON verification below exposes its profile,
run ID, and session ID. Audit verification completes successfully.

## Verification

Prove the machine-readable contract without changing configuration:

```bash
colossus -w . --output json \
  run "verified" > result.json
```

Open `result.json` and confirm that it contains `"profile": "echo"` and
`"output": "verified"`.

## Failure path

- **Configuration already exists:** inspect the global file and use it, choose an
  explicit unused `COLOSSUS_HOME`, or intentionally initialize a repository-local
  replacement with `config init --local`; initialization never overwrites.
- **Protected-storage credential error:** create a fresh environment-key configuration
  for a headless host rather than storing raw keys in YAML.
- **Audit verification fails:** stop before running effects and follow
  [Troubleshooting](../admin/troubleshooting.md). Verification failure puts the runtime
  into read-only recovery mode.
- **JSON appears in the terminal:** automatic output selection detected a redirected
  stream; add `--output human` when a human renderer is required.
- **Full host access is inappropriate:** initialize with an explicit
  `--sandbox-profile workspace-development` or `--sandbox-profile offline-default`
  preset, then review the platform-specific [Sandbox](../admin/sandbox.md)
  requirements. Ubuntu's AppArmor user-namespace restriction may require the release
  archive's exact-path profile for a root-owned Colossus installation.

## Next step

[Connect a model provider](connect-model.md), or first read
[Core concepts](core-concepts.md) to understand the safety boundaries.
