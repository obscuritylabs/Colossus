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
- A verified backup of configuration and canonical state, plus the secure anchor and
  matching key material or credential-store entries when protected storage is
  configured.
- A maintenance window for verification.

This site documents only the latest supported release. The root `CHANGELOG.md` is the
release-history authority.

## Steps

### 1. Read the release history

Review the target release in
[CHANGELOG.md](https://github.com/obscuritylabs/Colossus/blob/main/CHANGELOG.md) and note
configuration or operational changes.

### 2. Preserve the current authority set

Stop Desktop and active workers, then back up the Colossus home YAML configuration,
`AGENTS.md`, required workspace partitions, journal, secure anchor, index positions,
and their corresponding key identities. A state file without its keys and anchor is
not a complete recovery set. See
[Colossus home and workspace resolution](../reference/colossus-home.md#back-up-restore-and-uninstall).

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

Generate a fresh configuration at a separate path. Pass an isolating preset explicitly
when you do not want the new full-access default:

```bash
colossus --config .colossus/config.next.yaml config init \
  --access-profile development \
  --sandbox-profile workspace-development
```

Only `schemaVersion` and `storage` are required at the document root. Ordinary nested
groups are recursively defaultable, while explicit tagged variants still require their
`kind` and unknown fields remain errors. Copy required provider connections into
`providers.profiles`; copy model identifiers, context limits, capabilities, and role
mappings into `models.profiles` and `models.roles`. Transfer storage, policy, sandbox,
and integration settings deliberately, and retain credential references rather than
secret values. `config init` never overwrites an existing file.

The deprecated nested `research.search` adapter is no longer accepted. Move its SearXNG
endpoint to a named top-level `search.profiles` entry and map
`search.roles.research` to that profile. If the old configuration set an explicit access
decision for `network.http`, add the corresponding decision for `web.search`; search is
now evaluated as its own action and does not inherit the direct-HTTP decision.
Unversioned Python-era theme files are also no longer imported; generate a template with
interactive `/theme scaffold NAME` or convert them to the documented schema-version-1
format.
Inspect the completed file before making it active:

```bash
colossus --config .colossus/config.next.yaml config show
colossus --config .colossus/config.next.yaml config effective
```

Normal `config init` now creates `$COLOSSUS_HOME/config.yaml` with
`storage.location: home_workspace`; `config init --local` creates the repository
replacement. Existing repository-local `.colossus/config.yaml` files still take
precedence and omitted `storage.location` retains historical `workspace` behavior, so
their state paths continue to work without silent relocation.

This release intentionally changes the meaning of omitted schema-version-2 access and
sandbox fields. An omitted or empty `access` group resolves to `allow_all`; an omitted
or empty `sandbox` group resolves to acknowledged `danger_full_access`. That widening
applies to existing sparse files without a schema-version bump, including workflows and
background effects. Existing files that explicitly select a native, Windows, OCI,
external, or custom sandbox retain that selection. Run `config show` and `config
effective` before returning an upgraded host to service, and add an explicit isolating
sandbox block if ambient host filesystem, process, and HTTP(S) authority is not intended.

Desktop Managed Local starts fresh in the selected workspace's isolated Desktop home
partition. Earlier application-support data is preserved but ignored; Colossus neither
imports nor deletes it. Keep it until the new Desktop state is verified, then handle it
through your normal retention process.

Fresh schema-v4 Desktop settings default to **Allow all** with **Full access**. Existing
schema-v1–v3 settings instead preserve their earlier platform-isolated behavior during
migration: **Minimal** maps to **Offline isolated**, while **Development** and the legacy
`allow_all` spelling map to **Workspace isolated**. Review the new execution-boundary
selector separately from the approval-mode selector. **Offline isolated** hides the
generic model-visible `network.http`, `web.fetch`, and `docs.fetch` tools, but it retains
the selected provider's exact service and authentication/refresh destinations. It is
not an air gap; use the
[offline and air-gapped operation guide](../admin/offline-airgap.md) when remote provider
transport must also be absent.

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
