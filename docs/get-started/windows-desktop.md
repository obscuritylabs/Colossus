---
title: Windows Desktop Developer Preview
description: Install, verify, operate, diagnose, and remove the unsigned Windows x64 Desktop preview.
audience: user
type: how-to
---

# Windows Desktop Developer Preview

## Goal

Install the Windows x64 Developer Preview, verify its release checksum, start the
handle-bound Managed Local runtime, and know which security and release limitations
still distinguish it from a stable signed build.

## Prerequisites

Colossus Desktop supports Windows 10 22H2 and Windows 11 on x86-64 as an
**unsigned Developer Preview**. It is not a stable Windows release. Windows on ARM and
Authenticode-signed installers are not part of this milestone.

The preview installer, bundled CLI, and bundled sidecar are sealed by the Colossus
bundle manifest and release checksum, but they do not yet establish a Windows publisher
identity. Windows SmartScreen can therefore show an **Unknown publisher** warning.
Obtain the installer and checksum from the same Colossus GitHub release, verify the
checksum, and follow your organization's policy before choosing **Run anyway**. Do not
disable SmartScreen globally.

- A directory owned by the signed-in Windows user to use as the workspace.
- A provider credential for a real model run; the offline self-test needs no credential.
- Permission under your organization's policy to run a checksum-verified but unsigned
  preview installer.

## Steps

### 1. Install

Download the x64 NSIS installer and adjacent `.sha256` file. In PowerShell:

```powershell
$installer = "Colossus-Desktop-DEVELOPER-PREVIEW-vX.Y.Z-x86_64-pc-windows-msvc-setup.exe"
$expected = (Get-Content "$installer.sha256").Split()[0].ToLowerInvariant()
$actual = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Colossus Desktop checksum mismatch" }
Start-Process ".\$installer"
```

The per-user installer does not require administrator elevation. It installs the app,
bundled CLI and sidecar, icons, offline WebView2 bootstrap material, and an uninstaller.
The preview release also includes the sealed bundle manifest and release provenance.

### 2. Start Managed Local and choose a workspace

Choose the workspace with the native directory picker. Desktop and its sidecar
independently bind that directory by a retained handle, volume serial number, and
128-bit file ID. Reparse points, junction escapes, same-path replacement, and file
replacement during preview are rejected.

Managed Local starts the sealed sidecar suspended, verifies its retained image identity,
assigns it to a kill-on-close Job Object, establishes an authenticated local named pipe,
and only then resumes it. Closing Desktop closes the Job Object and cleans up the
sidecar process tree.

Desktop application storage and imported connection files are checked with Windows
owner and DACL rules. Provider credentials are collected by Windows Credential UI with
UI persistence disabled, then stored in Windows Credential Manager. Intermediate
credential buffers are zeroized. Credentials, prompts, model output, and private paths
are not included in diagnostics.

### 3. Import a private CA

Open **Settings → Additional CA certificates → Import PEM bundle**. Desktop accepts a
bounded PEM bundle, validates every certificate, and copies it into private application
storage. The original path is not retained or returned to the WebView.

Settings shows only whether a bundle is configured, the certificate count, and SHA-256
certificate fingerprints. Managed Local restarts transactionally and supplies the
private copy to providers, external gRPC clients, webhooks, search/vector services, pack
downloads, policy clients, and the other Colossus-owned network adapters. Removing the
bundle also restarts Managed Local; public system roots remain available.

### 4. Upgrade previews manually

Unsigned Windows Developer Previews do not advertise an automatic update channel.
**Settings → Desktop updates → Check for updates** therefore reports that updates are
not configured. Download each later preview and its `.sha256` sidecar from GitHub
Releases, verify the checksum, close Colossus Desktop, and run the newer installer.
SmartScreen can still warn because the preview has no Authenticode publisher identity.

### 5. Export diagnostics

Open **Settings → Diagnostics → Export diagnostics**. The local JSON export contains the
application version, platform, architecture, release channel, bundle-integrity state,
actual code-signing state, selected runtime kind, runtime health, and bounded sanitized
error codes. It excludes prompts, credentials, headers, model output, certificate paths,
and filesystem paths.

### 6. Remove cleanly

Close Desktop first, then use **Settings → Apps → Installed apps → Colossus Desktop →
Uninstall**. The uninstaller removes the installed application and shortcuts. Confirm
that no `colossus-sidecar.exe` or app-owned `colossus.exe` process remains:

```powershell
Get-Process "colossus-sidecar", "colossus" -ErrorAction SilentlyContinue
```

Application settings and Windows Credential Manager entries are intentionally treated
as user data. Remove them only under your organization's retention policy.

## Expected result

The per-user app starts without administrator elevation, Managed Local reaches
**Ready**, and a selected workspace remains bound to the same Windows file identity.
The app identifies itself as an unsigned Developer Preview, and importing a valid CA
bundle reports only certificate count and fingerprints.

## Verification

- Compare the installed release version and channel in the exported diagnostics with
  the release you downloaded.
- Run the offline Managed Local self-test and confirm it completes without a provider
  credential.
- Open a text file from the Files drawer and confirm it is read-only and syntax
  highlighted.
- After uninstalling, confirm the process query above returns no Colossus process.

## Failure path

- A checksum mismatch means the installer must not be opened; download both release
  files again from the same release.
- A workspace reparse-point, replacement, junction, or unsafe DACL error requires
  selecting an owner-controlled ordinary directory.
- A malformed or untrusted CA bundle is rejected without changing the active runtime.
  Export diagnostics if the sanitized runtime code is needed for support.
- A SmartScreen warning is expected for this unsigned preview. Do not disable
  SmartScreen globally; stop if organizational policy does not permit explicit use.

## Current preview limitations

- The dedicated TUI uses ConPTY only for the sealed bundled CLI. The process starts
  suspended, is checked against the bundle identity, enters a kill-on-close Job Object,
  and completes the private worker-key exchange before the terminal is released to the
  renderer. Colossus never substitutes an arbitrary shell PTY.
- Fleet, delegation, agent workflows, skills, and attachments remain hidden unless an
  authenticated runtime advertises them.
- Preview upgrades are manual. A stable Windows channel remains disabled until the
  installer, app, CLI, and sidecar can all be Authenticode signed.

## Next step

Configure the fixed provider preset in Desktop and run one Plan-mode request before
enabling Execute mode. Install later previews manually; move to a future stable Windows
release only after its Authenticode publisher identity and stable-channel release notes
have been verified.
