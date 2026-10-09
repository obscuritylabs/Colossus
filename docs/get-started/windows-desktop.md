---
title: Start with Desktop on Windows
description: Install the signed Windows app, start a workspace, and keep it updated or remove it.
audience: user
type: tutorial
icon: lucide/monitor-down
---

# Start with Desktop on Windows

Colossus Desktop supports Windows 10 22H2 and Windows 11 on x86-64. It installs per user and includes the app, its local runtime, bundled CLI, and WebView2 bootstrap material. Windows on ARM Desktop is not supported yet. For the feature tour after installation, see [Desktop overview](../desktop/index.md).

## 1. Install

Download the x64 Desktop installer and its adjacent `.sha256` file from the same [Colossus release](https://github.com/obscuritylabs/Colossus/releases). Use the exact downloaded filename in PowerShell:

```powershell
$installer = ".\Colossus-Desktop-STABLE-vX.Y.Z-x86_64-pc-windows-msvc-setup.exe"
$expected = (Get-Content "$installer.sha256").Split()[0].ToLowerInvariant()
$actual = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "Colossus Desktop checksum mismatch" }
$signature = Get-AuthenticodeSignature $installer
if ($signature.Status -ne 'Valid' -or
    $signature.SignerCertificate.Subject -notmatch 'CN=Obscurity Labs LLC(?:,|$)') {
    throw "Colossus Desktop publisher signature is invalid"
}
Start-Process $installer
```

For a Developer Preview, use its `DEVELOPER-PREVIEW` installer name and matching checksum file. A checksum detects changes relative to the release checksum; the Authenticode signature identifies the publisher. Windows SmartScreen may still warn while a signed app establishes reputation. Follow your organization's installation policy.

## 2. Start a workspace

Launch Desktop, choose a folder you own with the native picker, and follow **Desktop → Workspace → Provider → Model → Start**. Desktop manages its bundled local runtime and shows **Ready** when the workspace can connect. You can run the offline self-test without a model credential, then select a provider and model for real work. The ChatGPT subscription option requires the official Codex CLI for sign-in. [First-run walkthrough](desktop.md#choose-a-workspace)

The selected folder is the working context. It is not automatically a containment boundary: fresh Managed Local settings can use broad **Allow all** tool access and **Full access** execution. Review **Workspace → Runtime** and choose **Workspace isolated** or **Offline isolated** if you need stronger containment. [Settings and access](../desktop/settings.md#access-and-execution-boundaries)

Close the window to keep Desktop working from the Windows notification area. Left-click its icon to restore the window or right-click for actions such as **New Work**. Choose **Shut Down Colossus** to stop the app and its managed runtime. Windows notifications for a completed run or one needing input can include the thread title and a short response or error preview; control their visibility in Windows settings.

Desktop keeps its settings and state in a private Colossus home. Without an explicit `COLOSSUS_HOME`, Windows uses `%LOCALAPPDATA%\ColossusDesktopHome`. Provider keys are entered in a native credential prompt and kept out of the conversation view. A locked or unavailable credential store is reported separately from a missing token.

## 3. Import an organizational CA

If a private provider or TLS gateway needs an additional root, open **Settings → Global → Desktop → Additional CA certificates** and import a PEM bundle. Desktop validates and privately copies it, then shows only certificate count and fingerprints. Import or removal restarts Managed Local. This adds trust for Colossus-owned network connections; it does not alter the Windows trust store or an MCP subprocess's own TLS configuration.

## 4. Upgrade

Windows Desktop currently uses manual upgrades. **Check for updates** can report that no signed update channel is configured. Download the later installer and `.sha256` file from [Colossus releases](https://github.com/obscuritylabs/Colossus/releases), verify checksum and signature again, close Desktop, and run the installer. The normal upgrade preserves local data.

## 5. Export diagnostics

Open **Settings → Global → Desktop → Diagnostics → Export diagnostics** after reproducing a problem. The local report includes version, release channel, signing and bundle status, runtime health, and bounded error codes. It excludes prompts, model output, credentials, and private paths. For an MCP failure, run the workspace's **Test** action first so the current connection report is included.

## 6. Remove cleanly

Close Desktop, then use **Settings → Apps → Installed apps → Colossus Desktop → Uninstall**. Normal removal preserves your local conversations and settings. The optional **Delete application data** choice asks for a separate confirmation before permanently removing Desktop data, credentials, and application storage. It does not remove project folders, a custom Colossus home, CLI data, or credentials held by External daemons.

If cleanup reports files in use, close Colossus tasks and retry. If you want to preserve remaining data, cancel and uninstall again without **Delete application data**. Do not manually remove protected journal files or keys as a setup repair.

## What's next?

- [Start a Desktop task](../desktop/work.md#start-a-task).
- [Use the panes beside your conversation](../desktop/tools.md).
- [Understand settings and approvals](../desktop/settings.md).
