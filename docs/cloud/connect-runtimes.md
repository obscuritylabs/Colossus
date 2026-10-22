---
title: Connect runtimes
description: Enroll Desktop or CLI agents in Cloud and choose whether to share local conversations.
audience: operator
type: how-to
icon: lucide/plug
---

# Connect runtimes

Connect an existing Desktop workspace or CLI daemon to a Cloud project. The
runtime opens an outbound connection and keeps its own tools, credentials, policy,
and sandbox. Its machine needs no inbound public port.

In Cloud, select the project, open **Fleet**, and create an invitation with the
reviewed role ceiling. It expires after ten minutes and can be used once.

## Enroll Desktop

1. Select a Managed Local workspace and open **Control Plane** settings.
2. Enter the enrollment URL and paste the invitation.
3. Review the native confirmation of the cloud origin and execution/approval access.
4. Check that the workspace appears online in Fleet.

Desktop keeps credentials in native custody. External daemon targets use their
separately enrolled CLI connector. **Revoke enrollment** in Desktop or **Revoke
agent** in Fleet removes cloud authority; forgetting local data alone does not.

## Enroll a CLI daemon

Use an existing runtime configuration and an owner-private public API directory.
Stop the worker for offline application enrollment, then restart with the same
paths. Choose tools and roles within its existing ceilings:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api \
  --enroll-application app:colossus-cloud-cli \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond \
  --role primary --tool user.ask --tool filesystem.read \
  --credential-keyring-service dev.colossus.cloud \
  --credential-keyring-account engineering
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api
```

Create an owner-private `/secure/connector.json` using the exact public identities
returned by enrollment. The bearer stays in the native credential store:

```json
{
  "descriptor": "/secure/runtime-api/endpoint.json",
  "certificate": "/secure/runtime-api/certificate.pem",
  "instance_id": "replace-with-enrolled-local-instance",
  "certificate_sha256": "replace-with-exact-64-character-local-pin",
  "keyring_service": "dev.colossus.cloud",
  "keyring_account": "engineering"
}
```

Enroll and run the connector, pasting the Cloud invitation at its stdin prompt:

```sh
colossus control-plane enroll --local-config /secure/connector.json \
  --enrollment-url https://colossus.example.com/api/enroll --name engineering
colossus control-plane run --local-config /secure/connector.json --name engineering
```

In another terminal, use `colossus control-plane status --name engineering`,
`disconnect`, or `revoke`. Disconnect leaves the daemon and accepted work alive.
Revoke before `forget` or uninstalling; retry a failed revocation with its retained
enrollment. Certificates renew automatically, or use `control-plane renew`.
The standalone `colossus-connector` supports the same commands without the prefix.

## Run unattended

Supervise worker and connector separately as the same selected OS user: systemd
on Linux, launchd on macOS, or your Windows service supervisor/Task Scheduler.
Preserve their private state directories and exact enrollment selector.
Give concurrent CLI daemon/connector pairs separate `COLOSSUS_HOME` directories;
one native vault has an exclusive process lease. Desktop manages its shared vault.

Without an unlocked native credential store, inject a random 32-byte hex wrapping
key through a protected secret environment. Set the worker's
`--public-api-vault-key-variable COLOSSUS_HEADLESS_KEY` during enrollment and hosting,
then add this selector to the earlier connector configuration:

```json
{
  "headless_authority": {
    "directory": "/secure/runtime-api/headless-credentials",
    "key_variable": "COLOSSUS_HEADLESS_KEY"
  }
}
```

Pass `--vault-key-variable COLOSSUS_HEADLESS_KEY` on connector enrollment and
lifecycle commands. Missing or changed key authority fails closed.
Persistent execution needs a supervised CLI daemon; Desktop's embedded connection
and Managed Local runtime stop when the app exits.

## Share local conversations

Local history stays private until its source application grants sharing.
In Desktop's **Control Plane** settings, choose **Share Desktop history for
viewing** or **Share history and allow continuation**, then save and confirm.
This includes existing and future sessions for that source application/workspace.

For CLI applications, use the source application's protected configuration:

```sh
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli --allow-continuation
```

Omit `--allow-continuation` for viewing only; use `--disable` to stop future sharing.
The source needs Read and Control, plus Execute for continuation. Imported source
runs stay read-only: sharing does not transfer approval or cancellation authority.
Disabling sharing does not erase history already synchronized to Cloud.
