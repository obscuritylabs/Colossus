---
title: Offline operation
description: Install, initialize, and verify Colossus in an offline or air-gapped environment.
audience: operator
type: how-to
---

# Offline operation

## Goal

Run Colossus without external credentials or network access and retain encrypted,
verifiable state.

## Prerequisites

- A native archive with its checksum, or a signed offline bundle and trusted publisher
  key.
- A supported platform credential service, or two independently managed 32-byte keys
  injected at launch.
- Any local model, workflows, skills, policies, or extensions required inside the
  boundary, already reviewed and transferred.

## Steps

1. Verify the transferred archive checksum before extracting. For a signed bundle, use:

    ```bash
    colossus --config .colossus/config.yaml bundle verify ./bundle
    ```

2. Install the native archive using its included `install.sh` or `install.ps1`, or
   install the running target from a verified bundle:

    ```bash
    colossus --config .colossus/config.yaml --approval-mode ask \
      bundle install ./bundle --prefix "$HOME/.local"
    ```

3. Create fresh configuration and state:

    ```bash
    colossus --config .colossus/config.yaml config init
    colossus --config .colossus/config.yaml config show
    colossus --config .colossus/config.yaml config effective
    ```

4. Keep `sandbox.networkDestinations` empty, or limit it to exact loopback origins.
   The built-in `echo` route, redb journal, built-in policy, local workflows, repository
   tools, and lexical index need no internet access.

5. Run the acceptance sequence:

    ```bash
    colossus --config .colossus/config.yaml policy doctor
    colossus --config .colossus/config.yaml state doctor
    colossus --config .colossus/config.yaml sandbox doctor
    colossus --config .colossus/config.yaml run "airgap acceptance"
    colossus --config .colossus/config.yaml audit verify
    colossus --config .colossus/config.yaml audit anchor-status
    ```

For a local OpenAI-compatible model, grant only its loopback origin and route
`providers.roles.primary` to the local profile.

## Expected result

The run completes with no external network grant, writes encrypted journal events,
creates a signed checkpoint, and verifies against the secure anchor.

## Verification

Retain the Colossus version, archive or bundle digest, config hash, effective network
destinations, run ID, audit verification, and anchor status. Independently confirm at the
host boundary that no unapproved egress occurred.

## Failure path

- A checksum proves transport integrity, not publisher identity; require the signed
  bundle when authenticity matters.
- Do not enable plaintext storage when no credential service is available. Inject
  separately managed environment keys.
- An unavailable external adapter degrades explicitly; Colossus does not discover or
  contact alternatives.
- Never blindly retry an unknown external effect after reconnecting the environment.

## Next step

Review the exact [Bundle format](../reference/bundle-format.md) and the operational
[Troubleshooting](troubleshooting.md) guide before sealing the environment.
