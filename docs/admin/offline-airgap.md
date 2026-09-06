---
title: Offline operation
description: Install, initialize, and verify Colossus in an offline or air-gapped environment.
audience: operator
type: how-to
---

# Offline operation

## Goal

Run Colossus without external credentials or network access and retain verifiable
state, optionally with protected storage.

## Prerequisites

- A native archive with its checksum, or a signed offline bundle and trusted publisher
  key.
- For protected storage only: a supported platform credential service, or two
  independently managed 32-byte keys injected at launch.
- Any local model, workflows, Agent Plugin OCI layouts, policies, or extensions required inside the
  boundary, already reviewed and transferred.

## Steps

1. Verify the transferred archive checksum before extracting.

2. Install the native archive using its included `install.sh` or `install.ps1`.

3. Create fresh configuration and state:

    ```bash
    colossus -w . config init \
      --sandbox-profile offline-default
    colossus -w . config show
    colossus -w . config effective
    ```

4. When upgrading an existing installation from a signed offline bundle, verify and
   install it only after the configuration above exists:

    ```bash
    colossus -w . bundle verify ./bundle
    colossus -w . --approval-mode ask \
      bundle install ./bundle --prefix "$HOME/.local"
    ```

5. Keep `sandbox.networkDestinations` empty, or limit it to exact loopback origins.
   Never use `*` in an air-gapped configuration: it intentionally means public HTTP(S)
   egress. The explicit `--sandbox-profile offline-default` above is essential because
   the omitted sandbox default would otherwise permit ambient network access.
   The built-in `echo` route, redb journal, built-in policy, local workflows, repository
   tools, and lexical index need no internet access.

   The command above uses `--storage-keys none`, the dependency-free default. Add
   `--storage-keys environment` or `--storage-keys platform` when confidentiality,
   signed checkpoints, and rollback anchors are required.

6. Run the acceptance sequence:

    ```bash
    colossus -w . policy doctor
    colossus -w . state doctor
    colossus -w . sandbox doctor
    colossus -w . run "airgap acceptance"
    colossus -w . audit verify
    colossus -w . audit anchor-status
    ```

For a local OpenAI-compatible model, grant only its loopback origin, define a provider
connection plus a model profile with explicit limits/capabilities, and route
`models.roles.primary` to that model profile.

`workspace-development` may still be used for a physically disconnected developer
workstation, but it supplies workspace writes and a shell. `offline-default` remains the
recommended audit/smoke baseline and never acquires those derived grants.

## Expected result

The run completes with no external network grant and the hash-chained journal verifies.
With protected storage it also writes ciphertext, creates a signed checkpoint, and
verifies against the secure anchor.

## Verification

Retain the Colossus version, archive or bundle digest, config hash, effective network
destinations, run ID, audit verification, and anchor status. Independently confirm at the
host boundary that no unapproved egress occurred.

## Failure path

- A checksum proves transport integrity, not publisher identity; require the signed
  bundle when authenticity matters.
- Keyless storage is plaintext. Protect its volume, or initialize a fresh journal with
  separately managed environment keys when confidentiality is required.
- An unavailable external adapter degrades explicitly; Colossus does not discover or
  contact alternatives.
- Never blindly retry an unknown external effect after reconnecting the environment.

## Next step

Review the exact [Bundle format](../reference/bundle-format.md) and the operational
[Troubleshooting](troubleshooting.md) guide before sealing the environment.
