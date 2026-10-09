---
title: Install the Colossus CLI
description: Install the latest stable Colossus CLI and verify that it runs.
audience: user
type: how-to
icon: lucide/download
---

# Install the Colossus CLI

Install the standalone CLI on a [supported system](install-options.md#supported-targets).
For the app, see [Desktop setup](desktop.md); it includes the CLI.

## 1. Install the CLI

Choose the method for your system:

=== "macOS / Linux"

    ```bash
    curl -fsSL https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh | sh
    ```

    The direct installer verifies the release and its checksum. It installs to a
    user-local prefix by default and prints the `PATH` command to use if needed.

=== "Homebrew"

    On macOS, install from the official tap:

    ```bash
    brew install obscuritylabs/tap/colossus
    ```

    Homebrew owns updates and removal for this installation.

=== "Nix"

    Install from the repository's locked flake:

    ```bash
    nix profile install github:obscuritylabs/Colossus
    ```

    Nix owns updates and removal for this installation.

=== "Windows PowerShell"

    ```powershell
    irm https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.ps1 | iex
    ```

    Run this in **PowerShell**. The direct installer verifies the release and its
    checksum, installs to a user-local prefix by default, and prints the `PATH`
    command to use if needed.

=== "Offline archive"

    Download the archive for your [target](install-options.md#supported-targets) and
    its adjacent `.sha256` file from [Colossus Releases](https://github.com/obscuritylabs/Colossus/releases).
    Copy both files to the target computer. Replace `VERSION` and `TARGET` below with
    the exact names you downloaded.

    **macOS:**

    ```bash
    shasum -a 256 -c colossus-VERSION-TARGET.tar.gz.sha256
    tar -xzf colossus-VERSION-TARGET.tar.gz
    ./colossus-VERSION-TARGET/install.sh
    ```

    **Linux:**

    ```bash
    sha256sum --check colossus-VERSION-TARGET.tar.gz.sha256
    tar -xzf colossus-VERSION-TARGET.tar.gz
    ./colossus-VERSION-TARGET/install.sh
    ```

    **Windows PowerShell:**

    ```powershell
    $archive = "colossus-VERSION-TARGET.zip"
    $expected = (Get-Content "$archive.sha256").Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Colossus checksum mismatch" }
    Expand-Archive $archive
    .\colossus-VERSION-TARGET\install.ps1
    ```

For a pinned version, an inspected installer, or a custom prefix, see
[CLI installation options](install-options.md).

## 2. Verify the command

Open a new terminal if you changed `PATH`, then run:

```bash
colossus --version
```

You should see a version number. If the command is not found after a direct install,
apply the `PATH` command printed by the installer and open a new terminal.

## What's next?

<div class="grid cards" markdown>

-   :lucide-zap:{ .lg .middle } **Five-minute quickstart**

    ---

    Run Colossus offline, find your configuration, and verify the audit journal.

    [Start the quickstart :lucide-arrow-right:](quickstart.md)

-   :lucide-terminal:{ .lg .middle } **Commands and flags**

    ---

    Explore the commands available in the CLI and terminal UI.

    [Browse CLI commands :lucide-arrow-right:](../reference/cli.md)

</div>
