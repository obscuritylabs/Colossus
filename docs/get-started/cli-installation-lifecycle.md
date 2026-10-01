---
title: CLI installation lifecycle
description: Check for updates, understand install ownership, and remove a direct Colossus CLI installation.
audience: user
type: reference
icon: lucide/refresh-cw
---

# CLI installation lifecycle

The direct installer records which executable it owns. Homebrew and Nix installations
remain owned by those package managers. Preserve that distinction when updating or
removing the CLI.

## Check for updates

```bash
colossus update check
```

This read-only command reports the running version and the latest validated stable
release. An offline, timed-out, or rate-limited check reports `unavailable` instead of
interrupting Colossus. Successful metadata is cached for 24 hours. The interactive TUI
also checks once in the background after startup and shows a version-only notice when a
newer stable release is available.

## Update a direct installation

```bash
colossus update
```

To select one exact newer stable release:

```bash
colossus update --version vX.Y.Z
```

The update applies only when the direct-install receipt names the running executable.
Colossus refuses a downgrade, a stale receipt, and replacement of a source, Homebrew,
Nix, or unknown installation. Use the owning package manager for those installations.
Review [upgrade and compatibility guidance](upgrade-compatibility.md) before changing
an installation with state you depend on.

## Installation ownership and state

A direct install writes an ownership receipt at:

- Unix: `$XDG_DATA_HOME/colossus/install.json`, or
  `$HOME/.local/share/colossus/install.json` when `XDG_DATA_HOME` is unset.
- Windows: `%LOCALAPPDATA%\Colossus\install.json`.

The receipt records the release, target, prefix, binary path, distribution origin, and
`direct` installer kind. The installer checks the destination path before writing and
restores the previous executable if receipt creation fails.

A per-user install creates an empty, owner-private Colossus home at `$COLOSSUS_HOME` or
the platform default. It does not create configuration, a database, credentials, or
repository files. Elevated or system-token installs defer home creation until the first
ordinary user launch. See [Colossus home and workspace resolution](../reference/colossus-home.md)
for the directory layout and privacy rules.

## Remove a direct installation

Inspect the receipt before removing anything. Confirm `installerKind` is `direct` and
that `binaryPath` names the executable you intend to remove:

=== "macOS and Linux"

    ```bash
    receipt="${XDG_DATA_HOME:-$HOME/.local/share}/colossus/install.json"
    less "$receipt"
    ```

=== "Windows PowerShell"

    ```powershell
    $receipt = Join-Path $env:LOCALAPPDATA "Colossus\install.json"
    Get-Content $receipt | ConvertFrom-Json | Format-List
    ```

Remove only that verified binary and the receipt. Remove their parent directories only
when empty. The direct installer can leave versioned bundled tools so an older running
Colossus process can finish; remove obsolete versions only after those processes exit.
For a full uninstall, inspect the owned `PREFIX/bin/.colossus-tools` directory before
removing its current version as well.

Use Homebrew, Nix, or Desktop's own removal path for installations they own.
Uninstalling a CLI executable preserves the Colossus home, including configuration,
workspace state, instructions, and trust records. Back it up or remove it only as a
separate data-lifecycle decision; see
[backup, restore, and uninstall](../reference/colossus-home.md#back-up-restore-and-uninstall).
