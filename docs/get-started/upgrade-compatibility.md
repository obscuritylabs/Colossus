---
title: Upgrade and compatibility
description: Upgrade Colossus safely, refresh configuration deliberately, and understand the Rust cutover boundary.
audience: operator
type: how-to
---

# Upgrade and compatibility

## Goal

Move an installation to the latest supported Colossus release without overwriting
configuration or silently importing incompatible state.

## Prerequisites

- The release archive, checksum, and release notes for the target version.
- A verified backup of configuration, encrypted state, secure anchor, and the matching
  key material or credential-store entries.
- A maintenance window for verification.

This site documents only the latest supported release. The root `CHANGELOG.md` is the
release-history authority.

## Steps

### 1. Read the release history

Review the target release in
[CHANGELOG.md](https://github.com/obscuritylabs/Colossus/blob/main/CHANGELOG.md) and note
configuration or operational changes.

### 2. Preserve the current authority set

Back up the YAML configuration, journal, secure anchor, index positions, and their
corresponding key identities. A state file without its keys and anchor is not a complete
recovery set.

### 3. Install the new binary beside the old one

Verify the archive checksum, install into a staged prefix, and run:

```bash
/staged/prefix/bin/colossus --version
```

Do not remove the prior executable until the new binary has passed diagnostics.

### 4. Regenerate configuration when its shape changed

Colossus is pre-1.0, so configuration shapes may change without an automated migration
command. Version 0.10.1 and later require `schemaVersion: 2`, which separates provider connection
profiles from model profiles and logical role routing. Schema version 1 is rejected
instead of being silently reinterpreted.

Generate a fresh configuration at a separate path:

```bash
colossus --config .colossus/config.next.yaml config init \
  --access-profile development
```

Configurations without the required `access` block, or with removed `agent.tools`,
`policy.allow_actions`, or `policy.approval_actions` fields, are rejected. Copy required
provider connections into `providers.profiles`; copy model identifiers, context limits,
capabilities, and role mappings into `models.profiles` and `models.roles`. Transfer
storage, policy, sandbox, and integration settings deliberately, and retain credential
references rather than secret values. `config init` never overwrites an existing file.
Inspect the completed file before making it active:

```bash
colossus --config .colossus/config.next.yaml config show
colossus --config .colossus/config.next.yaml config effective
```

### 5. Verify local state, then run explicit diagnostics

```bash
colossus --config .colossus/config.next.yaml state doctor
colossus --config .colossus/config.next.yaml audit verify
colossus --config .colossus/config.next.yaml sandbox doctor
```

Use the original config path when no regenerated configuration was needed. These
commands stay local. After they pass, explicitly authorize the provider catalog
diagnostic for a network profile:

```bash
colossus --config .colossus/config.next.yaml --approval-mode ask \
  provider doctor
```

`provider doctor` is network-free for `echo`; network providers call their configured
model-catalog endpoint through the effect gateway.

## Expected result

The staged binary accepts the intended configuration, opens the expected canonical
state, verifies its audit chain and anchor, and reports provider and sandbox diagnostics
without exposing credentials.

## Verification

Run the credential-free, network-free `echo` profile first, then compare `config
effective` with the saved pre-upgrade output. Echo is still an audited provider effect.
Promote the staged binary only after the differences are understood.

## Failure path

- **Configuration is rejected:** generate a fresh file at a separate path and transfer
  reviewed settings; do not hand-edit the original under pressure.
- **State or anchor verification fails:** stop writes, restore the complete authority
  set, and use read-only recovery diagnostics.
- **Provider diagnostics fail:** confirm credential references and exact origins without
  printing secret values.
- **Rollback is required:** restore the prior binary and its matching complete state
  authority set. Never merge journals.

## Python 0.5 cutover

The active implementation is the native Rust runtime. Python 0.5 is retained only on the
`python-v0.5.0` tag and `python-legacy` branch. Fresh Rust installations do not import
Python configuration, SQLite state, history, or audit files. Keep legacy data isolated
and plan any required business-data transition as an explicit, reviewed migration.

## Next step

Review [Storage and worker](../admin/storage-worker.md) and
[Audit, telemetry, and recovery](../admin/audit-telemetry-recovery.md) before returning
an upgraded deployment to service.
