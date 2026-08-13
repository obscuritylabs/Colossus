---
title: Configuration recipes
description: Create, review, and safely change a strict Colossus configuration.
audience: operator
type: how-to
---

# Configuration recipes

## Goal

Create a configuration that fails closed, keeps credentials outside YAML, and makes its
effective access surface easy to review.

## Prerequisites

- An installed `colossus` binary.
- An owner-private [Colossus home](../reference/colossus-home.md).
- A repository where `.colossus/config.yaml` may intentionally replace user defaults.
- Absolute paths for every filesystem root and executable you intend to grant.

## Steps

1. Select the repository workspace and create the user-level configuration without
   overwriting any existing file:

    ```bash
    colossus -w /absolute/path/to/repository config init
    ```

   `config init` defaults `access.profile: development` to
   `sandbox.profile: workspace-development`, uses keyless plaintext journal storage,
   and writes `storage.location: home_workspace`. Override these choices explicitly:

    ```bash
    colossus -w /absolute/path/to/repository \
      config init \
      --access-profile pinned \
      --sandbox-profile offline-default \
      --storage-keys platform
    ```

   When one repository needs a complete replacement, use `config init --local` to
   create `<workspace>/.colossus/config.yaml`. Explicit, local, and user configurations
   are selected in that order and never merged.

2. Parse and render the result:

    ```bash
    colossus -w /absolute/path/to/repository config show
    ```

3. Inspect resolved tools, decisions, and unmet prerequisites:

    ```bash
    colossus -w /absolute/path/to/repository config effective
    ```

4. Add only the provider, access, and sandbox entries needed for this deployment.
   Credentials belong in an operating-system credential service or an `env:VARIABLE`
   reference. Never place a credential value in YAML.

5. Run the bounded diagnostic set after every material edit:

    ```bash
    colossus -w /absolute/path/to/repository state doctor
    colossus -w /absolute/path/to/repository policy doctor
    colossus -w /absolute/path/to/repository sandbox doctor
    ```

Common deployment shapes are:

| Shape | Provider | Access profile | Network |
| --- | --- | --- | --- |
| Offline smoke | `echo` | `minimal` or `development` | Empty |
| Interactive repository work | Configured local or hosted provider | `development` + `workspace-development` | Exact origins or reviewed public `*` |
| Reviewed catalog | Any configured provider | `pinned` | Exact required origins |
| Bounded test environment | Any configured provider | `allow_all` | Still sandboxed; wildcard remains HTTP(S)-only |

`allow_all` changes built-in action decisions. It does not create filesystem roots,
executables, origins, trusted extensions, credentials, or permits.

The `workspace-development` sandbox preset is a separate resource decision. It derives
workspace writes and a trusted non-interactive shell for users and agents outside
workflow lineage; it does not change the `development` profile's approval-required
execution decision.

## Storage protection choices

`config init --storage-keys none|platform|environment` selects the complete storage
protection tier. The default, `none`, needs no keychain or environment secrets and is
suited to disposable containers and simple jobs. It retains the append-only hash chain
and complete `audit verify`, but payload JSON and automatically selected MCP OAuth state
are plaintext. Interactive terminals show this effective posture; noninteractive logs
remain unchanged.

Protection mode is fixed when a redb path or PostgreSQL schema is initialized. Colossus
rejects opening a nonempty journal with a different mode; create a fresh path or schema
instead of attempting an in-place change.

## Headless environment-backed keys

On a headless host that requires encryption, generate the references directly:

```bash
colossus config init --storage-keys environment
```

This produces an equivalent `storage.keys` block without secret values:

```yaml
storage:
  location: home_workspace
  path: state.redb
  startupVerification: incremental
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: secure-anchor.json
```

Generate two independent 32-byte values for the process. These commands place the
generated values in process environment variables without printing them or embedding
them in shell history:

=== "macOS and Linux"

    ```bash
    export COLOSSUS_JOURNAL_KEY="$(openssl rand -hex 32)"
    export COLOSSUS_SIGNING_KEY="$(openssl rand -hex 32)"
    ```

=== "Windows PowerShell"

    ```powershell
    $journal = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($journal)
    $env:COLOSSUS_JOURNAL_KEY = [Convert]::ToHexString($journal).ToLowerInvariant()

    $signing = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($signing)
    $env:COLOSSUS_SIGNING_KEY = [Convert]::ToHexString($signing).ToLowerInvariant()
    ```

Hex and base64 are accepted, but each value must decode to exactly 32 bytes. Store the
two values in the deployment's secret manager before creating state, protect the anchor
separately, and restore all three together. Losing either key or the anchor makes the
state authority incomplete.

## Expected result

`config show` succeeds, `config effective` explains every visible and hidden capability,
and all doctor commands report a usable deployment or a specific unmet obligation.

## Verification

Run a credential-free turn before connecting an external provider:

```bash
colossus -w /absolute/path/to/repository run "configuration smoke"
colossus -w /absolute/path/to/repository audit verify
```

The run should complete through the configured role and the audit chain should verify.

## Failure path

- Unknown fields and invalid enum values are rejected; use the exact names in
  [Configuration fields](../reference/configuration.md).
- An exact tool include with a missing prerequisite is an error; inherited tools with
  missing prerequisites are hidden and explained by `config effective`.
- Relative explicit sandbox roots and executables are rejected. Workspace-owned config,
  workflow, skill, and pack paths resolve from canonical `--workspace`; storage uses
  its explicit `location`.
- A configured remote origin must also appear in `sandbox.networkDestinations`.
- A public HTTP(S) origin may match `*`; loopback/private/metadata origins require an
  exact entry.
- `config init` intentionally refuses to overwrite. Back up or choose a new home/path.

Do not weaken multiple controls at once to clear an error. Follow the first unmet
obligation reported by the diagnostics.

## Next step

Configure [provider routing](providers-routing.md), then review
[access and approvals](access-and-approvals.md).
