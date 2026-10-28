---
title: Connect runtimes
description: Enroll Desktop or CLI runtimes, enable web shells, and share local conversations with Cloud.
audience: operator
type: how-to
icon: lucide/plug
---

# Connect runtimes

Connect an existing Desktop workspace or CLI daemon to a Cloud project. The runtime
opens an outbound connection, so its machine needs no inbound public port. It keeps
its own tools, credentials, policy, and sandbox.

Select the project in Cloud, open **Fleet**, and create an invitation with the role
ceiling you intend to grant. Invitations expire after ten minutes and can be used once.

## Enroll Desktop

1. Select a Managed Local workspace and open **Control Plane** settings.
2. Enter the enrollment URL and paste the invitation.
3. Review the native confirmation of the Cloud origin and execution/approval access.
4. Check that the workspace appears online in Fleet.

Desktop keeps credentials in native custody. An external daemon target uses its
separately enrolled CLI connector. To remove Cloud authority, choose **Revoke
enrollment** in Desktop or **Revoke agent** in Fleet. Forgetting local data alone
does not revoke the enrollment.

## Enroll a CLI daemon

Use an existing runtime configuration and an owner-private public API directory.
The worker must be stopped while enrolling the local application.

### Grant the connector local access

Choose scopes, tools, and roles within the runtime's existing ceilings. This example
grants one connector application access to the primary role and two tools:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api \
  --enroll-application app:colossus-cloud-cli \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond \
  --role primary --tool user.ask --tool filesystem.read \
  --credential-keyring-service dev.colossus.cloud \
  --credential-keyring-account engineering
```

Restart the worker using the same configuration and public API directory:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api
```

### Save the connector's local identities

Create an owner-private `/secure/connector.json` with the exact public identities
returned by enrollment. The bearer remains in the native credential store:

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

### Enroll in Cloud and start the connector

Run enrollment and paste the Cloud invitation at its stdin prompt. Then start the
connector with the same local configuration and enrollment name:

```sh
colossus control-plane enroll --local-config /secure/connector.json \
  --enrollment-url https://colossus.example.com/api/enroll --name engineering
colossus control-plane run --local-config /secure/connector.json --name engineering
```

Check that the workspace is online in Fleet. In another terminal, inspect the saved
enrollment with `colossus control-plane status --name engineering`.

Use `disconnect` to stop its connection while leaving the daemon and accepted work
alive. Use `revoke` to remove Cloud authority before `forget` or uninstalling. Retain
the enrollment if revocation fails so you can retry it. Certificates renew
automatically; `control-plane renew` also requests renewal. The standalone
`colossus-connector` accepts the same commands without the `control-plane` prefix.

## Enable a web shell

Remote human shells require an explicitly enabled CLI daemon. Desktop's local
Terminal setting does not enable remote access.

Before enabling shells, complete the Control Plane rollout on every replica. Older
replicas cannot parse the Terminal operator role or provide the relay. Linux workers
also need kernel pidfd support and readable `/proc` for ordinary job cleanup.

Enable all three access controls:

1. During offline application enrollment, add `--scope terminals:control` to the
   connector application's local grant.
2. Start the public worker with `--enable-human-terminals`:

   ```sh
   colossus --workspace /work/project --config /secure/runtime.yaml worker \
     --public-api-dir /secure/runtime-api --enable-human-terminals
   ```

3. Give the human account the project's **Terminal operator** role. Operator,
   Approver, Project administrator, and global administrator access do not include
   Terminal permission.

Once all three controls permit access, an online workspace advertises **Open Shell**.
Follow the [web shell workflow](work.md#open-a-shell-on-a-connected-host) to test it.

The shell runs as the worker's OS account in the selected workspace, with that
account's filesystem and network authority. Commands run outside agent approvals
and do not become model context. The browser cannot choose another executable,
starting directory, environment, or application credential.

**Detach** preserves the process for up to two minutes without an active reader;
reopening reattaches to it. **Close Shell** ends the process. Each terminal has an
eight-hour maximum lifetime. A worker permits eight sessions, with at most four per
human/application. Browser, connector, or Control Plane reconnection can retain the
shell while the worker survives. A worker or machine restart ends it; Colossus never
reruns its commands.

Output history is bounded. A fresh pane replays retained output and resize events.
If earlier history was evicted, it reports a gap. Unconfirmed input is never
automatically resent after disconnect.

## Run unattended

Supervise the worker and connector separately under the same selected OS user:
systemd on Linux, launchd on macOS, or your Windows service supervisor/Task Scheduler.
Preserve their private state directories and exact enrollment selector. Give
concurrent CLI daemon/connector pairs separate `COLOSSUS_HOME` directories; one native
vault has an exclusive process lease. Desktop manages its shared vault.

If the native credential store will not be unlocked, inject a random 32-byte hex
wrapping key through a protected secret environment. Set the worker's
`--public-api-vault-key-variable COLOSSUS_HEADLESS_KEY` during enrollment and hosting.
Add this selector to the connector configuration:

```json
{
  "headless_authority": {
    "directory": "/secure/runtime-api/headless-credentials",
    "key_variable": "COLOSSUS_HEADLESS_KEY"
  }
}
```

Pass `--vault-key-variable COLOSSUS_HEADLESS_KEY` on connector enrollment and
lifecycle commands. A missing or changed wrapping key fails closed.

Use a supervised CLI daemon for persistent execution. Desktop's embedded connection
and Managed Local runtime stop when the app exits.

## Share local conversations

Local history stays private until its source application grants sharing. In Desktop's
**Control Plane** settings, choose **Share Desktop history for viewing** or
**Share history and allow continuation**, then save and confirm. The choice includes
existing and future sessions for that source application/workspace.

For a CLI application, use the source application's protected configuration:

```sh
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli --allow-continuation
```

Omit `--allow-continuation` for viewing only; use `--disable` to stop future sharing.
The source needs Read and Control, plus Execute for continuation. Imported source
runs stay read-only: sharing does not transfer approval or cancellation authority.
Disabling sharing does not erase history already synchronized to Cloud.
