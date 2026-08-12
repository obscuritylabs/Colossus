---
title: Colossus home and workspace resolution
description: Resolve per-user configuration, workspace-partitioned state, and AGENTS.md instructions.
audience: operator
type: reference
---

# Colossus home and workspace resolution

Colossus keeps per-user control state in one owner-private home while treating the
selected workspace as the repository and maximum filesystem/tool boundary. These are
separate concepts: `-w, --workspace` changes what repository a run can access; it does
not move the Colossus home.

## Resolve the home

Colossus resolves its home once at process startup:

1. `COLOSSUS_HOME`, when set.
2. `$HOME/.colossus` on macOS and Linux, or the `.colossus` directory beneath the
   Windows user home.

`COLOSSUS_HOME` must be a nonempty absolute path. The home must be a real directory,
not a symlink or reparse point, and every existing path component must pass the
platform's no-follow and namespace-authority checks. On Unix, ancestors must be owned
by root or the effective user and cannot be group- or world-writable unless protected
by the sticky bit; the home itself must be owned by the effective user and grant no
group or other access. Newly created Unix homes use mode `0700`. On Windows, ancestor
owners must be the current user or a trusted local system principal and their effective
ACLs cannot give an untrusted principal delete or permission-control authority; the
home itself uses an owner-private DACL. Unsafe state is rejected with a remediation
error rather than repaired or ignored.

The per-user direct installer creates this private root empty when it is absent, or
validates an existing root without changing its contents. It does not create
configuration, a database, or credentials. A privileged or system-token package install
has no reliable end-user identity, so it defers home creation until a non-privileged
user first starts Colossus. On Unix, `sudo ./install.sh --prefix /usr/local` is the
common example. Bootstrap `--dry-run`/`-DryRun` does not create the home.

## Directory layout

The supported layout is:

```text
~/.colossus/
  config.yaml
  AGENTS.md
  desktop/
    settings.json
    trust/
    self-test/
  workspaces/<partition-id>/
    cli/
    desktop/
```

| Path | Purpose |
| --- | --- |
| `config.yaml` | Complete user-level YAML configuration |
| `AGENTS.md` | User-level agent instructions applied to later top-level runs |
| `desktop/` | Desktop settings, imported trust, and self-test data |
| `workspaces/<partition-id>/cli/` | CLI/TUI state for one canonical workspace |
| `workspaces/<partition-id>/desktop/` | Separately isolated Desktop Managed Local state for that workspace |

`<partition-id>` is a versioned, domain-separated SHA-256 identity derived from the
canonical workspace and its filesystem object identity. It is opaque and not a public
identifier. Renaming or replacing a workspace selects a new partition; Colossus
never silently attaches a replacement filesystem object to the prior database.

The `cli` and `desktop` partitions intentionally cannot alias. Their redb writer
leases, worker authentication, provider-key namespaces, and application lifecycle stay
independent even when both interfaces select the same repository.

## Select configuration

For commands that need runtime configuration, Colossus selects exactly one complete
document in this order:

1. Explicit `--config PATH`.
2. `<workspace>/.colossus/config.yaml`.
3. `$COLOSSUS_HOME/config.yaml`.
4. An actionable missing-configuration error.

Relative explicit paths resolve from the canonical workspace. Selection is not a
merge: candidates are complete replacements, and the first one that applies replaces
every lower-priority document. A missing or malformed explicit path fails without
fallback. Automatic repository and home candidates are also opened no-follow and fail
without fallback when present but unsafe.

Create the normal user-level configuration with:

```bash
colossus -w /absolute/path/to/repository config init
```

Create a repository-specific replacement instead with:

```bash
colossus -w /absolute/path/to/repository config init --local
```

The local form writes `<workspace>/.colossus/config.yaml` and conflicts with explicit
`--config`. Initialization never overwrites an existing file.

`config effective` adds this exact credential-free `resolution` object alongside the
ordinary access diagnostics:

| Field | Values or meaning |
| --- | --- |
| `configSource` | `explicit`, `workspace`, or `global` |
| `configScope` | `explicit`, `local`, or `global` |
| `configPath` | Absolute selected configuration path |
| `colossusHome` | Absolute validated home |
| `workspacePartitionId` | Opaque workspace partition hash |
| `statePath` | Absolute resolved storage path |

This metadata contains no credentials or private sidecar bootstrap material.

## Resolve storage

`storage.location` controls the base for a relative `storage.path`:

| Value | Relative-path base | Use |
| --- | --- | --- |
| `workspace` | Canonical selected workspace | Repository-owned and existing configurations |
| `home_workspace` | `workspaces/<partition-id>/cli/` | Normal user-level CLI/TUI configuration |

Omitting `storage.location` preserves the historical `workspace` behavior. Under
`home_workspace`, `storage.path` must be a confined relative path: absolute paths,
parent traversal, and paths that escape the CLI partition are rejected. A global
`config init` writes `location: home_workspace` with `path: state.redb`; the local
initialization form writes `location: workspace`.

Desktop Managed Local does not reuse this CLI path. It stores generated runtime
configuration, the canonical database, indexes, and private runtime files beneath the
workspace's `desktop` partition.

## Load AGENTS.md

Each top-level user-facing agent run snapshots instructions in this precedence order:

1. `$COLOSSUS_HOME/AGENTS.md` supplies user defaults.
2. `<workspace>/AGENTS.md` refines them for the repository.
3. Explicit instructions supplied for the invocation take precedence.
4. Immutable Plan Mode, Goal Mode, and other runtime-mode instructions remain highest.

Home and repository instruction files must be no-follow regular UTF-8 files. Each is
limited to 64 KiB and their combined content to 128 KiB. A present file that is linked,
unreadable, invalid UTF-8, or oversized fails the run clearly instead of being skipped.

The snapshot is fixed when the top-level run starts. Provider turns, Goal iterations,
and delegated subagents inherit the same content and SHA-256 provenance even if a file
changes mid-run. A later top-level run reads the files again. Durable subagent recovery
uses the parent's persisted snapshot reference. `state doctor` reports the content-free
`instruction_sources` contract: `load_order`, `snapshot_refresh: top_level_run`, and
`sources` entries containing only a source label and `sha256`. It never returns file
paths or instruction text.

These files guide user-facing agent work only. They are not injected into risk
evaluation, context summarization, provider diagnostics, or other internal security
roles. Instructions cannot add tools, widen sandbox roots or network destinations,
grant policy or approval authority, or bypass immutable runtime constraints.

## Back up, restore, and uninstall

Stop Desktop and any CLI worker before copying canonical redb state. Back up the
configuration, `AGENTS.md`, required workspace partitions, secure anchors, and the
matching operating-system credential or environment-key material as one authority set.
Replaceable indexes and `desktop/self-test/` output can be rebuilt.

Restoring a directory does not make it authoritative for a different workspace
identity. Verify the selected partition and `config effective` state path before
starting writes; never merge journals from two partitions.

Uninstalling the executable or Desktop application preserves `$COLOSSUS_HOME` by
default. Remove that directory only when you explicitly intend to delete all user
configuration, instructions, Desktop settings and trust, and every workspace's CLI and
Desktop state. The direct installer receipt and update cache remain separate platform
data and cache records described in [Install Colossus](../get-started/install.md).

## Failure cases

- **Unsafe home:** move the contents to a real current-user-owned private directory;
  do not bypass the ownership, permission, or no-follow check.
- **Wrong configuration selected:** run `config effective` and inspect its source and
  scope before editing anything.
- **Unexpected empty state:** confirm the canonical workspace and partition ID. A
  renamed or replaced repository intentionally receives a different partition.
- **AGENTS.md rejected:** replace links with a bounded regular UTF-8 file and keep each
  source within 64 KiB.
