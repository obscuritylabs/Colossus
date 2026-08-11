---
title: Install Colossus
description: Install and verify the native Colossus CLI from public GitHub Releases.
audience: user
type: how-to
---

# Install Colossus

For the macOS folder-first application, use [Colossus Desktop](desktop.md). It ships the
CLI and managed runtime inside the signed app. Continue here for standalone CLI, TUI,
installed-daemon, and server deployments.

## Goal

Install the latest stable native `colossus` executable without Rust, Git, Homebrew,
Nix, administrator access, or a language runtime.

## Prerequisites

- macOS, Linux, or Windows on a [supported architecture](#supported-targets).
- `curl` and `tar` on macOS or Linux, or Windows PowerShell on Windows.
- Permission to write to the chosen installation prefix.
- Anonymous HTTPS access to the public
  [Colossus Releases](https://github.com/obscuritylabs/Colossus/releases) origin.

Public, immutable GitHub Release assets are the authoritative direct-install origin.
The bootstrap refuses draft releases, channel/version disagreements, missing target
assets, unexpected redirect hosts, oversized responses, unsafe archive layouts, and
checksum mismatches.

Choose one installation owner and keep using it for upgrades:

- **Direct installer (recommended):** one native binary, an owner-local receipt, and
  install-aware `colossus update` support.
- **Nix:** the repository flake installs the latest release pinned in that flake; Nix
  remains the owner and `colossus update` will not mutate the store.
- **Homebrew:** use the direct installer until the separately maintained
  `obscuritylabs/homebrew-tap` is published.
- **Manual archive:** the offline and root-owned system-install path.

## Steps

### 1. Install the latest stable release

=== "macOS and Linux"

    ```bash
    curl -fsSL https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh | sh
    ```

=== "Windows PowerShell"

    ```powershell
    irm https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.ps1 | iex
    ```

The installer defaults to `$HOME/.local`. It never uses `sudo` and never changes a
shell or PowerShell profile. If the prefix's `bin` directory is absent from `PATH`, the
installer prints the exact process-local command to use.

### 2. Review the bootstrap before running it

Use the two-step form when your policy does not permit piping a network response into a
shell.

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

The versioned source for each published bootstrap is also retained in the corresponding
Git tag under `release/bootstrap/`. Release assets include adjacent SHA-256 sidecars for
offline comparison of the bootstrap bytes.

### 3. Select a version, channel, or prefix

| Behavior | macOS and Linux | Windows PowerShell |
| --- | --- | --- |
| Exact stable version | `--version vX.Y.Z` | `-Version vX.Y.Z` |
| Latest preview | `--channel preview` | `-Channel preview` |
| Exact preview | `--channel preview --version vX.Y.Z-preview.N` | `-Channel preview -Version vX.Y.Z-preview.N` |
| Custom absolute prefix | `--prefix PATH` | `-Prefix PATH` |
| Resolve without installing | `--dry-run` | `-DryRun` |
| Explicitly forbid profile changes | `--no-modify-path` | `-NoModifyPath` |
| Intentional noninteractive use | `--yes` | `-Yes` |

Stable is always the default channel. The current scripts do not prompt or modify
profiles, so `--yes` marks intentional unattended use and `--no-modify-path` preserves
that contract explicitly.

### Supported targets

| Host | Release target | Archive |
| --- | --- | --- |
| macOS, Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| macOS, Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Linux, ARM64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| Linux, x86-64 | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows, ARM64 | `aarch64-pc-windows-msvc` | `.zip` |
| Windows, x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

The host detector maps only these exact operating-system and architecture pairs. An
unsupported host fails before any archive download.

### Installation receipt

A successful direct installation writes a bounded, credential-free ownership receipt:

- Unix: `$XDG_DATA_HOME/colossus/install.json`, falling back to
  `$HOME/.local/share/colossus/install.json`.
- Windows: `%LOCALAPPDATA%\Colossus\install.json`.

The receipt records only its schema version, release channel and version, target,
prefix, binary path, fixed distribution origin, and `direct` installer kind. The
installer rejects linked or unsafe destination directories and commits the binary and
receipt with same-directory temporary files. If receipt commit fails, it restores the
previous executable.

### Check for a newer stable release

Run update discovery independently of any workspace or configuration:

```bash
colossus update check
```

On a terminal, the command shows the running version and latest validated stable
version. When redirected, or with `--output json`, it emits the versioned structured
report. The check itself is always read-only.

The check contacts only the fixed public GitHub latest-stable metadata endpoint. It
rejects redirects, proxies, preview releases, malformed semantic versions, unexpected
release pages, and releases missing the exact target archive or checksum. Successful
metadata and conditional request state are cached for 24 hours. Offline, timed-out,
rate-limited, and malformed responses return a successful `unavailable` report instead
of interrupting Colossus; failed checks are also throttled for 24 hours.

The interactive TUI performs the same check once in the background after startup. It
shows a version-only notice when a newer stable release is available. No notice is
shown when discovery is offline or otherwise unavailable, and startup never waits for
the request.

Update cache locations are:

- Unix: `$XDG_CACHE_HOME/colossus/`, falling back to
  `$HOME/.cache/colossus/`.
- Windows: `%LOCALAPPDATA%\Colossus\`.

### Update a direct installation

The direct installer owns only the executable named by its matching receipt. Update to
the latest validated stable release with:

```bash
colossus update
```

Select one exact newer stable release for a reproducible update:

```bash
colossus update --version vX.Y.Z
```

Colossus refuses downgrades, preview-to-stable ownership changes, stale receipts, and
receipts that do not name the canonical running executable. Source builds and unknown,
Homebrew-owned, or Nix-owned executables are never replaced; use the installation
channel that owns them. To intentionally adopt a direct-install prefix, run the
reviewed bootstrap with an explicit `--prefix`/`-Prefix` instead.

The released binary embeds the exact reviewed bootstrap from its source tag. On macOS
and Linux the bootstrap downloads, verifies, and installs the selected archive before
returning. Windows hands the same bootstrap to a detached helper so the running image
can exit before replacement. The packaged installer stages the new binary and receipt
in their destination directories, replaces them atomically, and restores the previous
binary if the receipt cannot commit.

### Install with Nix

The repository includes a locked flake that selects the reviewed native archives and
digests for the published release pinned in that flake:

```bash
nix profile install github:obscuritylabs/Colossus
```

Nix remains the installation owner. Upgrade through the profile or your pinned flake
input, for example `nix profile upgrade colossus`; `colossus update` reports the Nix
ownership marker and refuses to mutate the Nix store.

### Homebrew packaging

The reviewed prebuilt formula source is maintained under
`packaging/homebrew/Formula/colossus.rb` and is ready to be mirrored to the planned
`obscuritylabs/homebrew-tap` repository. Until that tap is created and its first formula
is published, use the direct installer above. A tap installation wraps the upstream
binary with a Homebrew ownership marker, so update discovery can report
`brew upgrade obscuritylabs/tap/colossus` while self-replacement remains disabled.

### Manual archive installation

Every release retains the offline archive flow. Download the exact archive and its
adjacent `.sha256` file from the release page, then verify before extraction:

=== "macOS"

    ```bash
    shasum -a 256 -c colossus-VERSION-TARGET.tar.gz.sha256
    tar -xzf colossus-VERSION-TARGET.tar.gz
    ./colossus-VERSION-TARGET/install.sh
    ```

=== "Linux"

    ```bash
    sha256sum --check colossus-VERSION-TARGET.tar.gz.sha256
    tar -xzf colossus-VERSION-TARGET.tar.gz
    ./colossus-VERSION-TARGET/install.sh
    ```

=== "Windows PowerShell"

    ```powershell
    $archive = "colossus-VERSION-TARGET.zip"
    $expected = (Get-Content "$archive.sha256").Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Colossus checksum mismatch" }
    Expand-Archive $archive
    .\colossus-VERSION-TARGET\install.ps1
    ```

For `sandbox.profile: workspace-development` on Ubuntu 24.04 or later, install the
Linux binary at a root-owned, non-replaceable path and load the archive's narrowly
attached AppArmor profile:

```bash
sudo ./install.sh --prefix /usr/local
sudo ./install-apparmor.sh /usr/local/bin/colossus
```

This is not required where `sandbox doctor` already reports protected-path exclusions
as supported. Do not disable Ubuntu's host-wide unprivileged-user-namespace
restriction; use the exact-path profile or the OCI backend.

## Expected result

The selected release is installed at the requested prefix, its direct ownership is
recorded in the platform data directory, and any required `PATH` change is printed
without modifying a profile.

## Verification

Open a new terminal after applying the printed `PATH` guidance and run:

```bash
colossus --version
colossus update check
```

The first command prints the exact Colossus release identifier. The second validates
the public stable channel and reports `up_to_date` for a freshly installed latest
release.

## Uninstall a direct installation

Inspect the receipt before removing anything. Confirm that `installerKind` is `direct`
and that `binaryPath` is the executable you intend to remove. Delete that one binary
and the receipt; remove the parent directories only when they are empty.

=== "macOS and Linux"

    ```bash
    receipt="${XDG_DATA_HOME:-$HOME/.local/share}/colossus/install.json"
    less "$receipt"
    # After checking binaryPath, remove that exact file and then:
    rm -- "$receipt"
    ```

=== "Windows PowerShell"

    ```powershell
    $receipt = Join-Path $env:LOCALAPPDATA "Colossus\install.json"
    Get-Content $receipt | ConvertFrom-Json | Format-List
    # After checking binaryPath, remove that exact file and then:
    Remove-Item -LiteralPath $receipt
    ```

Homebrew, Nix, source builds, Desktop, and unknown installations are not direct
installer ownership and must be removed through their owning installation method.

## Failure path

- **Offline or rate limited:** retry later or use a previously downloaded archive and
  checksum.
- **Checksum or archive rejection:** do not bypass verification. Download the release
  again and report a reproducible mismatch.
- **Unsafe prefix:** choose an absolute, current-user-owned prefix without linked,
  group-writable, or world-writable installation directories.
- **Command not found:** apply the printed `PATH` command, then open a new terminal.
- **Platform blocked execution:** confirm that the detected target matches the host and
  follow your organization's software-verification process.

## Next step

Run the credential-free [five-minute quickstart](quickstart.md).
