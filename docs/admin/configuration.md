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
- A repository where `.colossus/` may hold local configuration and state.
- Absolute paths for every filesystem root and executable you intend to grant.

## Steps

1. Select the repository workspace and create configuration without overwriting any
   existing file:

    ```bash
    colossus -w /absolute/path/to/repository \
      --config .colossus/config.yaml config init
    ```

   `config init` defaults `access.profile: development` to
   `sandbox.profile: workspace-development`. Override either choice explicitly:

    ```bash
    colossus -w /absolute/path/to/repository \
      --config .colossus/config.yaml config init \
      --access-profile pinned \
      --sandbox-profile offline-default
    ```

2. Parse and render the result:

    ```bash
    colossus --config .colossus/config.yaml config show
    ```

3. Inspect resolved tools, decisions, and unmet prerequisites:

    ```bash
    colossus --config .colossus/config.yaml config effective
    ```

4. Add only the provider, access, and sandbox entries needed for this deployment.
   Credentials belong in an operating-system credential service or an `env:VARIABLE`
   reference. Never place a credential value in YAML.

5. Run the bounded diagnostic set after every material edit:

    ```bash
    colossus --config .colossus/config.yaml state doctor
    colossus --config .colossus/config.yaml policy doctor
    colossus --config .colossus/config.yaml sandbox doctor
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

## Headless environment-backed keys

On a headless host without Keychain, DPAPI, or Secret Service, replace the generated
`storage.keys` block with explicit environment references:

```yaml
storage:
  path: .colossus/state.redb
  keys:
    kind: environment
    journal_variable: COLOSSUS_JOURNAL_KEY
    journal_key_id: journal-production
    signing_variable: COLOSSUS_SIGNING_KEY
    anchor_path: .colossus/secure-anchor.json
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
colossus --config .colossus/config.yaml run "configuration smoke"
colossus --config .colossus/config.yaml audit verify
```

The run should complete through the configured role and the audit chain should verify.

## Failure path

- Unknown fields and invalid enum values are rejected; use the exact names in
  [Configuration fields](../reference/configuration.md).
- An exact tool include with a missing prerequisite is an error; inherited tools with
  missing prerequisites are hidden and explained by `config effective`.
- Relative explicit sandbox roots and executables are rejected; workspace-owned config,
  state, workflow, skill, and pack paths resolve from the canonical `--workspace`.
- A configured remote origin must also appear in `sandbox.networkDestinations`.
- A public HTTP(S) origin may match `*`; loopback/private/metadata origins require an
  exact entry.
- `config init` intentionally refuses to overwrite. Back up or choose a new path.

Do not weaken multiple controls at once to clear an error. Follow the first unmet
obligation reported by the diagnostics.

## Next step

Configure [provider routing](providers-routing.md), then review
[access and approvals](access-and-approvals.md).
