---
title: Windows Desktop
description: Install, verify, operate, diagnose, and remove signed Windows x64 Desktop releases.
audience: user
type: how-to
---

# Windows Desktop

## Goal

Install a Windows x64 stable release or Developer Preview, verify its release checksum
and Obscurity Labs publisher signature, and start the handle-bound Managed Local runtime.

## Prerequisites

Colossus Desktop supports Windows 10 22H2 and Windows 11 on x86-64. Stable and
Developer Preview releases use an Authenticode-signed installer, app, bundled CLI,
and sidecar. Windows on ARM Desktop is not supported yet.

The installer, bundled CLI, and sidecar are sealed by the Colossus bundle manifest
and release checksum. Their Authenticode signatures identify **Obscurity Labs LLC**.
SmartScreen may still warn until the signed application establishes reputation.
Obtain the installer and checksum from the same Colossus GitHub release and verify both.

- A directory owned by the signed-in Windows user to use as the workspace.
- A provider credential for a real model run; the offline self-test needs no credential.
- Permission under your organization's policy to install Colossus Desktop.

The **ChatGPT subscription (Codex)** provider runs the official Codex CLI for account
operations. When `CODEX_HOME` is not set in the Desktop process environment, the
Desktop uses a private Codex credential directory under application storage.
If you intentionally launch Desktop with `CODEX_HOME` set, it must be an absolute
owner-private directory whose `auth.json` can pass Windows DACL validation.

## Steps

### 1. Install

Download the x64 NSIS installer and adjacent `.sha256` file. In PowerShell:

```powershell
$installer = "Colossus-Desktop-STABLE-vX.Y.Z-x86_64-pc-windows-msvc-setup.exe"
$expected = (Get-Content "$installer.sha256").Split()[0].ToLowerInvariant()
$actual = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Colossus Desktop checksum mismatch" }
$signature = Get-AuthenticodeSignature $installer
if ($signature.Status -ne 'Valid' -or
    $signature.SignerCertificate.Subject -notmatch 'CN=Obscurity Labs LLC(?:,|$)') {
    throw "Colossus Desktop publisher signature is invalid"
}
Start-Process ".\$installer"
```

For a Developer Preview, use the `DEVELOPER-PREVIEW` installer name and matching
`.sha256` file instead.

The per-user installer does not require administrator elevation. It installs the app,
bundled CLI and sidecar, icons, offline WebView2 bootstrap material, and an uninstaller.
The release also includes the sealed bundle manifest and release provenance.

### 2. Start Managed Local and choose a workspace

Choose the workspace with the native directory picker. Desktop and its sidecar
independently bind that directory by a retained handle, volume serial number, and
128-bit file ID. Reparse points, junction escapes, same-path replacement, and file
replacement during launch are rejected.

Fresh Managed Local settings default to **Allow all** access with the explicitly unsafe
**Full access** execution boundary. Schema-v1–v3 migrations preserve the old effective
isolation: **Minimal** maps to **Offline isolated**, while **Development** and legacy
`allow_all` map to **Workspace isolated**. Setup and Settings let you change either axis
independently. Full access can reach host files, environment, executables, and network
outside the selected workspace. Warning banners at the top of the app are off by
default. Enable
**Show security warnings** in **Settings → Global → Desktop → Appearance** to display
them. This changes only their visibility. Offline isolated hides the generic model-visible
HTTP and fetch tools, but it is not an air gap: the exact configured provider and
authentication or refresh destinations remain available.

Managed Local starts the sealed sidecar suspended, verifies its retained image identity,
assigns it to a kill-on-close Job Object, establishes an authenticated local named pipe,
and only then resumes it. Closing Desktop closes the Job Object and cleans up the
sidecar process tree.

Desktop settings, generated configuration, runtime state, and imported connection files
live in the workspace's private Desktop partition under the Colossus home and are checked
with Windows owner and DACL rules. With no explicit `COLOSSUS_HOME`, Desktop creates and
uses `%LOCALAPPDATA%\ColossusDesktopHome`; an explicit override must still be absolute and
owner-private. The former application-support location is ignored and left untouched.
Provider credentials are collected by Windows Credential UI with UI persistence disabled,
then stored in Windows Credential Manager. Intermediate credential buffers are zeroized.
Credentials, prompts, model output, and private paths are not included in diagnostics.

Settings opens in a dedicated view with its own sidebar. Choose **Global** or
**Workspace**, select a workspace when needed, and navigate the categories below.
Use **Back to work** to return to your conversation. This view uses the same Windows
storage and credential boundary.
Global provider, model, MCP, search, telemetry, and credential definitions are
revisioned; each Workspace pins the revisions it has accepted. Repository configuration is
inspected by the sealed sidecar parser, and imported `env:` credential references must
be mapped to Windows-backed opaque records. Provider/model/search/MCP diagnostics run
through the selected Workspace's authenticated worker instead of from the WebView.

For a credential-free MCP connection check, add Cloudflare's documentation server to
the global MCP catalog with transport **Streamable HTTP**, URL
`https://docs.mcp.cloudflare.com/mcp`, allowed tool
`search_cloudflare_documentation`, and **Allow stateless HTTP** enabled. Accept and
enable that server revision in the Workspace, apply the configuration, and run its
MCP health test. Cloudflare omits the session ID, so the stateless option is required;
use the `/mcp` endpoint, not a legacy SSE transport. No API key or OAuth login is needed.
See [Cloudflare's server catalog](https://developers.cloudflare.com/agents/model-context-protocol/cloudflare/servers-for-cloudflare/)
and [MCP configuration](../reference/configuration/mcp.md).

Manual API keys and MCP tokens use a native masked-entry window with a byte count,
Save, and Cancel. It accepts up to 65,536 bytes of visible ASCII and rejects overflow
without truncation. The complete token is encrypted in Desktop's private
`credentials-v1.redb`; Windows Credential Manager stores only a small dedicated
encryption key. The vault and its companion lock file are created on first save,
and all manually entered credentials share them.

When upgrading from direct OS token storage, open **Settings → Global → Credentials**
and choose **Re-enter token**. This restores the existing credential ID and its
references. MCP OAuth connections require sign-in again. Settings and old OS entries
are preserved; old tokens are not imported or deleted. A locked or unavailable
credential store is reported separately from a missing token.

### 3. Import a private CA

Open **Settings → Additional CA certificates → Import PEM bundle**. Desktop accepts a
bounded PEM bundle, validates every certificate, and copies it into private application
storage. The original path is not retained or returned to the WebView.

Settings shows only whether a bundle is configured, the certificate count, and SHA-256
certificate fingerprints. Managed Local restarts transactionally and supplies the
private copy to providers, external gRPC clients, webhooks, search/vector services, plugin
registry transfers, policy clients, and the other Colossus-owned network adapters. Removing the
bundle also restarts Managed Local; public system roots remain available.

### 4. Upgrade manually

Windows Desktop releases do not yet advertise an automatic update channel.
**Settings → Desktop updates → Check for updates** therefore reports that updates are
not configured. Download each later release and its `.sha256` sidecar from GitHub
Releases, verify the checksum, close Colossus Desktop, and run the newer installer.
SmartScreen can still warn while the signed app establishes reputation.

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

Normal uninstall and upgrades preserve your data. **Delete application data** is
unchecked by default. Selecting it asks you to confirm permanent deletion of local
conversations, provider/model configuration, saved Desktop credentials, plugins, and
Desktop settings before cleanup starts.

Confirmed cleanup removes `%LOCALAPPDATA%\ColossusDesktopHome`, its identifiable
Windows Credential Manager keys, and Tauri's application/cache data. Project folders,
custom `COLOSSUS_HOME` directories, shared CLI data, and credentials for external
daemons remain outside this cleanup. If the default home was deliberately shared with
the CLI, or contains unrecognized folders, cleanup stops rather than deleting those
items. Supported older settings are inspected without requiring a first launch or
discarding their workspace references. Entries whose identifying
metadata was previously deleted cannot be safely attributed and are not swept by name.

If cleanup fails, uninstall stops and offers Retry; close running Colossus tasks first.
Some items may already have been deleted. Do not manually remove individual protected
journal files or keys to repair setup. Version 0.11.1 migrates the disposable setup
diagnostics automatically while leaving workspace histories and their security checks
intact.

## Expected result

The per-user app starts without administrator elevation, Managed Local reaches
**Ready**, and a selected workspace remains bound to the same Windows file identity.
The app identifies its release channel and verified signing state, and importing a valid CA
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
- A SmartScreen warning is possible for a signed release. Do not disable
  SmartScreen globally; stop if organizational policy does not permit explicit use.

## Current Windows limitations

- The dedicated TUI uses ConPTY only for the sealed bundled CLI. The process starts
  suspended, is checked against the bundle identity, enters a kill-on-close Job Object,
  and completes the private worker-key exchange before the terminal is released to the
  renderer. Colossus never substitutes an arbitrary shell PTY.
- Fleet, delegation, agent workflows, plugin skills, and attachments remain hidden unless an
  authenticated runtime advertises them.
- Stable and preview upgrades are manual until a separate Tauri updater signing key
  and HTTPS update feed are configured.

## Next step

Configure the fixed provider preset in Desktop and run one Plan-mode request before
enabling Execute mode. Install later releases manually after verifying the Authenticode
publisher identity and the release notes.
