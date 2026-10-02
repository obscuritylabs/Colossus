---
title: External runtime targets
description: Connect Desktop to an enrolled worker while keeping its credentials and lifecycle separate.
audience: operator
type: how-to
icon: lucide/network
---

# External runtime targets

**Managed Local** is the normal Desktop path: choose a folder and let the app start its bundled runtime. Use an **External** target when a worker is already installed or administered elsewhere and has been enrolled for this Desktop application. Desktop routes Work actions only to the selected target. It never takes over a workspace already owned by another worker.

## Enroll the worker

On the worker host, use its current CLI configuration to enroll `app:colossus-desktop`. The following example gives Desktop run, prompt, and approval scopes. Add only the exact `--tool TOOL_NAME` ceilings it needs:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:colossus-desktop \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond --role primary \
  --credential-keyring-service com.obscuritylabs.colossus.desktop.external \
  --credential-keyring-account auto
```

Omit `approvals:respond` if approvals are handled by another application credential or the target has no approval-gated tools. The CLI's `auto` account is bound to the daemon's instance identity and TLS certificate fingerprint; use the *exact* account it prints.

Create a non-secret connection JSON using the [Desktop connection example](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src-tauri/connection.json). Fill in `instanceId`, `certificateSha256`, `publicApiDir`, `credentialService`, and `credentialAccount`, and optionally a display `label`. Map the enrollment output's `instance_id` and `certificate_sha256` to the camel-case JSON fields. **Never put a bearer credential or provider key in this file.** Desktop resolves the enrolled credential from its identity-bound keyring entry.

## Add it in Desktop

Open **Connections → Add daemon**, select the connection JSON, and review the native confirmation showing the label, full instance ID, and full certificate pin. Desktop accepts only a bounded, regular, owner-controlled file; symlinks and files writable by other users are rejected. After import, choose **Use** on the External target. **Capabilities** shows the target's advertised features, and Work sends subsequent actions to the selected target.

An External daemon keeps its own configuration, workspace state, credential, and lifecycle. Closing Desktop does not stop it. Removing the target from Desktop removes the saved connection record but does not revoke the worker credential or stop the daemon; perform revocation through worker administration if it is required.

The local Files, Git, and bundled TUI paths are for Managed Local workspaces and may be unavailable on an External target. Plugin management may be read-only even when the target advertises plugin discovery.

## Upgrade an older connection

An older Desktop connection can remain listed but require re-enrollment because its credential was saved under a legacy selector. While the worker is stopped, run enrollment with the legacy retirement flags. Use the same scopes and exact `--tool` ceilings that this Desktop connection needs:

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

The CLI validates and retires the old worker credential before removing its keyring entry. Re-import the updated non-secret JSON to upgrade the saved target in place. If retirement or keyring cleanup is unconfirmed, keep the CLI's printed non-secret credential IDs for reconciliation; do not delete an entry that another process may have replaced.

## If connection fails

- **Re-enrollment required:** enroll the daemon using the identity-bound service and `auto` account above, update the JSON from the new output, and import it again. Older Desktop keyring selectors are not used automatically.
- **Certificate or instance mismatch:** confirm that the JSON came from this worker's current enrollment and certificate. Desktop will not replace a pin silently.
- **Permission or tool unavailable:** review the enrolled scopes and exact tool ceilings on the worker, then reconnect. Desktop cannot widen them from the connection file.
- **Workspace already owned:** keep its existing worker running and connect that worker as External instead of starting a second Managed Local owner.

For the public API and enrollment boundary, see [Application SDK](../develop/application-sdk.md). For worker operations, see [Storage and worker](../admin/storage-worker.md).
