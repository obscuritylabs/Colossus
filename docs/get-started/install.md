---
title: Install Colossus
description: Install and verify the native Colossus release archive without a runtime package manager.
audience: user
type: how-to
---

# Install Colossus

For the macOS folder-first application, use [Colossus Desktop](desktop.md). It ships the
CLI and managed runtime inside the signed app and does not require this separate native
installation. Continue here for CLI, TUI, installed-daemon, and server deployments.

## Goal

Install the native `colossus` executable into your user-local binary directory and
verify that your shell can find it.

## Prerequisites

- Access to the official
  [Colossus Releases](https://github.com/obscuritylabs/Colossus/releases) page.
- A terminal with permission to write to your chosen installation prefix.

The archive installer does not require a language runtime or make a network request.

## Steps

### 1. Choose and download the release asset

Open the current release or Developer Preview and download both the archive and its
adjacent `.sha256` sidecar. Asset names follow `colossus-VERSION-TARGET.EXT`:

| Host | Target | Extension |
| --- | --- | --- |
| macOS, Apple silicon | `aarch64-apple-darwin` | `tar.gz` |
| macOS, Intel | `x86_64-apple-darwin` | `tar.gz` |
| Linux, ARM64 | `aarch64-unknown-linux-musl` | `tar.gz` |
| Linux, x86-64 | `x86_64-unknown-linux-musl` | `tar.gz` |
| Windows, ARM64 | `aarch64-pc-windows-msvc` | `zip` |
| Windows, x86-64 | `x86_64-pc-windows-msvc` | `zip` |

For Apple silicon, substitute the release's value for `VERSION` in
`colossus-VERSION-aarch64-apple-darwin.tar.gz` and its `.sha256` sidecar. Keep both
files in the same directory.

### 2. Verify the download

Set `VERSION` and `TARGET` in the examples to the names on the release. The check must
report success before extraction:

=== "macOS"

    ```bash
    shasum -a 256 -c colossus-VERSION-TARGET.tar.gz.sha256
    ```

=== "Linux"

    ```bash
    sha256sum --check colossus-VERSION-TARGET.tar.gz.sha256
    ```

=== "Windows PowerShell"

    ```powershell
    $archive = "colossus-VERSION-TARGET.zip"
    $expected = (Get-Content "$archive.sha256").Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Colossus checksum mismatch" }
    ```

A checksum detects a damaged or substituted download. When publisher authenticity is
required, verify a signed offline bundle as described in
[Offline operation](../admin/offline-airgap.md).

### 3. Run the included installer

=== "macOS and Linux"

    ```bash
    tar -xzf colossus-VERSION-TARGET.tar.gz
    ./colossus-VERSION-TARGET/install.sh
    export PATH="$HOME/.local/bin:$PATH"
    ```

=== "Windows PowerShell"

    ```powershell
    Expand-Archive .\colossus-VERSION-TARGET.zip
    .\colossus-VERSION-TARGET\install.ps1
    $env:PATH = "$HOME\.local\bin;$env:PATH"
    ```

Use `--prefix PATH` on macOS or Linux, or `-Prefix PATH` on Windows, to choose another
installation root.

For `sandbox.profile: workspace-development` on Ubuntu 24.04 or later, first install the
Linux binary at a root-owned, non-replaceable path and then load the archive's narrowly
attached AppArmor profile:

```bash
sudo ./install.sh --prefix /usr/local
sudo ./install-apparmor.sh /usr/local/bin/colossus
```

This is not required for the offline quickstart or on hosts where `sandbox doctor`
already reports protected-path exclusions as supported. Do not disable Ubuntu's
host-wide unprivileged-user-namespace restriction; use the exact-path profile or the OCI
backend.

### 4. Confirm the executable

=== "macOS and Linux"

    ```bash
    colossus --version
    ```

=== "Windows PowerShell"

    ```powershell
    colossus.exe --version
    ```

## Expected result

The command prints the Colossus release identifier and exits successfully.

## Verification

Open a new terminal and run `colossus --version` again. If it succeeds without an
absolute path, the installation directory is present in your persistent `PATH`.

## Failure path

- **Command not found:** add the installer prefix's `bin` directory to your shell profile,
  then open a new terminal.
- **Checksum mismatch:** do not install the archive. Download it again from the official
  release and recheck.
- **Permission denied:** install to a user-owned prefix rather than elevating the
  installer.
- **Platform blocks execution:** confirm that the archive matches your operating system
  and architecture, then follow your organization's software verification process.

## Next step

Run the credential-free [five-minute quickstart](quickstart.md).
