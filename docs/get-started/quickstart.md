---
title: Five-minute quickstart
description: Initialize Colossus and complete a deterministic offline agent run in less than five minutes.
audience: user
type: tutorial
---

# Five-minute quickstart

## Goal

Create a fresh configuration, run the deterministic `echo` provider, and verify the
encrypted audit journal. No model credential or network connection is required.

## Prerequisites

- The installed `colossus` executable. See [Install Colossus](install.md).
- An empty directory where Colossus may create `.colossus/config.yaml` and encrypted
  state.
- A supported platform credential store. Headless environments can use environment key
  references through the
  [headless key recipe](../admin/configuration.md#headless-environment-backed-keys).

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
colossus -w . --config .colossus/config.yaml config init
colossus -w . --config .colossus/config.yaml config show
colossus -w . --config .colossus/config.yaml config effective
```

`config init` refuses to overwrite an existing file. The generated configuration uses
the local deterministic `echo` provider, the `development` access profile, and the
`workspace-development` sandbox preset. The selected workspace is canonicalized once;
relative configuration and runtime paths resolve from it.

### 3. Run the offline smoke

```bash
colossus -w . --config .colossus/config.yaml run "hello from Colossus"
```

On an interactive terminal, Colossus renders a human response card. When output is
redirected, it emits a stable JSON result.

### 4. Verify the journal

```bash
colossus -w . --config .colossus/config.yaml audit verify
```

## Expected result

The run returns `hello from Colossus` through the `echo` profile and reports a run ID and
session ID. Audit verification completes successfully.

## Verification

Prove the machine-readable contract without changing configuration:

```bash
colossus -w . --config .colossus/config.yaml --output json \
  run "verified" > result.json
```

Open `result.json` and confirm that it contains `"profile": "echo"` and
`"output": "verified"`.

## Failure path

- **Configuration already exists:** choose another directory or inspect the existing
  file; initialization never overwrites it.
- **Credential-store error:** use the environment-key recipe for a headless host rather
  than storing raw keys in YAML.
- **Audit verification fails:** stop before running effects and follow
  [Troubleshooting](../admin/troubleshooting.md). Verification failure puts the runtime
  into read-only recovery mode.
- **JSON appears in the terminal:** automatic output selection detected a redirected
  stream; add `--output human` when a human renderer is required.
- **Development sandbox is unsupported:** initialize with
  `--sandbox-profile offline-default` for the network-free echo smoke, then review the
  platform-specific [Sandbox](../admin/sandbox.md) requirements before enabling shell
  work. Ubuntu's AppArmor user-namespace restriction may require the release archive's
  exact-path profile for a root-owned Colossus installation.

## Next step

[Connect a model provider](connect-model.md), or first read
[Core concepts](core-concepts.md) to understand the safety boundaries.
