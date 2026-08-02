---
title: Audit, telemetry, and recovery
description: Verify evidence, export bounded audit records, inspect telemetry, and respond to uncertain effects.
audience: operator
type: how-to
---

# Audit, telemetry, and recovery

## Goal

Keep verifiable operational evidence and respond safely when canonical state or an
external effect is uncertain.

## Prerequisites

- Access to the deployment configuration and its key provider.
- The separately protected secure anchor.
- An approved evidence destination if audit export is enabled.

## Steps

1. Verify journal and anchor integrity:

    ```bash
    colossus --config .colossus/config.yaml audit verify
    colossus --config .colossus/config.yaml audit anchor-status
    colossus --config .colossus/config.yaml state doctor
    ```

2. Inspect bounded evidence and metadata-only telemetry:

    ```bash
    colossus --config .colossus/config.yaml audit show --limit 50
    colossus --config .colossus/config.yaml telemetry runs
    ```

3. If configured, inspect and drain the durable exporter queue:

    ```bash
    colossus --config .colossus/config.yaml audit exporter-status
    colossus --config .colossus/config.yaml --approval-mode ask \
      audit exporter-drain
    ```

   Draining writes to the configured directory or WORM destination. Under the
   development profile, the noninteractive CLI therefore needs an explicit approval
   mode.

4. If an effect is marked `outcome_unknown`, reconcile it with the external system
   before any retry. Use only the operation-specific explicit recovery route.

Directory export writes ciphertext-free evidence to an existing permitted directory.
HTTPS WORM export uses create-only object writes and deterministic names, but the remote
service must independently enforce retention or object lock.

## Expected result

Verification confirms the hash chain, signed checkpoint, and secure anchor. Telemetry
reports counts and timing without prompts, hidden reasoning, or raw tool output. Export
status identifies a durable position or a bounded actionable failure.

`audit verify` is always a complete journal audit. Normal startup may report
`incremental` after a version-two anchor has established the older prefix; inspect
`storage.startup_verification` in `state doctor` for the configured mode, actual path,
verified sequence range, inspected event count, and anchor format.

## Verification

Retain the config hash, journal head, anchor status, relevant run/effect ID, decision
revision, and export position with the operating record. Verify that exported evidence
does not contain payload ciphertext, plaintext, nonces, or credential values.

## Failure path

A chain, checkpoint, anchor, decryption, or projection-position failure activates
read-only recovery and blocks new effects. Preserve the journal, key identity, secure
anchor, and diagnostic output. Do not delete or rewrite events to make verification
pass.

An exporter `outcome_unknown` blocks implicit delivery replay. Investigate the
destination and use the operator-authorized reset only after establishing whether the
object exists.

## Next step

Use [Troubleshooting](troubleshooting.md) to collect a safe diagnostic bundle, or read
[State and recovery](../develop/state-recovery.md) for the underlying invariants.
