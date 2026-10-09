---
title: CLI installation options
description: Review the installer, choose a release or prefix, and check supported targets.
audience: user
type: reference
icon: lucide/package
---

# CLI installation options

The [CLI installation guide](install.md) has direct, package-manager, and offline
methods. Use this page to inspect the bootstrap, pin a version, or choose an
installation prefix. Keep using the same owner for updates and removal.

## Review the installer before running it

Download the bootstrap, read it, and run a dry run before installation:

=== "macOS and Linux"

    ```bash
    curl -fSLo colossus-install.sh \
      https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh
    less colossus-install.sh
    sh colossus-install.sh --dry-run
    sh colossus-install.sh --yes
    ```

=== "Windows PowerShell"

    ```powershell
    Invoke-WebRequest `
      https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.ps1 `
      -OutFile colossus-install.ps1
    Get-Content .\colossus-install.ps1
    .\colossus-install.ps1 -DryRun
    .\colossus-install.ps1 -Yes
    ```

The versioned bootstrap source is retained in the corresponding Git tag under
`release/bootstrap/`. Release assets include adjacent SHA-256 files for offline
comparison. A dry run resolves the release without downloading an archive, creating a
directory, or installing anything.

## Choose a version, channel, or prefix

Pass these options to the downloaded installer script:

| Choice | macOS and Linux | Windows PowerShell |
| --- | --- | --- |
| Exact stable version | `--version vX.Y.Z` | `-Version vX.Y.Z` |
| Latest preview | `--channel preview` | `-Channel preview` |
| Exact preview | `--channel preview --version vX.Y.Z-preview.N` | `-Channel preview -Version vX.Y.Z-preview.N` |
| Absolute installation prefix | `--prefix PATH` | `-Prefix PATH` |
| Resolve without installing | `--dry-run` | `-DryRun` |
| Forbid profile changes | `--no-modify-path` | `-NoModifyPath` |
| Mark unattended use | `--yes` | `-Yes` |

Stable is the default channel. The current installers do not prompt or change shell
profiles. The chosen prefix must have trusted ownership and no replaceable, shared
writable path components.

## Supported targets

| Host | Release target | Archive |
| --- | --- | --- |
| macOS, Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| macOS, Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Linux, ARM64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| Linux, x86-64 | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows, ARM64 | `aarch64-pc-windows-msvc` | `.zip` |
| Windows, x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

An unsupported host fails before any archive download.

## Ubuntu workspace sandbox

For `sandbox.profile: workspace-development` on Ubuntu 24.04 or later, a root-owned,
non-replaceable binary path and the archive's narrowly attached AppArmor profile may be
needed:

```bash
sudo ./install.sh --prefix /usr/local
sudo ./install-apparmor.sh /usr/local/bin/colossus
```

Check `sandbox doctor` first. If it already reports protected-path exclusions as
supported, this step is unnecessary. Do not disable Ubuntu's host-wide
unprivileged-user-namespace restriction to run Colossus; use the exact-path profile or
the OCI backend.

After installing, return to [CLI verification](install.md#2-verify-the-command). For
ownership, updates, and removal, see [CLI installation lifecycle](cli-installation-lifecycle.md).
