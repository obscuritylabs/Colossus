---
title: Colossus Desktop
description: Start with the managed local runtime, connect a provider, and understand external targets and local terminals.
audience: user
type: how-to
---

# Colossus Desktop

## Goal

Open one repository in Colossus Desktop without installing or starting a daemon, then
confirm that work uses the app-managed runtime and its bounded access profile.

## Prerequisites

- macOS 13 or later for the first desktop release.
- Apple silicon for the first direct-download build.
- A folder you own and intend Colossus to use as its workspace.
- An OpenAI Responses or OpenAI-compatible provider key for real model runs.

The offline self-test does not require a provider key or network connection.

## Steps

### 1. Download and verify Desktop

From the official release, download
`Colossus-Desktop-vX.Y.Z-aarch64-apple-darwin.zip` and its adjacent `.sha256` file.
Keep both files together and verify the archive before opening it:

```bash
shasum -a 256 -c Colossus-Desktop-vX.Y.Z-aarch64-apple-darwin.zip.sha256
```

The check must report success. Expand the zip and move **Colossus Desktop** to
Applications. The release workflow signs the app and both bundled executables with
Developer ID, notarizes the archive, staples the ticket, and verifies Gatekeeper before
publishing the draft asset.

### 2. Open a workspace

Launch Colossus Desktop and choose a folder through the native picker. The app records
an opaque workspace binding in application support storage. It does not write Colossus
configuration, state, or credentials into the selected repository.

An older preview that recorded only a path or inode cannot safely prove that the
current folder is the one you chose after the app has exited. After upgrading such a
preview, Desktop intentionally asks you to choose the folder again and starts a fresh
managed state partition; it never attaches the replacement folder to the old state.

New installations select **Managed Local**. The signed app supervises its bundled
`colossus-sidecar`, and the native desktop backend connects to that process over the
same authenticated, pinned loopback gRPC contract used by application SDKs. A WebView
reload does not stop the runtime.

### 3. Configure a model

Select the fixed OpenAI Responses or OpenRouter (OpenAI-compatible) preset and enter
the model. Continuing opens an operating-system secure-input dialog for that preset's
fixed origin; the WebView cannot submit a key or endpoint. The native layer stores the
key directly in the platform keychain.
The managed runtime receives it through a one-use inherited bootstrap channel and
resolves only its opaque `host:` reference after policy permits the provider action.

Later model or access-profile edits reuse the existing keychain entry when the
provider preset is unchanged. Select **Replace the stored API key** to rotate it;
first setup and every provider-preset change always force the native key prompt.

The key is not written to YAML, argv, environment variables, renderer state, logs, or
terminal sessions. Real model runs remain unavailable until this setup succeeds. Use
the explicit offline self-test when you only need to validate local startup.

### 4. Start work

Create new work, choose Plan or Execute, and submit a prompt. Every request names the
selected runtime target. **Minimal** grants no workspace tools; **Development** grants
the documented exact tool set with policy and approval still enforced. Managed Local's
native layer requires an additional operating-system confirmation before first
enabling Development or elevating from Minimal. Its
primary credential has exactly the run and prompt scopes plus the tools derived from
that selection. Approval responses use a separate native-only, tool-less credential after the
operating-system confirmation. Neither credential grants administrative,
delegated-agent, or unrestricted skill authority.

Settings shows the runtime as `Starting`, `Ready`, `Restarting`, `Stopping`, or
`Failed`. Workspace and provider changes drain and restart the sidecar. Unexpected
exits receive at most three bounded restart attempts; in-flight mutations are not
automatically replayed.

### 5. Add an External target when needed

Use **External** for an installed daemon or fleet node that was enrolled for this
application. External targets preserve their independent endpoint, certificate pin,
instance identity, application credential, and lifecycle. Fleet can show multiple
targets, while Work sends operations only to the selected target.

In **Settings → External targets**, choose **Add daemon** and select the non-secret
connection JSON created from the worker application-enrollment output. The strict file
contains `instanceId`, `certificateSha256`, `publicApiDir`, `credentialService`, and
`credentialAccount`, plus an optional human-readable `label`. Start from
`apps/desktop/src-tauri/connection.json` when creating it; map the CLI's printed
`instance_id`, `certificate_sha256`, and credential destination names to the camel-case
fields. Never add the bearer credential or provider key to this file.

Enroll an External target into Desktop's identity-bound keyring namespace with
`credential-keyring-account auto`. The CLI expands `auto` to an account bound to the
daemon's full instance ID and TLS certificate fingerprint and prints the exact result
for the JSON:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:colossus-desktop \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond --role primary \
  --credential-keyring-service com.obscuritylabs.colossus.desktop.external \
  --credential-keyring-account auto
```

Add exact `--tool TOOL_NAME` ceilings when that External target should execute tools.
The explicit `approvals:respond` scope lets Desktop answer a policy approval for those
effects; omit that scope when the target has no approval-gated tools or approvals are
handled by a different application credential. It never expands the tool ceiling.
The connection file cannot choose an arbitrary keychain entry: Desktop accepts only
the fixed service and the account derived from the file's instance and certificate
anchors. A target saved by an older Desktop build remains listed but reports that
re-enrollment is required. Migrate its legacy credential explicitly while the worker is
stopped:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:colossus-desktop \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond --role primary \
  --credential-keyring-service com.obscuritylabs.colossus.desktop.external \
  --credential-keyring-account auto \
  --retire-credential-keyring-service com.obscuritylabs.colossus.desktop \
  --retire-credential-keyring-account colossus-public-api
```

Add the same exact `--tool` ceilings the application needs. Omit
`--scope approvals:respond` during migration only when Desktop must not approve
effects for that target. The CLI reads the legacy
bearer only from the operating-system keyring, proves that it is an active credential
for `app:colossus-desktop` under this daemon's API authentication root, delivers and
activates the new identity-bound credential, durably revokes the legacy credential,
and only then deletes the legacy keyring entry. The retirement flags must appear
together and conflict with `--replace-credential`.

Import the updated JSON to upgrade that same target in place. Desktop never copies a
bearer out of a legacy selector. If the CLI reports an unconfirmed revocation or
keyring cleanup, keep the printed non-secret credential IDs for reconciliation; the
new credential remains active and neither bearer is printed. Do not delete the source
selector's current value unless it is confirmed to be the printed prior credential;
another process may have replaced that keyring entry.

Desktop accepts only a bounded, regular, non-symlink connection file owned by the
current user and not writable by group or other users. Before import, selection,
reconnection, or removal, a native dialog identifies the daemon by label, instance ID,
and full certificate SHA-256 pin. It copies the validated trust
anchors into owner-private native settings. The renderer receives only a newly
generated opaque target ID and the display label; discovery paths, certificate pins,
and keyring lookup labels are never returned to it. Removing a target deletes this
saved native connection record but does not revoke its worker credential or stop the
daemon; use worker administration when revocation is required.

A workspace already owned by another worker is never stopped or taken over. Connect
that worker as External instead. Closing Desktop stops Managed Local after graceful
drain and checkpoint; it does not stop an installed External daemon.

### 6. Opt into the local TUI

The macOS MVP deliberately does not expose a general Shell PTY. macOS provides no
supported race-free job primitive to an ordinary desktop app that can guarantee cleanup
after an arbitrary child creates a new session, double-forks, and is reparented. The
native DTO rejects `shell` instead of presenting a cleanup guarantee the platform cannot
enforce.

The dedicated terminal WebView can open only the bundled Colossus TUI for the active
managed workspace. It cannot supply an executable, environment, absolute working
directory, or arbitrary arguments. Clipboard escape writes, automatic URL opening,
remote navigation, and renderer-initiated process spawning are disabled; manual copy
and paste remain user actions.

**Open Colossus TUI** starts the verified bundled CLI suspended with fixed arguments,
binds its live code identity to the signed bundle manifest before resuming it, and then
requires the CLI to open and attest the exact selected workspace before delivering
worker authentication through bounded one-use inherited anonymous pipes that never
traverse the PTY. It requires the existing managed worker and fails instead of opening
a second writer. TUI actions remain inside normal Colossus policy and audit. External
targets never offer a TUI action.

## Expected result

The selected folder appears as a Managed Local workspace, runtime health reaches
`Ready`, and a provider-backed or offline test run produces ordered durable updates.
No daemon enrollment or terminal command is required for the default path.

## Verification

Open Settings and confirm the selected target, workspace display name, provider/model,
access profile, and runtime health. Restart Managed Local once and confirm that Work
refetches durable runs without resubmitting a create or effect request.

If the local TUI is enabled, open it and confirm that it attaches to the existing worker;
it must fail safely if Managed Local is not ready.

## Failure path

- **Needs workspace:** choose a folder through the native picker; renderer-supplied
  paths are intentionally unsupported. Upgrades from a preview-era path-only or
  inode-only binding also require this explicit reselection.
- **Needs provider:** choose a supported provider preset, enter its model, and save a
  valid key through the native secure prompt, or use the offline self-test.
- **Runtime integrity failure:** do not replace bundled files. Reinstall a signed
  desktop build.
- **Workspace already owned:** leave the owner running and add its authenticated daemon
  as an External target.
- **External re-enrollment required:** provision the credential into the Desktop-bound
  service with account `auto`, update the connection JSON from the command output, and
  import it again.
- **Provider failure:** confirm the fixed provider preset, model, keychain access, and
  key format. Errors shown to the renderer are sanitized.
- **TUI unavailable:** wait for Managed Local to become Ready; the launcher never falls
  back to a second local runtime.

## Next step

Read [Core concepts](core-concepts.md) before broadening access, or learn the full
[Terminal UI](../use/terminal-ui.md) interaction model.
